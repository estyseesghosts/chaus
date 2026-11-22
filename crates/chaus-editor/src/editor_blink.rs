//! Caret blinking.
//!
//! The caret is visible immediately after any selection change and then toggles
//! on a fixed interval. A generation counter retires the previous timer, so
//! typing quickly cannot accumulate one loop per keystroke. Normal mode in Vim
//! keeps the caret steady, matching the block-cursor convention.

use std::time::Duration;

use gpui::Context;

use crate::editor::Editor;

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

impl Editor {
    /// Whether the caret should blink in the current mode.
    pub(crate) fn caret_blinks(&self) -> bool {
        !self.vim || self.vim_engine.mode().accepts_text()
    }

    /// Restart blinking when the caret position or mode changed.
    pub(crate) fn sync_blink(&mut self, cx: &mut Context<Self>) {
        let key = (self.selection, self.caret_blinks());
        if self.blink_key == Some(key) {
            return;
        }
        self.blink_key = Some(key);
        self.caret_visible = true;
        self.blink_epoch = self.blink_epoch.wrapping_add(1);
        let epoch = self.blink_epoch;
        if !key.1 {
            return;
        }

        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(BLINK_INTERVAL).await;
            let keep_going = editor.update(cx, |editor, cx| {
                if editor.blink_epoch != epoch {
                    return false;
                }
                editor.caret_visible = !editor.caret_visible;
                cx.notify();
                true
            });
            if !matches!(keep_going, Ok(true)) {
                break;
            }
        })
        .detach();
    }
}
