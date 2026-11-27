//! Caret blink lifecycle for one editor window.
//!
//! `BlinkState` owns whether the caret is drawn and which selection/mode key
//! the current blink loop describes. It does not spawn timers itself: the
//! adapter starts a loop with the returned epoch and calls back for each tick.

use chaus_core::position::Selection;

/// What the current blink loop represents: caret position and whether blinking
/// applies in the active editing mode.
pub(crate) type BlinkKey = (Selection, bool);

/// Caret visibility, blink epoch and the key of the current loop.
pub(crate) struct BlinkState {
    caret_visible: bool,
    epoch: u64,
    key: Option<BlinkKey>,
}

impl Default for BlinkState {
    fn default() -> Self {
        Self::new()
    }
}

impl BlinkState {
    pub(crate) fn new() -> Self {
        Self {
            caret_visible: true,
            epoch: 0,
            key: None,
        }
    }

    pub(crate) fn visible(&self) -> bool {
        self.caret_visible
    }

    /// Whether a new loop must replace the current one.
    pub(crate) fn changed(&self, key: BlinkKey) -> bool {
        self.key != Some(key)
    }

    /// Start a new loop for `key`. Returns its epoch so a tick can check that
    /// it still belongs to the current caret and mode.
    pub(crate) fn restart(&mut self, key: BlinkKey) -> u64 {
        self.key = Some(key);
        self.caret_visible = true;
        self.epoch = self.epoch.wrapping_add(1);
        self.epoch
    }

    pub(crate) fn is_current(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    /// Toggle the caret for a tick that still owns this state.
    pub(crate) fn toggle(&mut self, epoch: u64) -> bool {
        if !self.is_current(epoch) {
            return false;
        }
        self.caret_visible = !self.caret_visible;
        true
    }

    /// Keep the caret steady, e.g. after a zoom change that invalidates the
    /// previous blink loop.
    pub(crate) fn show(&mut self) {
        self.caret_visible = true;
    }
}
