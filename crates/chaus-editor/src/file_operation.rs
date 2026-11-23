//! Identity and lifetime of a per-editor file operation.
//!
//! Open and save complete on background tasks. A completion is only allowed to
//! change editor state when it still owns the latest request and the document
//! it started against. A file chooser is part of the same request, so a second
//! Save dispatched while a picker is open cannot start a competing write.

use std::path::PathBuf;

use chaus_io::FileMeta;

/// Monotonic identity of one user-requested open or save, including its picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileRequestId(u64);

impl FileRequestId {
    /// Issue the next id. Wrapping is harmless because only equality matters.
    pub(crate) fn next(counter: &mut u64) -> Self {
        *counter = counter.wrapping_add(1);
        Self(*counter)
    }
}

/// Distinguishes documents replaced in this window from documents merely edited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentGeneration(u64);

impl DocumentGeneration {
    pub(crate) fn bump(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

/// Identity captured when an operation starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileRequest {
    pub(crate) id: FileRequestId,
    pub(crate) document: DocumentGeneration,
    pub(crate) state: u64,
}

/// Immutable details captured when a save starts. They travel with the
/// request so the write task does not need a live editor borrow.
#[derive(Debug)]
pub(crate) struct SaveTarget {
    pub(crate) target: PathBuf,
    pub(crate) requested: PathBuf,
    pub(crate) had_utf8_bom: bool,
    pub(crate) baseline: Option<FileMeta>,
    pub(crate) close_after: bool,
}

/// The single authoritative state of file I/O owned by one editor.
#[derive(Debug)]
pub(crate) enum FileOperation {
    Idle,
    Opening {
        request: FileRequest,
        path: PathBuf,
        force: bool,
    },
    ChoosingSaveTarget {
        request: FileRequest,
        close_after: bool,
        had_utf8_bom: bool,
    },
    Saving {
        request: FileRequest,
    },
}

impl FileOperation {
    pub(crate) fn request(&self) -> Option<FileRequest> {
        match self {
            Self::Idle => None,
            Self::Opening { request, .. }
            | Self::ChoosingSaveTarget { request, .. }
            | Self::Saving { request, .. } => Some(*request),
        }
    }

    pub(crate) fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
}
