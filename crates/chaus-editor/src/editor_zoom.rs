//! Per-window document font zoom.

use gpui::Context;

use crate::editor::Editor;
use crate::editor_keys::{ZoomIn, ZoomOut, ZoomReset};

/// Documented discrete zoom scale in logical pixels.
const ZOOM_STOPS: [f32; 25] = [
    9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0,
    26.0, 28.0, 30.0, 32.0, 34.0, 36.0, 40.0, 44.0, 48.0,
];

const MIN_ZOOM: f32 = ZOOM_STOPS[0];
const MAX_ZOOM: f32 = ZOOM_STOPS[ZOOM_STOPS.len() - 1];

impl Editor {
    pub(crate) fn zoom_in_action(&mut self, _: &ZoomIn, cx: &mut Context<Self>) {
        self.zoom_in(cx);
    }

    pub(crate) fn zoom_out_action(&mut self, _: &ZoomOut, cx: &mut Context<Self>) {
        self.zoom_out(cx);
    }

    pub(crate) fn zoom_reset_action(&mut self, _: &ZoomReset, cx: &mut Context<Self>) {
        self.reset_zoom(cx);
    }

    /// Increase this window's document font size by one documented step.
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        let next = next_zoom_in(self.editor_font_size());
        self.apply_zoom(next, cx);
    }

    /// Decrease this window's document font size by one documented step.
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let next = next_zoom_out(self.editor_font_size());
        self.apply_zoom(next, cx);
    }

    /// Return this window to the configured editor font size.
    pub fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.viewport.set_zoom(None);
        self.after_zoom(cx);
        self.flash_status(
            format!("Editor font {:.0}px (default)", self.editor_font_size()),
            cx,
        );
    }

    fn apply_zoom(&mut self, size: f32, cx: &mut Context<Self>) {
        self.viewport.set_zoom(Some(size.clamp(MIN_ZOOM, MAX_ZOOM)));
        self.after_zoom(cx);
        self.flash_status(format!("Editor font {:.0}px", self.editor_font_size()), cx);
    }

    fn after_zoom(&mut self, cx: &mut Context<Self>) {
        // The next paint recomputes wrap columns from the new glyph advance.
        // Rebuild the line-row cache against the old columns first so row
        // totals and scroll clamping stay consistent in the meantime.
        self.rebuild_rows();
        self.clamp_scroll();
        self.scroll_to_caret();
        self.show_caret();
        cx.notify();
    }
}

/// Next larger stop. A configured default between stops steps to the next
/// higher one rather than jumping to an unrelated preset.
#[must_use]
pub(crate) fn next_zoom_in(current: f32) -> f32 {
    ZOOM_STOPS
        .iter()
        .copied()
        .find(|stop| *stop > current + f32::EPSILON)
        .unwrap_or(MAX_ZOOM)
}

/// Next smaller stop, clamped at the documented minimum.
#[must_use]
pub(crate) fn next_zoom_out(current: f32) -> f32 {
    ZOOM_STOPS
        .iter()
        .rev()
        .copied()
        .find(|stop| *stop < current - f32::EPSILON)
        .unwrap_or(MIN_ZOOM)
}
