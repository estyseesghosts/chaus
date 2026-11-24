//! Explicit choice sheet for a save conflict.

use gpui::{
    div, px, App, Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
};
use silvestris::controls::Button;
use silvestris::ActiveTheme;

use crate::editor::Editor;
use crate::file_operation::CloseAfter;

#[derive(Clone)]
pub(crate) struct ConflictPromptFocus {
    pub(crate) reload: FocusHandle,
    pub(crate) save_as: FocusHandle,
    pub(crate) overwrite: FocusHandle,
    pub(crate) cancel: FocusHandle,
}

impl ConflictPromptFocus {
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            reload: cx.focus_handle(),
            save_as: cx.focus_handle(),
            overwrite: cx.focus_handle(),
            cancel: cx.focus_handle(),
        }
    }
}

impl Editor {
    pub(crate) fn show_conflict_prompt(
        &mut self,
        conflict: chaus_io::SaveConflict,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.conflict_prompt = Some(conflict);
        self.vim_engine.cancel_prefix();
        window.focus(&self.conflict_focus.cancel, cx);
        cx.notify();
    }

    pub(crate) fn cancel_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conflict_prompt = None;
        if self.close_prompt {
            window.focus(&self.close_focus.save, cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn reload_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conflict_prompt = None;
        let Some(path) = self
            .file
            .as_ref()
            .map(|file| file.meta.user_requested_path.clone())
        else {
            self.cancel_conflict(window, cx);
            return;
        };
        if !self.open_path(&path, true, cx) {
            window.focus(&self.focus, cx);
        }
    }

    pub(crate) fn save_as_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conflict_prompt = None;
        let _ = self.save_as_inner(CloseAfter::No, window, cx);
    }

    pub(crate) fn overwrite_conflict(&mut self, cx: &mut Context<Self>) {
        self.conflict_prompt = None;
        self.force_next_save = true;
        self.save_inner(CloseAfter::No, cx);
    }

    pub(crate) fn conflict_prompt_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_conflict(window, cx),
            "tab" => {
                cx.stop_propagation();
                let reverse = event.keystroke.modifiers.shift;
                let order = [
                    &self.conflict_focus.cancel,
                    &self.conflict_focus.reload,
                    &self.conflict_focus.save_as,
                    &self.conflict_focus.overwrite,
                ];
                let current = order
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(0);
                let next = if reverse {
                    (current + order.len() - 1) % order.len()
                } else {
                    (current + 1) % order.len()
                };
                window.focus(order[next], cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_conflict_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let message = self
            .conflict_prompt
            .map(|conflict| format!("{conflict}. Choose how to continue."))
            .unwrap_or_default();
        div()
            .id("chaus-conflict-sheet")
            .debug_selector(|| "chaus-conflict-sheet".to_string())
            .role(gpui::Role::AlertDialog)
            .aria_label("Save conflict")
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
                editor.conflict_prompt_key(event, window, cx);
            }))
            .child(
                div()
                    .w(px(460.0))
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
                            .child("The file changed on disk"),
                    )
                    .child(
                        div()
                            .text_size(px(theme.type_.size_small))
                            .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                            .child(message),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new("conflict-cancel", "Cancel")
                                    .focus_handle(self.conflict_focus.cancel.clone())
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.cancel_conflict(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("conflict-reload", "Reload")
                                    .focus_handle(self.conflict_focus.reload.clone())
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.reload_conflict(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("conflict-save-as", "Save As")
                                    .focus_handle(self.conflict_focus.save_as.clone())
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.save_as_conflict(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("conflict-overwrite", "Overwrite")
                                    .focus_handle(self.conflict_focus.overwrite.clone())
                                    .destructive(true)
                                    .on_click(cx.listener(|editor, _, _, cx| {
                                        editor.overwrite_conflict(cx);
                                    })),
                            ),
                    ),
            )
    }
}
