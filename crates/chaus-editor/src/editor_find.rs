//! In-window find and replace bar.

use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, Selection};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, AnyElement, App, AppContext as _, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Styled as _,
    Subscription, Window,
};
use silvestris::controls::{Button, ControlSize, TextField, TextFieldEvent};
use silvestris::ActiveTheme;

use crate::editor::Editor;

pub(crate) struct FindBar {
    pub(crate) find: Entity<TextField>,
    pub(crate) replace: Entity<TextField>,
    pub(crate) case_sensitive: bool,
    pub(crate) show_replace: bool,
    _subscriptions: Vec<Subscription>,
}

impl Editor {
    /// Open the find bar, optionally with the replace row visible, and focus
    /// the search field.
    pub fn open_find(&mut self, show_replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.vim_engine.cancel_prefix();
        if self.find_bar.is_none() {
            let find = cx.new(|cx| {
                let mut field = TextField::new(window, cx);
                field.set_search(true, cx);
                field.set_placeholder("Find", cx);
                field.set_label("Find", cx);
                field
            });
            let replace = cx.new(|cx| {
                let mut field = TextField::new(window, cx);
                field.set_placeholder("Replace", cx);
                field.set_label("Replace", cx);
                field
            });
            let find_subscription = cx.subscribe(&find, |editor, _, event, cx| {
                if matches!(event, TextFieldEvent::Submitted) {
                    editor.find_next(true, cx);
                }
            });
            let replace_subscription = cx.subscribe(&replace, |editor, _, event, cx| {
                if matches!(event, TextFieldEvent::Submitted) {
                    editor.replace_one(cx);
                }
            });
            self.find_bar = Some(FindBar {
                find,
                replace,
                case_sensitive: false,
                show_replace,
                _subscriptions: vec![find_subscription, replace_subscription],
            });
        } else if let Some(bar) = self.find_bar.as_mut() {
            bar.show_replace |= show_replace;
        }
        let focus = self
            .find_bar
            .as_ref()
            .map(|bar| bar.find.read(cx).focus_handle(cx));
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_bar = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn find_next(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some((needle, case_sensitive)) = self.find_query(cx) else {
            self.set_status("Find: enter some text");
            cx.notify();
            return;
        };
        let range = self.selection.range();
        let from = if forward { range.end } else { range.start };
        let Some(start) = self
            .document
            .find_literal(&needle, from, forward, case_sensitive)
        else {
            self.set_status("No matches");
            cx.notify();
            return;
        };
        let end = CharOffset(start.0 + needle.chars().count());
        self.selection = Selection::new(start, end);
        self.preferred_column = None;
        self.scroll_to_caret();
        self.set_status(format!("Match at {}", self.describe_offset(start)));
        cx.notify();
    }

    pub(crate) fn toggle_find_case(&mut self, cx: &mut Context<Self>) {
        if let Some(bar) = self.find_bar.as_mut() {
            bar.case_sensitive = !bar.case_sensitive;
            let label = if bar.case_sensitive {
                "Match case on"
            } else {
                "Match case off"
            };
            self.set_status(label);
            cx.notify();
        }
    }

    pub(crate) fn replace_one(&mut self, cx: &mut Context<Self>) {
        let Some((needle, case_sensitive)) = self.find_query(cx) else {
            self.set_status("Replace: enter search text");
            cx.notify();
            return;
        };
        let replacement = self
            .find_bar
            .as_ref()
            .map(|bar| bar.replace.read(cx).text().to_string())
            .unwrap_or_default();
        let from = self.selection.range().end;
        let Some(start) = self
            .document
            .find_literal(&needle, from, true, case_sensitive)
        else {
            self.set_status("No matches");
            cx.notify();
            return;
        };
        let range = CharRange::new(start, CharOffset(start.0 + needle.chars().count()));
        let before = self.selection;
        let after = Selection::caret(CharOffset(start.0 + replacement.chars().count()));
        let transaction = EditTransaction::new(
            EditCause::ReplaceAll,
            TextEdit::new(range, replacement),
            before,
            after,
        );
        match self.document.apply(transaction) {
            Ok(result) => {
                self.selection = after.clamp(self.document.len_chars());
                self.after_change(result.affected_lines, cx);
                self.find_next(true, cx);
                self.set_status(format!("Replaced match at {}", self.describe_offset(start)));
            }
            Err(error) => self.set_status(format!("Replace failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some((needle, case_sensitive)) = self.find_query(cx) else {
            self.set_status("Replace: enter search text");
            cx.notify();
            return;
        };
        let replacement = self
            .find_bar
            .as_ref()
            .map(|bar| bar.replace.read(cx).text().to_string())
            .unwrap_or_default();
        let replaced = self.replace_all_text(&needle, &replacement, case_sensitive, cx);
        if replaced == 0 {
            self.set_status("No matches");
        } else {
            self.set_status(format!("Replaced {replaced} occurrences"));
        }
        cx.notify();
    }

    /// Replace every literal match as one undoable transaction. Returns the
    /// number of replacements; an empty needle changes nothing.
    pub fn replace_all_text(
        &mut self,
        needle: &str,
        replacement: &str,
        case_sensitive: bool,
        cx: &mut Context<Self>,
    ) -> usize {
        let ranges = self.document.literal_ranges(needle, case_sensitive);
        if ranges.is_empty() {
            return 0;
        }
        let before = self.selection;
        let after = Selection::caret(CharOffset(ranges[0].start.0 + replacement.chars().count()));
        let edits = ranges
            .iter()
            .map(|range| TextEdit::new(*range, replacement.to_string()))
            .collect();
        let transaction = EditTransaction {
            edits,
            before_selection: before,
            after_selection: after,
            cause: EditCause::ReplaceAll,
        };
        match self.document.apply(transaction) {
            Ok(result) => {
                self.selection = after.clamp(self.document.len_chars());
                self.after_change(result.affected_lines, cx);
                cx.notify();
                ranges.len()
            }
            Err(error) => {
                self.set_status(format!("Replace failed: {error}"));
                cx.notify();
                0
            }
        }
    }

    pub(crate) fn find_bar_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.close_find(window, cx);
            }
            "tab" => {
                cx.stop_propagation();
                let Some(bar) = self.find_bar.as_ref() else {
                    return;
                };
                let find_focused = bar.find.read(cx).focus_handle(cx).is_focused(window);
                let target = if find_focused {
                    bar.replace.read(cx).focus_handle(cx)
                } else {
                    bar.find.read(cx).focus_handle(cx)
                };
                window.focus(&target, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_find_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(bar) = self.find_bar.as_ref() else {
            return div().into_any_element();
        };
        div()
            .id("chaus-find-bar")
            .debug_selector(|| "chaus-find-bar".to_string())
            .absolute()
            .top(px(8.0))
            .right(px(10.0))
            .w(px(470.0))
            .p(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(silvestris::convert::hsla(theme.colors.group_border))
            .bg(silvestris::convert::hsla(theme.colors.window))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_key_down(cx.listener(|editor, event: &KeyDownEvent, window, cx| {
                editor.find_bar_key(event, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .debug_selector(|| "find-field".to_string())
                            .child(bar.find.clone()),
                    )
                    .child(
                        Button::new("find-previous", "Previous")
                            .size(ControlSize::Small)
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.find_next(false, cx);
                            })),
                    )
                    .child(
                        Button::new("find-next", "Next")
                            .size(ControlSize::Small)
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.find_next(true, cx);
                            })),
                    )
                    .child(
                        Button::new("find-case", "Aa")
                            .size(ControlSize::Small)
                            .tooltip("Match case")
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.toggle_find_case(cx);
                            })),
                    )
                    .child(
                        Button::new("find-close", "Close")
                            .size(ControlSize::Small)
                            .on_click(cx.listener(|editor, _, window, cx| {
                                editor.close_find(window, cx);
                            })),
                    ),
            )
            .when(bar.show_replace, |element| {
                element.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_1()
                                .debug_selector(|| "replace-field".to_string())
                                .child(bar.replace.clone()),
                        )
                        .child(
                            Button::new("replace-one", "Replace")
                                .size(ControlSize::Small)
                                .on_click(cx.listener(|editor, _, _, cx| {
                                    editor.replace_one(cx);
                                })),
                        )
                        .child(
                            Button::new("replace-all", "Replace All")
                                .size(ControlSize::Small)
                                .on_click(cx.listener(|editor, _, _, cx| {
                                    editor.replace_all(cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    fn find_query(&self, cx: &App) -> Option<(String, bool)> {
        let bar = self.find_bar.as_ref()?;
        let needle = bar.find.read(cx).text().to_string();
        if needle.is_empty() {
            return None;
        }
        Some((needle, bar.case_sensitive))
    }

    fn describe_offset(&self, offset: CharOffset) -> String {
        let line = self.document.line_of(offset);
        let column = offset.0.saturating_sub(self.document.line_start(line).0);
        format!("{}:{}", line.0 + 1, column + 1)
    }
}
