//! Undo/redo bookkeeping for one document.
//!
//! [`History`] owns replay steps, state identities and grouping metadata. It
//! never touches the Rope: `Document` takes an entry out, replays its steps and
//! hands the entry back to the opposite stack. That keeps all text mutation in
//! the document's single transaction path.

use std::time::{Duration, Instant};

use crate::edit::{EditCause, TextEdit};
use crate::eol::Eol;
use crate::position::Selection;

/// How long contiguous typing keeps coalescing into one undo entry.
const TYPING_COALESCE_WINDOW: Duration = Duration::from_millis(750);

#[derive(Debug)]
pub(crate) struct UndoStep {
    pub(crate) edit: TextEdit,
    pub(crate) removed: String,
    pub(crate) eol: EolOp,
}

#[derive(Debug)]
pub(crate) struct EolOp {
    pub(crate) break_index: usize,
    pub(crate) removed: Vec<Eol>,
    pub(crate) inserted: Vec<Eol>,
}

#[derive(Debug)]
pub(crate) struct HistoryEntry {
    /// Steps in the order they were applied (descending char offsets within
    /// each transaction, transactions in chronological order).
    pub(crate) steps: Vec<UndoStep>,
    pub(crate) before_selection: Selection,
    pub(crate) after_selection: Selection,
    pub(crate) cause: EditCause,
    pub(crate) time: Instant,
    /// Identity of the history state before and after this entry. State
    /// identities are unique, unlike undo-stack depths, so branching and
    /// coalescing cannot make a different state look saved.
    pub(crate) before_state: u64,
    pub(crate) after_state: u64,
}

/// All undo/redo state for a document, independent of its text storage.
#[derive(Debug)]
pub(crate) struct History {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    current_state: u64,
    saved_state: u64,
    next_state: u64,
    group_depth: usize,
    group_start_depth: usize,
}

impl History {
    pub(crate) fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_state: 0,
            saved_state: 0,
            next_state: 1,
            group_depth: 0,
            group_start_depth: 0,
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub(crate) fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub(crate) fn current_state(&self) -> u64 {
        self.current_state
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.current_state != self.saved_state
    }

    pub(crate) fn fresh_state(&mut self) -> u64 {
        let state = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);
        state
    }

    pub(crate) fn set_current_state(&mut self, state: u64) {
        self.current_state = state;
    }

    pub(crate) fn mark_saved(&mut self) {
        self.saved_state = self.current_state;
    }

    pub(crate) fn mark_saved_at(&mut self, state: u64) {
        self.saved_state = state;
    }

    pub(crate) fn mark_unsaved(&mut self) {
        self.saved_state = self.fresh_state();
    }

    /// Whether transactions are being merged into the most recent entry by an
    /// open user operation.
    pub(crate) fn is_grouped(&self) -> bool {
        self.group_depth > 0 && self.undo_stack.len() > self.group_start_depth
    }

    /// Whether contiguous typing should extend the most recent undo entry.
    pub(crate) fn can_coalesce(
        &self,
        cause: EditCause,
        edits: &[TextEdit],
        before: Selection,
    ) -> bool {
        if cause != EditCause::Typing || edits.len() != 1 {
            return false;
        }
        let edit = &edits[0];
        if edit.insert.contains('\n') || !edit.range.is_empty() {
            return false;
        }
        let Some(last) = self.undo_stack.last() else {
            return false;
        };
        last.cause == EditCause::Typing
            && last.after_state != self.saved_state
            && last.after_selection.is_empty()
            && last.after_selection.head == edit.range.start
            && last.after_selection == before
            && last.time.elapsed() < TYPING_COALESCE_WINDOW
    }

    /// Move a completed transaction into the undo stack, clearing the redo
    /// branch and merging it into the last entry when grouping or typing
    /// coalescing applies.
    pub(crate) fn record(&mut self, entry: HistoryEntry, grouped: bool, contiguous_typing: bool) {
        let after_state = entry.after_state;
        self.redo_stack.clear();
        let append_to_last = (grouped || contiguous_typing)
            && self
                .undo_stack
                .last()
                .is_some_and(|last| last.after_state != self.saved_state);
        if append_to_last {
            if let Some(last) = self.undo_stack.last_mut() {
                last.steps.extend(entry.steps);
                last.after_selection = entry.after_selection;
                last.time = entry.time;
                last.after_state = after_state;
            }
        } else {
            self.undo_stack.push(entry);
        }
        self.current_state = after_state;
    }

    /// Hand an entry to `Document` for inverse replay. The entry must be
    /// returned with [`Self::push_redo`] after the Rope has been patched.
    pub(crate) fn take_undo(&mut self) -> Option<HistoryEntry> {
        self.undo_stack.pop()
    }

    /// Hand an entry to `Document` for forward replay. The entry must be
    /// returned with [`Self::push_undo`] after the Rope has been patched.
    pub(crate) fn take_redo(&mut self) -> Option<HistoryEntry> {
        self.redo_stack.pop()
    }

    pub(crate) fn push_undo(&mut self, entry: HistoryEntry) {
        self.undo_stack.push(entry);
    }

    pub(crate) fn push_redo(&mut self, entry: HistoryEntry) {
        self.redo_stack.push(entry);
    }

    pub(crate) fn begin_user_operation(&mut self) {
        if self.group_depth == 0 {
            self.group_start_depth = self.undo_stack.len();
        }
        self.group_depth += 1;
    }

    pub(crate) fn end_user_operation(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.group_start_depth = 0;
        }
    }

    pub(crate) fn cancel_user_operation(&mut self) {
        self.group_depth = 0;
        self.group_start_depth = 0;
    }

    /// Clear history and mark a freshly loaded document clean at a new state.
    pub(crate) fn reset(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        let state = self.fresh_state();
        self.current_state = state;
        self.saved_state = state;
        self.group_depth = 0;
        self.group_start_depth = 0;
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}
