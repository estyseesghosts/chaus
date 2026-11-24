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

/// What happens to the window after a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseAfter {
    /// Keep the window open.
    No,
    /// Close on success; on failure or cancel re-show the dirty-close sheet.
    Sheet,
    /// `:wq`/`:x`: close on success; on failure keep the window and report,
    /// without opening the GUI sheet.
    Ex,
    /// `:wqa`: keep the window open; the orchestrator closes every window only
    /// after all required saves have succeeded.
    Wqa,
}

impl CloseAfter {
    pub(crate) fn closes(self) -> bool {
        matches!(self, Self::Sheet | Self::Ex)
    }
}

/// Whether a finished write changes the document's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteAssociation {
    /// Save, Save As, `:saveas` and a first `:w path`: the target becomes the
    /// document's file and the captured revision is marked saved.
    Associate,
    /// `:w path` on a named document: the bytes land at `path`, but the file,
    /// dirty state and crash-recovery snapshot are untouched.
    Copy,
}

/// Immutable details captured when a save starts. They travel with the
/// request so the write task does not need a live editor borrow.
#[derive(Debug)]
pub(crate) struct SaveTarget {
    pub(crate) target: PathBuf,
    pub(crate) requested: PathBuf,
    pub(crate) had_utf8_bom: bool,
    pub(crate) baseline: Option<FileMeta>,
    pub(crate) close_after: CloseAfter,
    pub(crate) association: WriteAssociation,
    /// Fail instead of replacing an existing file (Ex writes without `!`).
    pub(crate) refuse_existing: bool,
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
        close_after: CloseAfter,
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
