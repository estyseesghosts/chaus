//! Per-window file session state.
//!
//! `FileSession` owns the loaded file metadata, the single authoritative file
//! operation, request/document generations, the forced-overwrite decision,
//! close/open intents that must survive a conflict, disk-watcher epoch and
//! in-flight state, `:wqa` participation and the test load hook. It does not own
//! the document, setting status messages, spawning background work or showing
//! prompts: adapters pass explicit values and `Editor` coordinates those steps.

use std::path::{Path, PathBuf};

use chaus_io::{FileMeta, SaveConflict};

use crate::editor_file::{FileState, LoadHook};
use crate::file_operation::{
    CloseAfter, DocumentGeneration, FileOperation, FileRequest, FileRequestId,
};

/// Everything tied to one file identity in one editor window.
pub(crate) struct FileSession {
    state: Option<FileState>,
    operation: FileOperation,
    request_next: u64,
    document_generation: DocumentGeneration,
    force_next_save: bool,
    pending_close_after: Option<CloseAfter>,
    pending_open_close: Option<CloseAfter>,
    disk_change: Option<SaveConflict>,
    disk_epoch: u64,
    disk_check_in_flight: bool,
    wqa_active: bool,
    wqa_failure: Option<String>,
    load_hook: Option<LoadHook>,
}

impl FileSession {
    pub(crate) fn new(state: Option<FileState>) -> Self {
        Self {
            state,
            operation: FileOperation::Idle,
            request_next: 0,
            document_generation: DocumentGeneration::default(),
            force_next_save: false,
            pending_close_after: None,
            pending_open_close: None,
            disk_change: None,
            disk_epoch: 0,
            disk_check_in_flight: false,
            wqa_active: false,
            wqa_failure: None,
            load_hook: None,
        }
    }

    // ---- file identity ------------------------------------------------------

    pub(crate) fn state(&self) -> Option<&FileState> {
        self.state.as_ref()
    }

    pub(crate) fn has_state(&self) -> bool {
        self.state.is_some()
    }

    pub(crate) fn meta(&self) -> Option<&FileMeta> {
        self.state.as_ref().map(|file| &file.meta)
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.meta().map(|meta| meta.user_requested_path.as_path())
    }

    pub(crate) fn set_state(&mut self, state: Option<FileState>) {
        self.state = state;
    }

    /// Save As suggestion. A recovered document has no live file, so the
    /// caller supplies the retained original path and BOM explicitly.
    pub(crate) fn save_as_source(
        &self,
        recovery_path: Option<PathBuf>,
        recovery_bom: bool,
    ) -> (Option<PathBuf>, bool) {
        let file = self.state.as_ref();
        (
            file.map(|file| file.meta.user_requested_path.clone())
                .or(recovery_path),
            file.map(|file| file.meta.had_utf8_bom)
                .unwrap_or(recovery_bom),
        )
    }

    pub(crate) fn bom(&self, fallback: bool) -> bool {
        self.meta().map_or(fallback, |meta| meta.had_utf8_bom)
    }

    // ---- operation ownership ------------------------------------------------

    /// Start a request that owns the file session until it settles. Rejecting
    /// a second request keeps a re-entrant picker from retargeting the editor.
    pub(crate) fn begin_request(&mut self, state_token: u64) -> Option<FileRequest> {
        if !self.operation.is_idle() {
            return None;
        }
        Some(FileRequest {
            id: FileRequestId::next(&mut self.request_next),
            document: self.document_generation,
            state: state_token,
        })
    }

    pub(crate) fn owns_request(&self, request: FileRequest) -> bool {
        self.operation.request() == Some(request)
    }

    pub(crate) fn operation_active(&self) -> bool {
        !self.operation.is_idle()
    }

    pub(crate) fn is_saving(&self) -> bool {
        matches!(self.operation, FileOperation::Saving { .. })
    }

    pub(crate) fn begin_open(&mut self, request: FileRequest, path: PathBuf, force: bool) {
        self.operation = FileOperation::Opening {
            request,
            path,
            force,
        };
    }

    pub(crate) fn opening_details(&self) -> Option<(bool, PathBuf)> {
        match &self.operation {
            FileOperation::Opening { force, path, .. } => Some((*force, path.clone())),
            _ => None,
        }
    }

    pub(crate) fn begin_save_selection(
        &mut self,
        request: FileRequest,
        close_after: CloseAfter,
        had_utf8_bom: bool,
    ) {
        self.operation = FileOperation::ChoosingSaveTarget {
            request,
            close_after,
            had_utf8_bom,
        };
    }

    pub(crate) fn save_selection_details(&self) -> Option<(CloseAfter, bool)> {
        match &self.operation {
            FileOperation::ChoosingSaveTarget {
                close_after,
                had_utf8_bom,
                ..
            } => Some((*close_after, *had_utf8_bom)),
            _ => None,
        }
    }

    pub(crate) fn begin_save(&mut self, request: FileRequest) {
        self.operation = FileOperation::Saving { request };
    }

    pub(crate) fn settle(&mut self) {
        self.operation = FileOperation::Idle;
    }

    /// Abandon an operation whose document was replaced elsewhere. Bumping the
    /// request counter ensures no late completion can own the next operation.
    pub(crate) fn cancel_operation(&mut self) {
        self.request_next = self.request_next.wrapping_add(1);
        self.settle();
    }

    // ---- generation and overwrite decision ---------------------------------

    pub(crate) fn generation(&self) -> DocumentGeneration {
        self.document_generation
    }

    pub(crate) fn owns_document(&self, generation: DocumentGeneration) -> bool {
        self.document_generation == generation
    }

    pub(crate) fn bump_generation(&mut self) {
        self.document_generation.bump();
    }

    pub(crate) fn clear_force(&mut self) {
        self.force_next_save = false;
    }

    pub(crate) fn take_force(&mut self) -> bool {
        std::mem::take(&mut self.force_next_save)
    }

    pub(crate) fn set_force(&mut self) {
        self.force_next_save = true;
    }

    // ---- pending intents ----------------------------------------------------

    pub(crate) fn set_close_intent(&mut self, intent: Option<CloseAfter>) {
        self.pending_close_after = intent;
    }

    pub(crate) fn take_close_intent(&mut self) -> Option<CloseAfter> {
        self.pending_close_after.take()
    }

    pub(crate) fn set_open_close_intent(&mut self, intent: Option<CloseAfter>) {
        self.pending_open_close = intent;
    }

    pub(crate) fn take_open_close_intent(&mut self) -> Option<CloseAfter> {
        self.pending_open_close.take()
    }

    // ---- disk watcher -------------------------------------------------------

    pub(crate) fn bump_epoch(&mut self) {
        self.disk_epoch = self.disk_epoch.wrapping_add(1);
    }

    pub(crate) fn owns_epoch(&self, epoch: u64) -> bool {
        self.disk_epoch == epoch
    }

    pub(crate) fn check_in_flight(&self) -> bool {
        self.disk_check_in_flight
    }

    /// Claim the single in-flight disk check. Returns `false` when one is
    /// already running, so a slow poll cannot stack behind a faster one.
    pub(crate) fn begin_check(&mut self) -> bool {
        if self.disk_check_in_flight {
            return false;
        }
        self.disk_check_in_flight = true;
        true
    }

    pub(crate) fn end_check(&mut self) {
        self.disk_check_in_flight = false;
    }

    pub(crate) fn change(&self) -> Option<SaveConflict> {
        self.disk_change
    }

    pub(crate) fn take_change(&mut self) -> Option<SaveConflict> {
        self.disk_change.take()
    }

    pub(crate) fn set_change(&mut self, change: Option<SaveConflict>) {
        self.disk_change = change;
    }

    /// Record a fresh disk conflict. Returns `false` when it equals the one
    /// already displayed, so a repeated poll does not flash the status again.
    pub(crate) fn record_change(&mut self, conflict: SaveConflict) -> bool {
        if self.disk_change == Some(conflict) {
            return false;
        }
        self.disk_change = Some(conflict);
        true
    }

    pub(crate) fn watch_snapshot(&self) -> Option<(FileMeta, u64)> {
        self.meta().map(|meta| (meta.clone(), self.disk_epoch))
    }

    // ---- :wqa participation -------------------------------------------------

    pub(crate) fn wqa_active(&self) -> bool {
        self.wqa_active
    }

    pub(crate) fn wqa_failure(&self) -> Option<&str> {
        self.wqa_failure.as_deref()
    }

    pub(crate) fn start_wqa(&mut self) {
        self.wqa_active = true;
        self.wqa_failure = None;
    }

    /// Keep the first failure: later failures are consequences of the same
    /// aborted command and must not replace the actionable reason.
    pub(crate) fn note_wqa_failure(&mut self, message: impl Into<String>) {
        if self.wqa_active && self.wqa_failure.is_none() {
            self.wqa_failure = Some(message.into());
        }
    }

    pub(crate) fn clear_wqa(&mut self) {
        self.wqa_active = false;
        self.wqa_failure = None;
    }

    // ---- test seam ----------------------------------------------------------

    pub(crate) fn set_load_hook(&mut self, hook: LoadHook) {
        self.load_hook = Some(hook);
    }

    pub(crate) fn load_hook(&self) -> Option<LoadHook> {
        self.load_hook
    }

    /// Reset every per-document field when a new document is installed. The
    /// generation is bumped by the caller after the document itself is reset.
    pub(crate) fn reset_for_document(&mut self, state: Option<FileState>) {
        self.state = state;
        self.force_next_save = false;
        self.disk_change = None;
        self.pending_close_after = None;
        self.pending_open_close = None;
        self.bump_epoch();
    }
}
