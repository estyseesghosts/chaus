//! Unsaved-changes sheet shown when a dirty window is closed.

use gpui::{
    div, px, App, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
};
use silvestris::controls::Button;
use silvestris::ActiveTheme;

use crate::editor::Editor;
use crate::file_operation::CloseAfter;

/// Focus handles for the three choices. They live on the editor so the sheet
/// can hand keyboard focus to one of them while it is open.
#[derive(Clone)]
pub(crate) struct ClosePromptFocus {
    pub(crate) save: FocusHandle,
    pub(crate) discard: FocusHandle,
    pub(crate) cancel: FocusHandle,
}

impl ClosePromptFocus {
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            save: cx.focus_handle(),
            discard: cx.focus_handle(),
            cancel: cx.focus_handle(),
        }
    }
}

impl Editor {
    /// Ask to close the window. A clean document closes immediately; a dirty
    /// one shows the Save / Discard / Cancel sheet and keeps the window open
    /// until the person chooses.
    pub fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.file_operation_active() {
            self.set_status("A file operation is in progress");
            cx.notify();
            return false;
        }
        if !self.document.is_dirty() {
            self.pending_close_after = None;
            self.remove_recovery_snapshot(cx);
            return true;
        }
        self.close_prompt = true;
        self.pending_close_after = None;
        self.vim_engine.cancel_prefix();
        window.focus(&self.close_focus.save, cx);
        cx.notify();
        false
    }

    pub(crate) fn cancel_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt = false;
        self.pending_close_after = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Re-show the dirty-close sheet after a cancelled or failed save. The
    /// window stays open and keyboard focus returns to a reachable button.
    pub(crate) fn restore_close_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt = true;
        self.vim_engine.cancel_prefix();
        window.focus(&self.close_focus.save, cx);
        cx.notify();
    }

    pub(crate) fn discard_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt = false;
        self.pending_close_after = None;
        self.pending_open_close = None;
        self.remove_recovery_snapshot(cx);
        window.remove_window();
    }

    pub(crate) fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_prompt = false;
        let started = if self.file.is_some() {
            self.save_inner(CloseAfter::Sheet, cx)
        } else {
            self.save_as_inner(CloseAfter::Sheet, window, cx)
        };
        if !started {
            self.restore_close_prompt(window, cx);
        }
    }

    pub(crate) fn close_prompt_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_close(window, cx),
            "tab" => {
                let reverse = event.keystroke.modifiers.shift;
                let next = if self.close_focus.save.is_focused(window) {
                    if reverse {
                        &self.close_focus.cancel
                    } else {
                        &self.close_focus.discard
                    }
                } else if self.close_focus.discard.is_focused(window) {
                    if reverse {
                        &self.close_focus.save
                    } else {
                        &self.close_focus.cancel
                    }
                } else if reverse {
                    &self.close_focus.discard
                } else {
                    &self.close_focus.save
                };
                window.focus(next, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_close_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div()
            .id("chaus-close-sheet")
            .debug_selector(|| "chaus-close-sheet".to_string())
            .role(gpui::Role::AlertDialog)
            .aria_label("Unsaved changes")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(58.0))
            .bg(silvestris::convert::hsla(theme.colors.scrim))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|editor, event: &KeyDownEvent, window, cx| {
                cx.stop_propagation();
                editor.close_prompt_key(event, window, cx);
            }))
            .child(
                div()
                    .w(px(400.0))
                    .max_w_full()
                    .p(px(18.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(silvestris::convert::hsla(theme.colors.group_border))
                    .bg(silvestris::convert::hsla(theme.colors.window))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Save changes before closing?"),
                    )
                    .child(
                        div()
                            .text_size(px(theme.type_.size_small))
                            .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                            .child("Your changes will be lost if you discard them."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new("close-cancel", "Cancel")
                                    .focus_handle(self.close_focus.cancel.clone())
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.cancel_close(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("close-discard", "Discard")
                                    .focus_handle(self.close_focus.discard.clone())
                                    .destructive(true)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.discard_and_close(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("close-save", "Save")
                                    .focus_handle(self.close_focus.save.clone())
                                    .default_button(true)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.save_and_close(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
