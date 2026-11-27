//! Platform text-input bookkeeping for one editor window.
//!
//! `TextInputState` owns the IME marked range, the start of the active preedit,
//! and the flag that drops the Windows `WM_CHAR` following a consumed key-down.
//! It does not own the selection, the document, or the composition edits: the
//! platform adapter supplies those and `Editor` routes transactions through
//! the document.

use chaus_core::position::{CharOffset, CharRange};

/// IME composition and single-character suppression state.
pub(crate) struct TextInputState {
    marked: Option<CharRange>,
    preedit_start: Option<CharOffset>,
    preedit_active: bool,
    suppress_text_input: bool,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInputState {
    pub(crate) fn new() -> Self {
        Self {
            marked: None,
            preedit_start: None,
            preedit_active: false,
            suppress_text_input: false,
        }
    }

    pub(crate) fn marked(&self) -> Option<CharRange> {
        self.marked
    }

    pub(crate) fn preedit_active(&self) -> bool {
        self.preedit_active
    }

    pub(crate) fn preedit_start(&self) -> Option<CharOffset> {
        self.preedit_start
    }

    pub(crate) fn composition_active(&self) -> bool {
        self.marked.is_some() || self.preedit_active
    }

    /// Clear the fields that describe a finished or abandoned composition.
    pub(crate) fn clear_composition(&mut self) {
        self.marked = None;
        self.preedit_start = None;
        self.preedit_active = false;
    }

    /// Drop the pending character message from a key-down the modal engine
    /// consumed. A new key-down clears the flag even when it is not consumed.
    pub(crate) fn clear_suppression(&mut self) {
        self.suppress_text_input = false;
    }

    pub(crate) fn suppress_next(&mut self) {
        self.suppress_text_input = true;
    }

    /// Take the suppression flag: it applies to exactly one text callback.
    pub(crate) fn take_suppression(&mut self) -> bool {
        let suppressed = self.suppress_text_input;
        self.suppress_text_input = false;
        suppressed
    }

    /// Install a new preedit after its edit was applied.
    pub(crate) fn set_preedit(&mut self, start: CharOffset, marked: CharRange) {
        self.preedit_start = Some(start);
        self.preedit_active = true;
        self.marked = Some(marked);
    }
}
