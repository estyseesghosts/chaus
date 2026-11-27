//! Mouse selection state for one editor window.
//!
//! `PointerState` owns the selecting flag, the last drag position and the
//! generation that retires a stationary-pointer autoscroll loop. It does not
//! own selection values, geometry, scrolling or the timer: adapters pass the
//! events and the editor performs hit testing and scroll changes.

use gpui::{Pixels, Point};

/// Drag and autoscroll bookkeeping.
pub(crate) struct PointerState {
    selecting: bool,
    drag_pointer: Option<Point<Pixels>>,
    drag_generation: u64,
    drag_autoscroll_active: bool,
}

impl Default for PointerState {
    fn default() -> Self {
        Self::new()
    }
}

impl PointerState {
    pub(crate) fn new() -> Self {
        Self {
            selecting: false,
            drag_pointer: None,
            drag_generation: 0,
            drag_autoscroll_active: false,
        }
    }

    pub(crate) fn selecting(&self) -> bool {
        self.selecting
    }

    pub(crate) fn pointer(&self) -> Option<Point<Pixels>> {
        self.drag_pointer
    }

    pub(crate) fn autoscroll_active(&self) -> bool {
        self.drag_autoscroll_active
    }

    /// Start a new drag at `position` and retire any older autoscroll loop.
    pub(crate) fn begin_drag(&mut self, position: Point<Pixels>) {
        self.selecting = true;
        self.drag_generation = self.drag_generation.wrapping_add(1);
        self.drag_pointer = Some(position);
        self.drag_autoscroll_active = false;
    }

    pub(crate) fn set_pointer(&mut self, position: Point<Pixels>) {
        self.drag_pointer = Some(position);
    }

    pub(crate) fn end_drag(&mut self) {
        self.selecting = false;
        self.drag_pointer = None;
        self.drag_autoscroll_active = false;
    }

    pub(crate) fn stop_selecting(&mut self) {
        self.selecting = false;
    }

    /// Start an autoscroll loop for the current drag. Returns its generation,
    /// or `None` when one is already running.
    pub(crate) fn begin_autoscroll(&mut self) -> Option<u64> {
        if self.drag_autoscroll_active {
            return None;
        }
        self.drag_autoscroll_active = true;
        Some(self.drag_generation)
    }

    /// End an autoscroll loop, but only if `generation` still owns it.
    pub(crate) fn end_autoscroll(&mut self, generation: u64) {
        if self.drag_generation == generation {
            self.drag_autoscroll_active = false;
        }
    }

    pub(crate) fn is_current_drag(&self, generation: u64) -> bool {
        self.drag_generation == generation && self.selecting
    }
}
