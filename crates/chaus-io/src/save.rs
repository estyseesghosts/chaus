//! Save orchestration and encoding.
//!
//! A save checks the load-time baseline, writes a synced temporary file beside
//! the target, rechecks immediately before the commit, then hands the commit to
//! the atomic-replace leaf. Any failure leaves the document dirty and the
//! original file intact, except for a durability-unconfirmed commit, which
//! reports the committed metadata so the caller can adopt a new baseline.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

use chaus_core::eol::Eol;
use tempfile::Builder;

use crate::atomic_replace::{replace_path, ReplaceError};
use crate::conflict::{check_conflict, ConflictCheckError, SaveConflict};
use crate::file_meta::{hard_link_count, platform_identity, FileMeta};
use crate::loaded_file::{fingerprint, UTF8_BOM};

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
    /// Fail with [`SaveError::Exists`] instead of replacing an existing path.
    pub refuse_existing: bool,
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
            refuse_existing: false,
        }
    }

    /// Refuse to replace anything already at the target path. Used by Ex
    /// writes to a new name without `!`.
    #[must_use]
    pub fn refusing_existing(mut self) -> Self {
        self.refuse_existing = true;
        self
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
    /// The replacement is visible on disk, but the directory entry could not
    /// be synced. This is not a failed write and the original was not kept.
    /// The target exists and the request asked not to replace it.
    Exists {
        path: PathBuf,
    },
    /// `meta` describes the committed content so callers can adopt it as the
    /// new baseline instead of reporting a false external change later.
    DurabilityUnconfirmed {
        path: PathBuf,
        source: std::io::Error,
        meta: Box<FileMeta>,
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
            SaveError::Exists { path } => write!(
                formatter,
                "{}: file exists (add ! to overwrite)",
                path.display()
            ),
            SaveError::DurabilityUnconfirmed { path, source, .. } => write!(
                formatter,
                "{}: replacement committed but directory sync failed ({source}); the new content is on disk",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io { source, .. }
            | SaveError::ReplaceFailed { source, .. }
            | SaveError::DurabilityUnconfirmed { source, .. } => Some(source),
            SaveError::Conflict(_) | SaveError::MultipleLinks { .. } | SaveError::Exists { .. } => {
                None
            }
        }
    }
}

type PreReplaceGate = (mpsc::Receiver<()>, Option<mpsc::Sender<()>>);

static PRE_REPLACE_GATE: Mutex<Option<PreReplaceGate>> = Mutex::new(None);

/// Test seam: pause a writer after its final conflict check and before commit.
/// The optional sender signals that the gate has been reached.
pub fn set_pre_replace_gate_for_test(gate: Option<PreReplaceGate>) {
    if let Ok(mut slot) = PRE_REPLACE_GATE.lock() {
        *slot = gate;
    }
}

fn wait_for_pre_replace_gate() {
    let gate = PRE_REPLACE_GATE
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some((release, reached)) = gate {
        if let Some(reached) = reached {
            let _ = reached.send(());
        }
        let _ = release.recv();
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
    let check_absent = || -> Result<(), SaveError> {
        // `symlink_metadata` so a dangling symlink also counts as occupied.
        if request.refuse_existing && fs::symlink_metadata(&target).is_ok() {
            return Err(SaveError::Exists {
                path: target.clone(),
            });
        }
        Ok(())
    };
    check_baseline(request.baseline)?;
    check_absent()?;
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
    wait_for_pre_replace_gate();
    // The gate exists to test the check/commit boundary. Recheck afterwards so
    // a writer that was paused while another save landed cannot overwrite it
    // silently. This still cannot make the check and rename one OS operation.
    check_baseline(request.baseline)?;
    check_absent()?;
    let committed = commit(&temporary_path, &target);

    let size = request.bytes.len() as u64;
    let meta = FileMeta {
        canonical_target: target.clone(),
        user_requested_path: request.user_requested_path.to_path_buf(),
        had_utf8_bom: request.had_utf8_bom,
        preferred_eol: request.preferred_eol,
        size,
        modified: fs::metadata(&target)
            .ok()
            .and_then(|meta| meta.modified().ok()),
        identity: platform_identity(&target),
        fingerprint: fingerprint(request.bytes),
    };
    match committed {
        Ok(()) => Ok(meta),
        Err(Unconfirmed::Durability(source)) => Err(SaveError::DurabilityUnconfirmed {
            path: target,
            source,
            meta: Box::new(meta),
        }),
        Err(Unconfirmed::Failed(error)) => Err(error),
    }
}

/// A commit either failed outright or landed without a confirmed directory sync.
enum Unconfirmed {
    Failed(SaveError),
    Durability(std::io::Error),
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

fn commit(temporary: &Path, target: &Path) -> Result<(), Unconfirmed> {
    replace_path(temporary, target).map_err(|error| match error {
        ReplaceError::Io(source) => Unconfirmed::Failed(SaveError::ReplaceFailed {
            path: target.to_path_buf(),
            source,
        }),
        ReplaceError::DurabilityUnconfirmed(source) => Unconfirmed::Durability(source),
    })
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
