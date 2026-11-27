//! Caret blinking.
//!
//! The caret is visible immediately after any selection change and then toggles
//! on a fixed interval. A generation counter retires the previous timer, so
//! typing quickly cannot accumulate one loop per keystroke. Normal mode in Vim
//! keeps the caret steady, matching the block-cursor convention.
//!
//! `BlinkState` owns the visibility, epoch and key; this adapter starts the
//! timer and calls back on each tick.

use std::time::Duration;

use gpui::Context;

use crate::editor::Editor;

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

impl Editor {
    /// Whether the caret should blink in the current mode.
    pub(crate) fn caret_blinks(&self) -> bool {
        !self.vim.enabled() || self.vim.mode().accepts_text()
    }

    /// Restart blinking when the caret position or mode changed.
    pub(crate) fn sync_blink(&mut self, cx: &mut Context<Self>) {
        let blink = (self.selection, self.caret_blinks());
        if !self.blink.changed(blink) {
            return;
        }
        let epoch = self.blink.restart(blink);
        if !blink.1 {
            return;
        }

        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(BLINK_INTERVAL).await;
            let keep_going = editor.update(cx, |editor, cx| {
                if !editor.blink.toggle(epoch) {
                    return false;
                }
                cx.notify();
                true
            });
            if !matches!(keep_going, Ok(true)) {
                break;
            }
        })
        .detach();
    }

    /// Keep the caret visible after a zoom change invalidates the old loop.
    pub(crate) fn show_caret(&mut self) {
        self.blink.show();
    }
}
