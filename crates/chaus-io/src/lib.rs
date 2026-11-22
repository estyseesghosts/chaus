//! File loading and saving.
//!
//! Loads only UTF-8 (with or without BOM), never lossily decodes, records each
//! original newline kind, and writes through a same-directory temporary file
//! followed by an OS-appropriate atomic replacement. A save is checked against
//! the identity and content hash captured at load time.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub mod recovery;

use chaus_core::eol::{Eol, EolMap};
use chaus_core::settings::Settings;
use tempfile::Builder;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const BINARY_NUL_RATIO: f64 = 0.01;

/// A stable description of the file content and identity at load/save time.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    /// The path actually written (symlinks resolved).
    pub canonical_target: PathBuf,
    /// The path the person asked for, used for titles and errors.
    pub user_requested_path: PathBuf,
    pub had_utf8_bom: bool,
    pub preferred_eol: Eol,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub identity: Option<FileIdentity>,
    /// FNV-1a hash of the raw bytes, independent of size/mtime.
    pub fingerprint: u64,
}

/// OS file identity where available.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

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
            identity: file_identity(path),
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

/// FNV-1a, used only for change detection inside one process lifetime.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn file_identity(path: &Path) -> Option<FileIdentity> {
    platform_identity(path)
}

#[cfg(unix)]
fn platform_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).ok()?;
    Some(FileIdentity {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_identity(path: &Path) -> Option<FileIdentity> {
    windows_identity(path)
}

#[cfg(not(any(unix, windows)))]
fn platform_identity(_path: &Path) -> Option<FileIdentity> {
    None
}

/// Number of hard links to `path`, where the platform can report it.
fn hard_link_count(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).ok().map(|metadata| metadata.nlink())
    }
    #[cfg(windows)]
    {
        windows_link_count(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Why a save may not proceed automatically.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveConflict {
    ChangedOnDisk,
    DeletedOnDisk,
}

impl std::fmt::Display for SaveConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChangedOnDisk => {
                write!(formatter, "file changed on disk since it was opened")
            }
            Self::DeletedOnDisk => {
                write!(formatter, "file was deleted on disk since it was opened")
            }
        }
    }
}

/// A baseline check that could neither confirm nor deny a conflict.
#[non_exhaustive]
#[derive(Debug)]
pub enum ConflictCheckError {
    Conflict(SaveConflict),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ConflictCheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict(conflict) => conflict.fmt(formatter),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ConflictCheckError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Conflict(_) => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

/// Compare the current on-disk target with the baseline captured at load.
///
/// A target that cannot be read is reported as [`ConflictCheckError::Io`],
/// never as "unchanged": proceeding with the save would overwrite content that
/// could not be inspected.
pub fn check_conflict(meta: &FileMeta) -> Result<(), ConflictCheckError> {
    let current = match fs::read(&meta.canonical_target) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConflictCheckError::Conflict(SaveConflict::DeletedOnDisk));
        }
        Err(source) => {
            return Err(ConflictCheckError::Io {
                path: meta.canonical_target.clone(),
                source,
            });
        }
    };
    let identity_changed = match (meta.identity, platform_identity(&meta.canonical_target)) {
        (Some(expected), Some(actual)) => expected != actual,
        _ => false,
    };
    if identity_changed || fingerprint(&current) != meta.fingerprint {
        return Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk));
    }
    Ok(())
}

/// One save request.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct SaveRequest<'a> {
    pub target: &'a Path,
    pub user_requested_path: &'a Path,
    pub bytes: &'a [u8],
    pub had_utf8_bom: bool,
    pub preferred_eol: Eol,
    /// Content baseline to compare against before overwriting, if any.
    pub baseline: Option<&'a FileMeta>,
}

impl<'a> SaveRequest<'a> {
    /// Build a save request. `baseline` is the metadata captured when the
    /// document was opened; pass `None` to write a new file.
    #[must_use]
    pub fn new(
        target: &'a Path,
        user_requested_path: &'a Path,
        bytes: &'a [u8],
        had_utf8_bom: bool,
        preferred_eol: Eol,
        baseline: Option<&'a FileMeta>,
    ) -> Self {
        Self {
            target,
            user_requested_path,
            bytes,
            had_utf8_bom,
            preferred_eol,
            baseline,
        }
    }
}

#[non_exhaustive]
#[derive(Debug)]
pub enum SaveError {
    Conflict(SaveConflict),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    ReplaceFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    MultipleLinks {
        path: PathBuf,
        links: u64,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Conflict(conflict) => conflict.fmt(formatter),
            SaveError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            SaveError::ReplaceFailed { path, source } => {
                write!(
                    formatter,
                    "{}: could not replace file: {source}",
                    path.display()
                )
            }
            SaveError::MultipleLinks { path, links } => write!(
                formatter,
                "{}: file has {links} hard links; save as a new file or break the links first",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io { source, .. } | SaveError::ReplaceFailed { source, .. } => Some(source),
            SaveError::Conflict(_) | SaveError::MultipleLinks { .. } => None,
        }
    }
}

/// Write `bytes` to `target` atomically and return the new baseline metadata.
///
/// The temporary file lives beside the target so the final commit stays on the
/// same filesystem. The original file is never deleted before replacement.
pub fn save(request: SaveRequest<'_>) -> Result<FileMeta, SaveError> {
    // Compare and commit against the same canonical spelling that `load`
    // recorded, otherwise Windows' `\\?\` prefix would defeat conflict checks.
    let target = canonical_spelling(request.target);
    let check_baseline = |baseline: Option<&FileMeta>| -> Result<(), SaveError> {
        let Some(baseline) = baseline else {
            return Ok(());
        };
        if baseline.canonical_target != target {
            return Ok(());
        }
        match check_conflict(baseline) {
            Ok(()) => Ok(()),
            Err(ConflictCheckError::Conflict(conflict)) => Err(SaveError::Conflict(conflict)),
            Err(ConflictCheckError::Io { path, source }) => Err(SaveError::Io { path, source }),
        }
    };
    check_baseline(request.baseline)?;
    if let Some(links) = hard_link_count(&target) {
        if links > 1 {
            return Err(SaveError::MultipleLinks {
                path: target.clone(),
                links,
            });
        }
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = Builder::new()
        .prefix(".chaus-save-")
        .tempfile_in(parent)
        .map_err(|source| SaveError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(request.bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| SaveError::Io {
            path: temporary.path().to_path_buf(),
            source,
        })?;
    if let Ok(existing) = fs::metadata(&target) {
        // Best effort: the replacement itself must still succeed when the
        // temporary file cannot inherit the old mode bits.
        let _ = fs::set_permissions(temporary.path(), existing.permissions());
    }

    let temporary_path = temporary.into_temp_path();
    // Revalidate immediately before the commit. This cannot make the check
    // atomic with the rename, but it narrows the window to the operation that
    // follows rather than the whole temporary-file write.
    check_baseline(request.baseline)?;
    commit(&temporary_path, &target)?;

    let size = request.bytes.len() as u64;
    Ok(FileMeta {
        canonical_target: target.clone(),
        user_requested_path: request.user_requested_path.to_path_buf(),
        had_utf8_bom: request.had_utf8_bom,
        preferred_eol: request.preferred_eol,
        size,
        modified: fs::metadata(&target)
            .ok()
            .and_then(|meta| meta.modified().ok()),
        identity: file_identity(&target),
        fingerprint: fingerprint(request.bytes),
    })
}

/// Canonical spelling even when the file itself is gone: canonicalize the
/// parent so a deleted target still compares equal to the load-time baseline.
fn canonical_spelling(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    match (fs::canonicalize(parent), path.file_name()) {
        (Ok(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

fn commit(temporary: &Path, target: &Path) -> Result<(), SaveError> {
    replace_path(temporary, target).map_err(|source| SaveError::ReplaceFailed {
        path: target.to_path_buf(),
        source,
    })
}

/// Replace `target` with `temporary` using OS-appropriate atomic semantics.
///
/// Shared by file saves and crash-recovery snapshots. The temporary file must
/// live on the same filesystem as the target. On Unix the parent directory is
/// synced after the rename; on Windows `ReplaceFileW` refuses to run when the
/// target is missing, so that case falls back to a move-with-replace.
pub(crate) fn replace_path(temporary: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::rename(temporary, target)?;
        if let Some(parent) = target.parent() {
            if let Ok(directory) = File::open(parent) {
                let _ = directory.sync_all();
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows_replace::replace(temporary, target)
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::rename(temporary, target)
    }
}

#[cfg(windows)]
mod windows_replace {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, MoveFileExW, ReplaceFileW, BY_HANDLE_FILE_INFORMATION,
        MOVEFILE_REPLACE_EXISTING,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn information(path: &Path) -> Option<BY_HANDLE_FILE_INFORMATION> {
        let file = std::fs::File::open(path).ok()?;
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: the file handle is valid for the duration of the call, and
        // the output pointer refers to writable storage for the structure.
        let success = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr())
        };
        if success == 0 {
            None
        } else {
            // SAFETY: GetFileInformationByHandle initialized the structure on
            // success.
            Some(unsafe { information.assume_init() })
        }
    }

    pub(crate) fn identity(path: &Path) -> Option<super::FileIdentity> {
        let information = information(path)?;
        Some(super::FileIdentity {
            volume: u64::from(information.dwVolumeSerialNumber),
            index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }

    pub(crate) fn link_count(path: &Path) -> Option<u64> {
        Some(u64::from(information(path)?.nNumberOfLinks))
    }

    pub fn replace(temporary: &Path, target: &Path) -> std::io::Result<()> {
        let temporary_wide = wide(temporary);
        let target_wide = wide(target);
        // ReplaceFileW preserves the target's ACLs and attributes and writes a
        // backup-free replacement. It requires the target to exist.
        //
        // SAFETY: both arguments are NUL-terminated UTF-16 buffers that remain
        // alive for the duration of the call; ReplaceFileW does not retain
        // them. The remaining arguments are documented to accept null.
        let replaced = unsafe {
            ReplaceFileW(
                target_wide.as_ptr(),
                temporary_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if replaced != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_FILE_NOT_FOUND as i32) {
            return Err(error);
        }
        // Target did not exist: a normal move-with-replace is correct.
        //
        // SAFETY: both arguments are NUL-terminated UTF-16 buffers that remain
        // alive for the duration of the call; MoveFileExW does not retain them.
        let moved = unsafe {
            MoveFileExW(
                temporary_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING,
            )
        };
        if moved == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
fn commit(temporary: &Path, target: &Path) -> Result<(), SaveError> {
    fs::rename(temporary, target).map_err(|source| SaveError::ReplaceFailed {
        path: target.to_path_buf(),
        source,
    })
}

/// Replace `target` with `bytes` atomically.
///
/// The temporary file lives in the target's directory and is synced before the
/// OS-appropriate replacement. A failed write leaves the previous file intact.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = Builder::new().prefix(".chaus-write-").tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if let Ok(existing) = fs::metadata(target) {
        let _ = fs::set_permissions(temporary.path(), existing.permissions());
    }
    let temporary_path = temporary.into_temp_path();
    replace_path(&temporary_path, target)
}

/// Persist theme and Vim-default settings to an rc file.
///
/// Comments, unrelated directives and the file's line-ending convention are
/// preserved. The existing file is re-read immediately before the atomic
/// commit; a concurrent external edit is reported instead of overwritten.
pub fn write_settings_rc(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let updated = chaus_core::settings::update_rc(&source, settings);
    let current = match fs::read_to_string(path) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    if current != source {
        return Err(std::io::Error::other(format!(
            "{} changed on disk; reopen Preferences and try again",
            path.display()
        )));
    }
    write_atomic(path, updated.as_bytes())
}

/// Serialize canonical document text to save bytes, re-adding a UTF-8 BOM when
/// the file originally had one.
#[must_use]
pub fn encode_for_save(text_with_eol: &str, had_utf8_bom: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text_with_eol.len() + UTF8_BOM.len());
    if had_utf8_bom {
        bytes.extend_from_slice(UTF8_BOM);
    }
    bytes.extend_from_slice(text_with_eol.as_bytes());
    bytes
}

#[cfg(windows)]
fn windows_identity(path: &Path) -> Option<FileIdentity> {
    windows_replace::identity(path)
}

#[cfg(windows)]
fn windows_link_count(path: &Path) -> Option<u64> {
    windows_replace::link_count(path)
}
