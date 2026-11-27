//! UTF-8 validation, load and change-detection hashing.
//!
//! Only UTF-8 (with or without a BOM) is accepted. Invalid UTF-8, UTF-16 and
//! NUL-heavy data are rejected without modifying the file. Newline kinds are
//! recorded in an [`EolMap`] so the original spelling can be restored on save.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use chaus_core::eol::{Eol, EolMap};

use crate::file_meta::{platform_identity, FileMeta};

pub(crate) const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const BINARY_NUL_RATIO: f64 = 0.01;

/// A successfully loaded UTF-8 document.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct LoadedFile {
    /// Canonical text with `\n` hard breaks.
    pub text: String,
    pub eol: EolMap,
    pub preferred_eol: Eol,
    pub had_utf8_bom: bool,
    pub meta: FileMeta,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum LoadError {
    NotFound(PathBuf),
    NotAFile(PathBuf),
    UnsupportedUtf16Bom(PathBuf),
    InvalidUtf8(PathBuf),
    LikelyBinary(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound(path) => write!(formatter, "{}: file not found", path.display()),
            LoadError::NotAFile(path) => {
                write!(formatter, "{}: not a regular file", path.display())
            }
            LoadError::UnsupportedUtf16Bom(path) => write!(
                formatter,
                "{}: UTF-16 is not supported; only UTF-8 can be edited",
                path.display()
            ),
            LoadError::InvalidUtf8(path) => write!(
                formatter,
                "{}: file is not valid UTF-8; it was left untouched",
                path.display()
            ),
            LoadError::LikelyBinary(path) => write!(
                formatter,
                "{}: file looks binary (contains NUL bytes); it was left untouched",
                path.display()
            ),
            LoadError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Load a UTF-8 file. The path is preserved for display; the canonical target
/// resolves symlinks so a save replaces the file the link points at.
pub fn load(path: &Path) -> Result<LoadedFile, LoadError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadError::NotFound(path.to_path_buf()));
        }
        Err(source) => {
            return Err(LoadError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_file() {
        return Err(LoadError::NotAFile(path.to_path_buf()));
    }
    let mut file = File::open(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| LoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(LoadError::UnsupportedUtf16Bom(path.to_path_buf()));
    }
    if looks_binary(&bytes) {
        return Err(LoadError::LikelyBinary(path.to_path_buf()));
    }

    let had_utf8_bom = bytes.starts_with(UTF8_BOM);
    let body = if had_utf8_bom {
        &bytes[UTF8_BOM.len()..]
    } else {
        &bytes[..]
    };
    let source =
        std::str::from_utf8(body).map_err(|_| LoadError::InvalidUtf8(path.to_path_buf()))?;
    let preferred_eol = Eol::detect(source);
    let (text, eol) = EolMap::from_source(source);
    let canonical_target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    Ok(LoadedFile {
        text,
        eol,
        preferred_eol,
        had_utf8_bom,
        meta: FileMeta {
            canonical_target,
            user_requested_path: path.to_path_buf(),
            had_utf8_bom,
            preferred_eol,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            identity: platform_identity(path),
            fingerprint: fingerprint(&bytes),
        },
    })
}

fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.is_empty() {
        return false;
    }
    let nul_count = sample.iter().filter(|byte| **byte == 0).count();
    nul_count as f64 / sample.len() as f64 > BINARY_NUL_RATIO
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Fixed working buffer: hashing a 100 MiB target must not allocate 100 MiB.
const FINGERPRINT_CHUNK: usize = 64 * 1024;

fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
    let mut hash = hash;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// FNV-1a, used only for change detection inside one process lifetime.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> u64 {
    fnv1a(FNV_OFFSET_BASIS, bytes)
}

/// Streams the same FNV-1a fingerprint as [`fingerprint`] through a fixed buffer.
///
/// Every chunk folds into the running hash, so the result is identical to
/// hashing the whole byte sequence at once. Read errors reach the caller:
/// inspection must not silently degrade into "unchanged".
pub fn fingerprint_reader<R: Read>(mut reader: R) -> Result<u64, std::io::Error> {
    let mut hash = FNV_OFFSET_BASIS;
    let mut buffer = [0_u8; FINGERPRINT_CHUNK];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(hash);
        }
        hash = fnv1a(hash, &buffer[..read]);
    }
}
