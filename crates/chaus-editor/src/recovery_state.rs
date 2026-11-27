//! Crash-recovery state for one editor window.
//!
//! `RecoveryState` owns the launch-time recovery queue, the sheet's focus
//! handles and keyboard selection, the snapshot key/epoch, window identity of
//! the key interceptor, and the retained path/BOM of a recovered document. It
//! does not own the document, the filesystem, or status messages: adapters
//! spawn and report background work, and `Editor` coordinates document changes.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use chaus_io::recovery::{RecoveryEntry, RecoveryRecord};
use gpui::{App, FocusHandle, Subscription, WindowId};

use crate::editor_recovery::{new_recovery_id, RecoveryPromptFocus};

/// All per-window recovery bookkeeping.
pub(crate) struct RecoveryState {
    queue: VecDeque<RecoveryEntry>,
    focus: RecoveryPromptFocus,
    key: u64,
    epoch: u64,
    selection: usize,
    interceptor: Option<Subscription>,
    window: Option<WindowId>,
    original_path: Option<PathBuf>,
    had_utf8_bom: bool,
    directory: Option<PathBuf>,
    path: Option<PathBuf>,
}

impl RecoveryState {
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            queue: VecDeque::new(),
            focus: RecoveryPromptFocus::new(cx),
            key: new_recovery_id(),
            epoch: 0,
            selection: 0,
            interceptor: None,
            window: None,
            original_path: None,
            had_utf8_bom: false,
            directory: None,
            path: None,
        }
    }

    // ---- sheet --------------------------------------------------------------

    pub(crate) fn is_open(&self) -> bool {
        !self.queue.is_empty()
    }

    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(crate) fn front(&self) -> Option<&RecoveryEntry> {
        self.queue.front()
    }

    pub(crate) fn focus(&self) -> &RecoveryPromptFocus {
        &self.focus
    }

    pub(crate) fn recover_focus(&self) -> FocusHandle {
        self.focus.recover.clone()
    }

    /// Replace the queue for a launch-time scan and bind its interceptor to
    /// the window that received the records.
    pub(crate) fn queue_records(&mut self, records: Vec<RecoveryEntry>, window: WindowId) {
        self.queue = records.into();
        self.selection = 0;
        self.window = Some(window);
    }

    pub(crate) fn pop_front(&mut self) -> Option<RecoveryEntry> {
        self.queue.pop_front()
    }

    pub(crate) fn push_front(&mut self, entry: RecoveryEntry) {
        self.queue.push_front(entry);
    }

    pub(crate) fn drain(&mut self) -> Vec<RecoveryEntry> {
        self.queue.drain(..).collect()
    }

    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }

    pub(crate) fn selection(&self) -> usize {
        self.selection
    }

    pub(crate) fn cycle_selection(&mut self, reverse: bool) {
        self.selection = if reverse {
            (self.selection + 2) % 3
        } else {
            (self.selection + 1) % 3
        };
    }

    pub(crate) fn window_matches(&self, window: WindowId) -> bool {
        self.window == Some(window)
    }

    pub(crate) fn has_interceptor(&self) -> bool {
        self.interceptor.is_some()
    }

    pub(crate) fn set_interceptor(&mut self, interceptor: Subscription) {
        self.interceptor = Some(interceptor);
    }

    // ---- snapshot identity --------------------------------------------------

    pub(crate) fn key(&self) -> u64 {
        self.key
    }

    /// Invalidate writes already in flight. The returned epoch is the
    /// watermark for retirement and the new snapshot.
    pub(crate) fn bump_epoch(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    pub(crate) fn owns_epoch(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    pub(crate) fn retire_values(&self) -> (u64, Option<PathBuf>) {
        (self.key, self.path.clone())
    }

    /// Values a background snapshot needs. A live file wins over the retained
    /// metadata of a recovered document.
    pub(crate) fn snapshot_source(
        &self,
        file_path: Option<&Path>,
        file_bom: Option<bool>,
    ) -> (u64, Option<PathBuf>, bool, Option<PathBuf>) {
        (
            self.key,
            file_path
                .map(Path::to_path_buf)
                .or_else(|| self.original_path.clone()),
            file_bom.unwrap_or(self.had_utf8_bom),
            self.directory.clone(),
        )
    }

    pub(crate) fn metadata(&self) -> (Option<PathBuf>, bool) {
        (self.original_path.clone(), self.had_utf8_bom)
    }

    /// Adopt the record that was read: its id becomes this window's snapshot
    /// key and the entry's path pins the recovery store for later writes.
    pub(crate) fn adopt_record(&mut self, record: &RecoveryRecord, entry: &RecoveryEntry) {
        self.key = record.id;
        self.bump_epoch();
        self.original_path = record.original_path.clone();
        self.had_utf8_bom = record.had_utf8_bom;
        self.path = Some(entry.path.clone());
        self.directory = entry.path.parent().map(Path::to_path_buf);
    }

    /// Retire the adopted `.rec` and start a new identity after a successful
    /// associate-save. Returns the values needed for retirement.
    pub(crate) fn finish_after_save(&mut self) -> Option<(u64, Option<PathBuf>, u64)> {
        if self.path.is_none() && self.original_path.is_none() {
            return None;
        }
        let old_key = self.key;
        let old_path = self.path.take();
        let epoch = self.bump_epoch();
        self.key = new_recovery_id();
        self.original_path = None;
        self.had_utf8_bom = false;
        self.directory = None;
        Some((old_key, old_path, epoch))
    }

    /// Reset per-document identity when a document is replaced.
    pub(crate) fn reset_for_document(&mut self) {
        self.bump_epoch();
        self.key = new_recovery_id();
        self.original_path = None;
        self.had_utf8_bom = false;
        self.directory = None;
        self.path = None;
    }
}
