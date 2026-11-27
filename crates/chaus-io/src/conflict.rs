//! Save-time conflict checks against the baseline captured at load.
//!
//! A cheap metadata pre-check lets an idle watcher skip hashing a large file.
//! It is not authoritative: the save path always hashes the current bytes and
//! re-reads identity so a same-size, same-mtime edit is still a conflict.

use std::fs::{self, File};
use std::path::PathBuf;

use crate::file_meta::{platform_identity, FileMeta};
use crate::loaded_file::fingerprint_reader;

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

/// Cheap pre-check: does the target's identity, size and mtime still match?
///
/// This exists so a periodic watcher can skip reading and hashing a large file
/// on idle ticks. It is deliberately not a conflict check: a matching result
/// means "no cheap evidence of change", not "content is unchanged". Callers
/// must still run [`check_conflict`] at save time, and should periodically hash
/// anyway, because timestamp granularity can hide a same-size edit.
#[must_use]
pub fn metadata_matches(meta: &FileMeta) -> bool {
    let Ok(current) = fs::metadata(&meta.canonical_target) else {
        return false;
    };
    if !current.is_file() || current.len() != meta.size {
        return false;
    }
    if let (Some(expected), Some(actual)) = (meta.modified, current.modified().ok()) {
        if expected != actual {
            return false;
        }
    }
    if let (Some(expected), Some(actual)) =
        (meta.identity, platform_identity(&meta.canonical_target))
    {
        if expected != actual {
            return false;
        }
    }
    true
}

/// Compare the current on-disk target with the baseline captured at load.
///
/// A target that cannot be read is reported as [`ConflictCheckError::Io`],
/// never as "unchanged": proceeding with the save would overwrite content that
/// could not be inspected.
pub fn check_conflict(meta: &FileMeta) -> Result<(), ConflictCheckError> {
    // Open first so a missing target stays a deletion conflict. A directory or
    // a read error after a successful open is an inspection failure, never a
    // clean result: callers may be about to overwrite it.
    let file = match File::open(&meta.canonical_target) {
        Ok(file) => file,
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
    let current = fingerprint_reader(file).map_err(|source| ConflictCheckError::Io {
        path: meta.canonical_target.clone(),
        source,
    })?;
    // Identity is checked after hashing, not as a shortcut: an unreadable
    // replacement must still surface as Io rather than an already-known change.
    let identity_changed = match (meta.identity, platform_identity(&meta.canonical_target)) {
        (Some(expected), Some(actual)) => expected != actual,
        _ => false,
    };
    if identity_changed || current != meta.fingerprint {
        return Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk));
    }
    Ok(())
}
