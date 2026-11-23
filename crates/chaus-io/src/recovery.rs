//! Crash-recovery snapshots.
//!
//! Snapshots live in the platform state directory, never beside the original
//! file. Each record is written atomically under a stable key derived from the
//! editor window, so repeated debounced writes replace the previous snapshot.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chaus_core::eol::Eol;
use tempfile::Builder;

const MAGIC: &[u8; 8] = b"CHAUSR01";
/// Largest serialized snapshot the reader accepts, including header and path.
pub const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;
const FILE_PREFIX: &str = "recovery-";
const FILE_SUFFIX: &str = ".rec";

/// Serializes writes and removals so a snapshot update is never interleaved
/// with another snapshot update for the same window.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
/// Monotonic ordering for callers that do not supply an editor epoch.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
/// Highest writer epoch accepted per `(directory, id)`.
static WATERMARKS: Mutex<Vec<Watermark>> = Mutex::new(Vec::new());

struct Watermark {
    directory: PathBuf,
    id: u64,
    epoch: u64,
    revision: u64,
}

/// Why a recovery snapshot could not be written.
#[non_exhaustive]
#[derive(Debug)]
pub enum RecoveryError {
    /// No writable platform state directory is configured.
    NoDirectory,
    /// The serialized record (including path and header) exceeds the readable
    /// limit, so accepting it would create a snapshot that startup discards.
    TooLarge { bytes: u64, limit: u64 },
    /// The filesystem rejected the write or replacement.
    Io(std::io::Error),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDirectory => write!(formatter, "no platform state directory for recovery"),
            Self::TooLarge { bytes, limit } => write!(
                formatter,
                "recovery snapshot is {bytes} bytes; the limit is {limit} bytes"
            ),
            Self::Io(error) => write!(formatter, "recovery snapshot failed: {error}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<std::io::Error> for RecoveryError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// One unsaved document snapshot.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRecord {
    /// Stable per-window key; repeated snapshots of the same window reuse it.
    pub id: u64,
    /// The file the document came from, if any.
    pub original_path: Option<PathBuf>,
    /// Expanded text with the document's original newline kinds.
    pub text: String,
    pub preferred_eol: Eol,
    pub had_utf8_bom: bool,
    pub revision: u64,
    pub saved_at: SystemTime,
}

impl RecoveryRecord {
    #[must_use]
    pub fn new(
        id: u64,
        original_path: Option<PathBuf>,
        text: impl Into<String>,
        preferred_eol: Eol,
        had_utf8_bom: bool,
        revision: u64,
    ) -> Self {
        Self {
            id,
            original_path,
            text: text.into(),
            preferred_eol,
            had_utf8_bom,
            revision,
            saved_at: SystemTime::now(),
        }
    }

    /// Name shown in the recovery sheet.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.original_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }
}

/// Platform state directory for recovery records.
///
/// `CHAUS_RECOVERY_DIR` overrides it for tests and portable runs.
#[must_use]
pub fn recovery_directory() -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os("CHAUS_RECOVERY_DIR") {
        return Some(PathBuf::from(directory));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Chaus").join("Recovery"));
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .map(|base| base.join("chaus").join("recovery"))
}

/// Write a snapshot to the platform recovery directory.
pub fn write(record: &RecoveryRecord) -> Result<PathBuf, RecoveryError> {
    let directory = recovery_directory().ok_or(RecoveryError::NoDirectory)?;
    write_to(&directory, record)
}

/// Write a snapshot using an explicit ordering epoch.
///
/// Only the highest epoch for a snapshot id is retained. A stale debounced
/// write that finishes after a newer edit or after the record was retired is
/// discarded instead of resurrecting older text.
pub fn write_with_epoch(record: &RecoveryRecord, epoch: u64) -> Result<PathBuf, RecoveryError> {
    let directory = recovery_directory().ok_or(RecoveryError::NoDirectory)?;
    write_to_with_epoch(&directory, record, epoch)
}

/// Write a snapshot to an explicit directory. Existing snapshots with the same
/// id are replaced. Call order supplies the ordering epoch.
pub fn write_to(directory: &Path, record: &RecoveryRecord) -> Result<PathBuf, RecoveryError> {
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    write_to_with_epoch(directory, record, epoch)
}

/// Write a snapshot to an explicit directory with an explicit ordering epoch.
pub fn write_to_with_epoch(
    directory: &Path,
    record: &RecoveryRecord,
    epoch: u64,
) -> Result<PathBuf, RecoveryError> {
    // Check the complete serialized size before touching the filesystem. A
    // record the reader would discard must never replace a usable snapshot.
    let bytes = encode_checked(record)?;
    fs::create_dir_all(directory)?;
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery write"))?;
    let target = directory.join(file_name(record.id));
    if is_stale(directory, record.id, epoch, record.revision)? {
        return Ok(target);
    }

    // Unique same-directory temporary: a crash before the replacement leaves
    // the previous record intact, and concurrent writes cannot collide.
    let mut temporary = Builder::new()
        .prefix(".chaus-recovery-")
        .tempfile_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    let temporary_path = temporary.into_temp_path();
    crate::replace_path(&temporary_path, &target)?;
    record_watermark(directory, record.id, epoch, record.revision)?;
    Ok(target)
}

/// Every readable snapshot in the platform recovery directory, newest first.
#[must_use]
pub fn list() -> Vec<RecoveryRecord> {
    let Some(directory) = recovery_directory() else {
        return Vec::new();
    };
    list_in(&directory)
}

/// Every readable snapshot in an explicit directory, newest first. Corrupt or
/// unrelated files are skipped.
#[must_use]
pub fn list_in(directory: &Path) -> Vec<RecoveryRecord> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if !name.starts_with(FILE_PREFIX) || !name.ends_with(FILE_SUFFIX) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if let Some(record) = decode(&bytes) {
            records.push(record);
        }
    }
    records.sort_by(|left, right| {
        right
            .saved_at
            .cmp(&left.saved_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    records
}

/// Retire a snapshot with an explicit ordering epoch.
///
/// Pending writes from older epochs for the same id are discarded. Later edits
/// can still write again once their epoch advances.
pub fn retire(id: u64, epoch: u64) -> std::io::Result<()> {
    let Some(directory) = recovery_directory() else {
        return Ok(());
    };
    retire_in(&directory, id, epoch)
}

/// Retire a snapshot in an explicit directory with an explicit epoch.
pub fn retire_in(directory: &Path, id: u64, epoch: u64) -> std::io::Result<()> {
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery retire"))?;
    record_watermark(directory, id, epoch, u64::MAX)?;
    remove_file(directory, id)
}

/// Remove a snapshot from the platform recovery directory permanently.
pub fn remove(id: u64) -> std::io::Result<()> {
    let Some(directory) = recovery_directory() else {
        return Ok(());
    };
    remove_in(&directory, id)
}

/// Remove a snapshot from an explicit directory. Missing files are ignored.
pub fn remove_in(directory: &Path, id: u64) -> std::io::Result<()> {
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery remove"))?;
    record_watermark(directory, id, u64::MAX, u64::MAX)?;
    remove_file(directory, id)
}

fn remove_file(directory: &Path, id: u64) -> std::io::Result<()> {
    match fs::remove_file(directory.join(file_name(id))) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn poisoned(context: &str) -> std::io::Error {
    std::io::Error::other(format!("{context} lock is poisoned"))
}

fn is_stale(directory: &Path, id: u64, epoch: u64, revision: u64) -> std::io::Result<bool> {
    let marks = WATERMARKS
        .lock()
        .map_err(|_| poisoned("recovery watermark"))?;
    Ok(marks
        .iter()
        .find(|mark| mark.id == id && mark.directory == directory)
        .is_some_and(|mark| {
            epoch < mark.epoch || (epoch == mark.epoch && revision <= mark.revision)
        }))
}

fn record_watermark(directory: &Path, id: u64, epoch: u64, revision: u64) -> std::io::Result<()> {
    let mut marks = WATERMARKS
        .lock()
        .map_err(|_| poisoned("recovery watermark"))?;
    if let Some(mark) = marks
        .iter_mut()
        .find(|mark| mark.id == id && mark.directory == directory)
    {
        if epoch >= mark.epoch {
            mark.epoch = epoch;
            mark.revision = revision;
        }
        return Ok(());
    }
    marks.push(Watermark {
        directory: directory.to_path_buf(),
        id,
        epoch,
        revision,
    });
    Ok(())
}

fn file_name(id: u64) -> String {
    format!("{FILE_PREFIX}{id:016x}{FILE_SUFFIX}")
}

fn encode_checked(record: &RecoveryRecord) -> Result<Vec<u8>, RecoveryError> {
    let bytes = encode(record);
    let size = bytes.len() as u64;
    if size > MAX_RECORD_BYTES {
        return Err(RecoveryError::TooLarge {
            bytes: size,
            limit: MAX_RECORD_BYTES,
        });
    }
    Ok(bytes)
}

fn encode(record: &RecoveryRecord) -> Vec<u8> {
    let path = record
        .original_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let seconds = record
        .saved_at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let mut bytes = Vec::with_capacity(64 + record.text.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&record.id.to_le_bytes());
    bytes.push(eol_byte(record.preferred_eol));
    bytes.push(u8::from(record.had_utf8_bom));
    bytes.extend_from_slice(&record.revision.to_le_bytes());
    bytes.extend_from_slice(&seconds.to_le_bytes());
    match &path {
        Some(path) => {
            bytes.push(1);
            let path = path.as_bytes();
            bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
            bytes.extend_from_slice(path);
        }
        None => bytes.push(0),
    }
    let text = record.text.as_bytes();
    bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(text);
    bytes
}

fn decode(bytes: &[u8]) -> Option<RecoveryRecord> {
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(MAGIC.len())? != MAGIC {
        return None;
    }
    let id = reader.u64()?;
    let preferred_eol = eol_from_byte(reader.u8()?)?;
    let had_utf8_bom = reader.u8()? != 0;
    let revision = reader.u64()?;
    let seconds = reader.u64()?;
    let original_path = if reader.u8()? == 1 {
        let length = reader.u32()? as usize;
        let path = std::str::from_utf8(reader.take(length)?).ok()?;
        Some(PathBuf::from(path))
    } else {
        None
    };
    let text_length = reader.u64()?;
    let text_length = usize::try_from(text_length).ok()?;
    let text = std::str::from_utf8(reader.take(text_length)?).ok()?;
    Some(RecoveryRecord {
        id,
        original_path,
        text: text.to_string(),
        preferred_eol,
        had_utf8_bom,
        revision,
        saved_at: UNIX_EPOCH + std::time::Duration::from_secs(seconds),
    })
}

fn eol_byte(eol: Eol) -> u8 {
    match eol {
        Eol::Lf => 0,
        Eol::CrLf => 1,
        Eol::Cr => 2,
    }
}

fn eol_from_byte(byte: u8) -> Option<Eol> {
    match byte {
        0 => Some(Eol::Lf),
        1 => Some(Eol::CrLf),
        2 => Some(Eol::Cr),
        _ => None,
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(length)?;
        let slice = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    }
}
