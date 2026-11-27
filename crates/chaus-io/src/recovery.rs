//! Crash-recovery snapshots.
//!
//! Snapshots live in the platform state directory, never beside the original
//! file. Each record is written atomically under a stable key derived from the
//! editor window, so repeated debounced writes replace the previous snapshot.
//!
//! Startup discovery is header-first: [`scan_in`] reads only the fixed header
//! and the path, so a large recovery backlog is a handful of small reads.
//! [`RecoveryEntry::read`] loads one body only when the person chooses it.
//!
//! This stays a leaf of `chaus-io` rather than becoming `chaus-recovery`.
//! `remove_saved_matches` calls the document loader to compare a snapshot with
//! a file saved after it, and writes commit through the same atomic-replace
//! primitive as document saves. Splitting either out would duplicate the
//! load/validation or platform replacement contract, which cleanup.md Task 7
//! forbids. The UI that offers snapshots to the person remains in `chaus-editor`.

use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chaus_core::eol::Eol;
use tempfile::Builder;

const MAGIC: &[u8; 8] = b"CHAUSR01";
/// Largest serialized snapshot the reader accepts, including header and path.
pub const MAX_RECORD_BYTES: u64 = 64 * 1024 * 1024;
/// Longest original path stored in a header. Longer headers are corrupt.
pub const MAX_HEADER_PATH_BYTES: usize = 8 * 1024;
/// Snapshot older than this is not offered. Its document has not been touched
/// for a month; keeping it only grows the prompt backlog.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const FILE_PREFIX: &str = "recovery-";
const FILE_SUFFIX: &str = ".rec";
/// Fixed header fields before the optional path. `MAGIC`, id, EOL, BOM,
/// revision, seconds, path flag, path length and text length.
const HEADER_FIXED_BYTES: u64 = 8 + 8 + 1 + 1 + 8 + 8 + 1 + 4 + 8;

/// Serializes writes and removals so a snapshot update is never interleaved
/// with another snapshot update for the same window.
static WRITE_LOCK: Mutex<()> = Mutex::new(());
/// Monotonic ordering for callers that do not supply an editor epoch.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
/// Highest writer epoch accepted per `(directory, id)`.
static WATERMARKS: Mutex<Vec<Watermark>> = Mutex::new(Vec::new());
/// Test seam: hold the next retirement until the test releases the receiver.
/// Normal runs always leave this `None`.
static RETIRE_GATE: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);

/// Application-owned accounting for recovery work that must finish before the
/// process exits. A retirement can otherwise be racing the last window close,
/// leaving a `.rec` file that comes back on the next launch.
static PENDING_OPERATIONS: Mutex<usize> = Mutex::new(0);
static PENDING_CONDVAR: Condvar = Condvar::new();

/// Install a one-shot gate in front of the next snapshot retirement.
pub fn set_retire_gate_for_test(receiver: Option<mpsc::Receiver<()>>) {
    *RETIRE_GATE.lock().expect("recovery retire gate poisoned") = receiver;
}

fn wait_for_retire_gate() {
    let receiver = RETIRE_GATE
        .lock()
        .expect("recovery retire gate poisoned")
        .take();
    if let Some(receiver) = receiver {
        let _ = receiver.recv();
    }
}

/// RAII ticket for one recovery write or retirement. [`drain_pending`] waits
/// on these tickets, so the application can bound shutdown without knowing
/// which window scheduled the work.
#[must_use = "the operation is only tracked while the ticket is alive"]
pub struct PendingRecoveryOp {
    _private: (),
}

impl PendingRecoveryOp {
    /// Register a new in-flight operation.
    pub fn begin() -> Self {
        let mut active = PENDING_OPERATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_add(1);
        Self { _private: () }
    }
}

impl Drop for PendingRecoveryOp {
    fn drop(&mut self) {
        let mut active = PENDING_OPERATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        PENDING_CONDVAR.notify_all();
    }
}

/// Recovery writes and retirements currently in flight.
#[must_use]
pub fn pending_operations() -> usize {
    *PENDING_OPERATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Wait for every tracked operation to finish. Returns `false` when the
/// timeout expired first.
pub fn drain_pending(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut active = PENDING_OPERATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    while *active > 0 {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let (next, wait) = PENDING_CONDVAR
            .wait_timeout(active, deadline - now)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        active = next;
        if wait.timed_out() && *active > 0 {
            return false;
        }
    }
    true
}

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

    /// Basename shown in compact status text.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.original_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Full spelling for the recovery sheet: ten `README.md` basenames are
    /// otherwise indistinguishable.
    #[must_use]
    pub fn display_detail(&self) -> String {
        self.original_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Untitled snapshot".to_string())
    }

    /// Human-readable snapshot age for the recovery sheet.
    #[must_use]
    pub fn saved_label(&self) -> String {
        saved_label(self.saved_at)
    }
}

/// Header-only description of one snapshot on disk.
///
/// Discovery reads the fixed header and path, never the body. The body is read
/// only by [`RecoveryEntry::read`] after the person chooses Recover.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryEntry {
    pub id: u64,
    pub original_path: Option<PathBuf>,
    pub preferred_eol: Eol,
    pub had_utf8_bom: bool,
    pub revision: u64,
    pub saved_at: SystemTime,
    /// Bytes in the stored body. The whole file must be exactly this plus the
    /// header, so a truncated or padded file is rejected without reading it.
    pub text_len: u64,
    /// Canonical snapshot path in the recovery directory.
    pub path: PathBuf,
}

impl RecoveryEntry {
    /// Basename shown in compact status text.
    #[must_use]
    pub fn display_name(&self) -> String {
        self.original_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Full spelling for the recovery sheet.
    #[must_use]
    pub fn display_detail(&self) -> String {
        self.original_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Untitled snapshot".to_string())
    }

    /// Human-readable snapshot age for the recovery sheet.
    #[must_use]
    pub fn saved_label(&self) -> String {
        saved_label(self.saved_at)
    }

    /// Load the document body. Revalidates the embedded id, so a snapshot file
    /// replaced between scan and read is not accepted under the wrong name.
    #[must_use]
    pub fn read(&self) -> Option<RecoveryRecord> {
        read_entry(self)
    }
}

fn saved_label(saved_at: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(saved_at)
        .unwrap_or_default()
        .as_secs();
    match age {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", age / 60),
        3600..=86_399 => format!("{} h ago", age / 3600),
        _ => format!("{} d ago", age / 86_400),
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
    crate::atomic_replace::replace_path(&temporary_path, &target)
        .map_err(|error| error.into_io())?;
    record_watermark(directory, record.id, epoch, record.revision)?;
    Ok(target)
}

/// Every readable snapshot in the platform recovery directory, newest first.
///
/// Startup uses [`scan`] instead: this compatibility helper reads every body.
#[must_use]
pub fn list() -> Vec<RecoveryRecord> {
    let Some(directory) = recovery_directory() else {
        return Vec::new();
    };
    list_in(&directory)
}

/// Every readable snapshot in an explicit directory, newest first. Corrupt or
/// unrelated files are skipped.
///
/// This reads every body and applies no age limit; it exists for tests and
/// callers that actually need complete records. Startup discovery is
/// [`scan_in`].
#[must_use]
pub fn list_in(directory: &Path) -> Vec<RecoveryRecord> {
    scan_in_with_max_age(directory, None)
        .iter()
        .filter_map(read_entry)
        .collect()
}

/// Header-only snapshots in the platform recovery directory.
#[must_use]
pub fn scan() -> Vec<RecoveryEntry> {
    let Some(directory) = recovery_directory() else {
        return Vec::new();
    };
    scan_in(&directory)
}

/// Header-only snapshots in an explicit directory, newest first. Applies the
/// default age limit and prunes duplicates and snapshots already matched by
/// the saved file.
#[must_use]
pub fn scan_in(directory: &Path) -> Vec<RecoveryEntry> {
    scan_in_with_max_age(directory, Some(DEFAULT_MAX_AGE))
}

/// Header-only snapshots with no age limit. Used by `list_in` and tests that
/// need older fixtures.
#[must_use]
pub fn scan_in_without_age(directory: &Path) -> Vec<RecoveryEntry> {
    scan_in_with_max_age(directory, None)
}

/// Header-first discovery with optional age pruning.
///
/// Bodies are read only for two cleanup decisions: deciding whether two
/// snapshots for the same path are exact duplicates, and checking whether a
/// snapshot still matches a file saved after it. The returned entries carry no
/// text, so a backlog of 100 snapshots costs 100 small header reads up front.
#[must_use]
pub fn scan_in_with_max_age(directory: &Path, max_age: Option<Duration>) -> Vec<RecoveryEntry> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut scanned = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let Some(file_id) = id_from_file_name(name) else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
            continue;
        }
        let Some(entry) = read_header(&path, metadata.len()) else {
            continue;
        };
        // The filename is part of the record's identity. A copied or renamed
        // snapshot whose embedded id disagrees would otherwise be retired
        // under its canonical name and come back forever.
        if entry.id != file_id {
            continue;
        }
        if max_age.is_some_and(|limit| {
            now.duration_since(entry.saved_at)
                .is_ok_and(|age| age > limit)
        }) {
            let _ = remove_entry(&entry);
            continue;
        }
        scanned.push(entry);
    }
    scanned.sort_by(|left, right| {
        right
            .saved_at
            .cmp(&left.saved_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    remove_exact_duplicates(&mut scanned);
    remove_saved_matches(&mut scanned);
    scanned
}

/// Collapse snapshots for the same original file only when their bodies are
/// byte-identical after newline normalization. Different unsaved work is kept.
fn remove_exact_duplicates(entries: &mut Vec<RecoveryEntry>) {
    let mut index = 0;
    while index < entries.len() {
        let Some(path) = entries[index].original_path.clone() else {
            index += 1;
            continue;
        };
        let mut duplicate = None;
        for later in index + 1..entries.len() {
            if entries[later].original_path.as_ref() != Some(&path) {
                continue;
            }
            let (Some(current), Some(other)) =
                (read_entry(&entries[index]), read_entry(&entries[later]))
            else {
                continue;
            };
            if current.preferred_eol == other.preferred_eol
                && current.had_utf8_bom == other.had_utf8_bom
                && current.text == other.text
            {
                duplicate = Some(later);
                break;
            }
        }
        if let Some(later) = duplicate {
            let retired = entries.remove(later);
            let _ = remove_entry(&retired);
        } else {
            index += 1;
        }
    }
}

/// Drop a snapshot whose original file has since been saved with identical
/// contents. Only candidates whose file is newer than the snapshot are read,
/// so a normal backlog stays header-only.
fn remove_saved_matches(entries: &mut Vec<RecoveryEntry>) {
    let mut index = 0;
    while index < entries.len() {
        let Some(path) = entries[index].original_path.clone() else {
            index += 1;
            continue;
        };
        let Ok(metadata) = fs::metadata(&path) else {
            index += 1;
            continue;
        };
        let file_is_newer = metadata
            .modified()
            .ok()
            .is_some_and(|modified| modified > entries[index].saved_at);
        if !file_is_newer {
            index += 1;
            continue;
        }
        let Some(record) = read_entry(&entries[index]) else {
            index += 1;
            continue;
        };
        if let Ok(loaded) = crate::loaded_file::load(&path) {
            if loaded.text == normalized(&record.text) {
                let retired = entries.remove(index);
                let _ = remove_entry(&retired);
                continue;
            }
        }
        index += 1;
    }
}

/// Text with every original newline kind mapped back to `\n`.
fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
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
    wait_for_retire_gate();
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery retire"))?;
    record_watermark(directory, id, epoch, u64::MAX)?;
    remove_file(directory, id)
}

/// Retire the snapshot file a window already owns.
///
/// `path` is the `.rec` file this window adopted when it recovered. Removing
/// that exact path keeps the operation correct even when the platform recovery
/// directory was overridden for this process or changed between launches.
pub fn retire_path(path: &Path, id: u64, epoch: u64) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    retire_in(directory, id, epoch)
}

/// Retire several snapshots under one lock and ordering epoch. Bulk discard
/// must not pay a lock round-trip per record.
pub fn retire_many(ids: impl IntoIterator<Item = u64>, epoch: u64) -> std::io::Result<()> {
    let Some(directory) = recovery_directory() else {
        return Ok(());
    };
    retire_many_in(&directory, ids, epoch)
}

/// Retire the exact files described by discovery entries.
///
/// This removes `entry.path`, not a canonical name reconstructed from the id,
/// so a snapshot that reached the queue under another spelling is still the
/// file that is deleted.
pub fn retire_entries(entries: &[RecoveryEntry], epoch: u64) -> std::io::Result<()> {
    let mut groups: Vec<(PathBuf, Vec<&RecoveryEntry>)> = Vec::new();
    for entry in entries {
        let directory = entry
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        match groups
            .iter_mut()
            .find(|(existing, _)| *existing == directory)
        {
            Some((_, grouped)) => grouped.push(entry),
            None => groups.push((directory, vec![entry])),
        }
    }
    for (directory, grouped) in groups {
        retire_entry_files(&directory, &grouped, epoch)?;
    }
    Ok(())
}

fn retire_entry_files(
    directory: &Path,
    entries: &[&RecoveryEntry],
    epoch: u64,
) -> std::io::Result<()> {
    wait_for_retire_gate();
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery retire"))?;
    for entry in entries {
        record_watermark(directory, entry.id, epoch, u64::MAX)?;
        remove_path(&entry.path)?;
    }
    Ok(())
}

/// Remove the exact file an entry described.
fn remove_entry(entry: &RecoveryEntry) -> std::io::Result<()> {
    let directory = entry.path.parent().unwrap_or_else(|| Path::new("."));
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery remove"))?;
    record_watermark(directory, entry.id, u64::MAX, u64::MAX)?;
    remove_path(&entry.path)
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Retire several snapshots in an explicit directory.
pub fn retire_many_in(
    directory: &Path,
    ids: impl IntoIterator<Item = u64>,
    epoch: u64,
) -> std::io::Result<()> {
    wait_for_retire_gate();
    let _guard = WRITE_LOCK.lock().map_err(|_| poisoned("recovery retire"))?;
    for id in ids {
        record_watermark(directory, id, epoch, u64::MAX)?;
        remove_file(directory, id)?;
    }
    Ok(())
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

fn id_from_file_name(name: &str) -> Option<u64> {
    let hex = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
    if hex.is_empty() || !hex.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
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

/// Read only the header and path. `file_len` is checked against the declared
/// body length so a truncated copy is rejected before anyone allocates it.
fn read_header(path: &Path, file_len: u64) -> Option<RecoveryEntry> {
    let limit = HEADER_FIXED_BYTES + MAX_HEADER_PATH_BYTES as u64;
    let mut bytes = Vec::with_capacity(limit.min(file_len) as usize);
    File::open(path)
        .ok()?
        .take(limit)
        .read_to_end(&mut bytes)
        .ok()?;
    decode_header(path, &bytes, file_len)
}

fn decode_header(path: &Path, bytes: &[u8], file_len: u64) -> Option<RecoveryEntry> {
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
        if length > MAX_HEADER_PATH_BYTES {
            return None;
        }
        let path = std::str::from_utf8(reader.take(length)?).ok()?;
        Some(PathBuf::from(path))
    } else {
        None
    };
    let text_len = reader.u64()?;
    let body_offset = reader.offset as u64;
    let expected = body_offset.checked_add(text_len)?;
    if expected != file_len || file_len > MAX_RECORD_BYTES {
        return None;
    }
    Some(RecoveryEntry {
        id,
        original_path,
        preferred_eol,
        had_utf8_bom,
        revision,
        saved_at: UNIX_EPOCH + Duration::from_secs(seconds),
        text_len,
        path: path.to_path_buf(),
    })
}

fn read_entry(entry: &RecoveryEntry) -> Option<RecoveryRecord> {
    let metadata = fs::metadata(&entry.path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_RECORD_BYTES {
        return None;
    }
    let bytes = fs::read(&entry.path).ok()?;
    let record = decode(&bytes)?;
    (record.id == entry.id).then_some(record)
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
    if reader.offset != bytes.len() {
        return None;
    }
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
