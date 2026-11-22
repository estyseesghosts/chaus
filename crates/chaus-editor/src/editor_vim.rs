//! Routing platform key events into the modal engine.
//!
//! Only unmodified keys (plus `Ctrl+R` for redo) reach the modal grammar. That
//! keeps application shortcuts such as `Ctrl+Shift+V` and `Ctrl+S` working while
//! Normal mode is active, and it prevents Normal-mode letters from leaking into
//! the platform text-input service.

use std::time::Duration;

use chaus_core::position::Selection;
use chaus_vim::{Key as VimKey, Mode, Outcome};
use gpui::{Context, KeyDownEvent, Keystroke, Window};

use crate::editor::Editor;

/// Ambiguous user-mapping prefixes wait this long for another key. The timeout
/// applies only to mappings, not to an operator waiting for its motion.
const MAPPING_TIMEOUT: Duration = Duration::from_millis(500);

impl Editor {
    /// Feed a key event to the modal engine. Returns `true` when the event was
    /// consumed and must not reach text input or other handlers.
    pub(crate) fn handle_vim_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // A new key-down starts a new platform message; whatever the previous
        // key suppressed can no longer arrive.
        self.suppress_text_input = false;
        if !self.vim
            || self.find_bar.is_some()
            || self.close_prompt
            || self.conflict_prompt.is_some()
            || !self.recovery_queue.is_empty()
        {
            return false;
        }
        if event.prefer_character_input {
            return false;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let control_redo = modifiers.control
            && !modifiers.shift
            && !modifiers.alt
            && !modifiers.platform
            && keystroke.key.eq_ignore_ascii_case("r");
        let clipboard_shortcut = (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && !modifiers.alt
            && matches!(keystroke.key.as_str(), "c" | "x" | "v");
        if clipboard_shortcut {
            // Copy/cut/paste must still see the Visual selection, so cancel a
            // pending Vim prefix without collapsing it.
            self.vim_engine.cancel_prefix();
            cx.notify();
            return false;
        }
        if (modifiers.control || modifiers.alt || modifiers.platform || modifiers.function)
            && !control_redo
        {
            // An application shortcut abandons a partial Vim command rather
            // than leaving it armed for the next keystroke after the action.
            self.cancel_vim_prefix(cx);
            return false;
        }
        let Some(mut key) = translate(keystroke, control_redo) else {
            return false;
        };
        let mode = self.vim_engine.mode();
        if mode == Mode::CommandLine
            && matches!(
                keystroke.key.as_str(),
                "left" | "right" | "up" | "down" | "home" | "end" | "delete"
            )
        {
            // Arrow/navigation names must not turn into `h`, `l`, `0`, `$`
            // inside an Ex command line.
            return false;
        }
        if matches!(mode, Mode::Normal | Mode::VisualChar | Mode::VisualLine) {
            key = match key {
                VimKey::Enter => VimKey::Char('j'),
                VimKey::Backspace => VimKey::Char('h'),
                other => other,
            };
        }
        let outcome = self
            .vim_engine
            .handle_key(key, &mut self.document, &mut self.selection);
        let consumed = !matches!(outcome, Outcome::Unhandled);
        self.apply_vim_outcome(outcome, window, cx);
        self.arm_mapping_timeout(cx);
        self.suppress_text_input = consumed;
        consumed
    }

    fn arm_mapping_timeout(&mut self, cx: &mut Context<Self>) {
        if !self.vim_engine.mapping_prefix_pending() {
            return;
        }
        self.mapping_epoch = self.mapping_epoch.wrapping_add(1);
        let epoch = self.mapping_epoch;
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(MAPPING_TIMEOUT).await;
            editor
                .update_in(cx, |editor, window, cx| {
                    if editor.mapping_epoch != epoch || !editor.vim_engine.mapping_prefix_pending()
                    {
                        return;
                    }
                    let outcome = editor
                        .vim_engine
                        .resolve_mapping_timeout(&mut editor.document, &mut editor.selection);
                    editor.apply_vim_outcome(outcome, window, cx);
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Cancel a pending modal sequence from the Escape action.
    pub(crate) fn handle_vim_escape(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.vim {
            return false;
        }
        let outcome =
            self.vim_engine
                .handle_key(VimKey::Escape, &mut self.document, &mut self.selection);
        self.apply_vim_outcome(outcome, window, cx);
        true
    }

    fn apply_vim_outcome(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        match outcome {
            Outcome::Unhandled => {}
            Outcome::Changed => {
                let affected = self.document.last_changed_lines();
                self.refresh_rows(affected);
                self.scroll_to_caret();
                self.force_next_save = false;
                cx.notify();
            }
            Outcome::Handled => {
                self.scroll_to_caret();
                cx.notify();
            }
            Outcome::Message(message) => {
                self.flash_status(message, cx);
                cx.notify();
            }
            Outcome::Command(command) => self.run_ex_command(&command, window, cx),
            // `Outcome` is `#[non_exhaustive]`; future variants are treated as
            // informational until routed explicitly.
            _ => {}
        }
    }

    /// Drop a partially typed Vim command when focus leaves the editor.
    pub fn cancel_vim_prefix(&mut self, cx: &mut Context<Self>) {
        if self.vim {
            let was_visual = matches!(self.vim_engine.mode(), Mode::VisualChar | Mode::VisualLine);
            self.vim_engine.cancel_prefix();
            if was_visual {
                self.selection = Selection::caret(self.selection.head);
            }
            cx.notify();
        }
    }

    /// Corner indicator text for the current modal state.
    #[must_use]
    pub fn vim_mode_label(&self) -> Option<&'static str> {
        if self.vim {
            Some(self.vim_engine.mode().label())
        } else {
            None
        }
    }

    fn run_ex_command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name_end = command.find(char::is_whitespace).unwrap_or(command.len());
        let name = &command[..name_end];
        let rest = command[name_end..].trim();
        match name {
            "w" => {
                self.save_inner(false, cx);
            }
            "q" => {
                if self.confirm_close(window, cx) {
                    window.remove_window();
                }
            }
            "q!" => self.discard_and_close(window),
            "wq" => {
                self.save_inner(true, cx);
            }
            "e" | "e!" => {
                if rest.is_empty() {
                    self.set_status("usage: :e {path}");
                    cx.notify();
                } else {
                    self.open_path(std::path::Path::new(rest), name == "e!", cx);
                }
            }
            "set" => self.run_set_command(rest, cx),
            "format" => self.format_document(cx),
            "trim" => self.trim_trailing_whitespace(cx),
            "retab" => self.convert_indentation_to_spaces(cx),
            _ => {
                self.set_status(format!("unknown Ex command ':{name}'"));
                cx.notify();
            }
        }
    }

    fn run_set_command(&mut self, arguments: &str, cx: &mut Context<Self>) {
        if arguments.is_empty() {
            self.set_status(
                "usage: :set wrap=word|char|none|wrap|nowrap|expandtab|noexpandtab|tabstop=N",
            );
            cx.notify();
            return;
        }
        let mut settings = self.settings.clone();
        let mut changed = false;
        for argument in arguments.split_whitespace() {
            match argument {
                "wrap" => {
                    settings.wrap = chaus_core::settings::WrapMode::Word;
                    changed = true;
                }
                "nowrap" => {
                    settings.wrap = chaus_core::settings::WrapMode::None;
                    changed = true;
                }
                "expandtab" => {
                    settings.expand_tabs = true;
                    changed = true;
                }
                "noexpandtab" => {
                    settings.expand_tabs = false;
                    changed = true;
                }
                _ => {
                    if let Some(value) = argument.strip_prefix("wrap=") {
                        match chaus_core::settings::WrapMode::parse(value) {
                            Some(mode) => {
                                settings.wrap = mode;
                                changed = true;
                            }
                            None => {
                                self.set_status(format!("unknown wrap mode '{value}'"));
                                cx.notify();
                                return;
                            }
                        }
                    } else if let Some(value) = argument.strip_prefix("tabstop=") {
                        match value.parse::<usize>() {
                            Ok(tab_stop) if (1..=16).contains(&tab_stop) => {
                                settings.tab_stop = tab_stop;
                                changed = true;
                            }
                            _ => {
                                self.set_status("tabstop must be between 1 and 16");
                                cx.notify();
                                return;
                            }
                        }
                    } else {
                        self.set_status(format!("unsupported setting '{argument}'"));
                        cx.notify();
                        return;
                    }
                }
            }
        }
        if changed {
            self.set_settings(settings);
            self.flash_status("Settings updated for this window", cx);
        }
        cx.notify();
    }
}

fn translate(keystroke: &Keystroke, control_redo: bool) -> Option<VimKey> {
    if control_redo {
        return Some(VimKey::Control('r'));
    }
    match keystroke.key.as_str() {
        "escape" => Some(VimKey::Escape),
        "enter" => Some(VimKey::Enter),
        "backspace" => Some(VimKey::Backspace),
        "tab" => Some(VimKey::Tab),
        "left" => Some(VimKey::Char('h')),
        "right" => Some(VimKey::Char('l')),
        "up" => Some(VimKey::Char('k')),
        "down" => Some(VimKey::Char('j')),
        "home" => Some(VimKey::Char('0')),
        "end" => Some(VimKey::Char('$')),
        "delete" => Some(VimKey::Char('x')),
        "space" => Some(VimKey::Char(' ')),
        "pageup" | "pagedown" | "insert" => None,
        key => {
            if let Some(text) = &keystroke.key_char {
                return text.chars().next().map(VimKey::Char);
            }
            // Never turn a multi-character key name such as "f5" into its
            // first letter.
            let mut characters = key.chars();
            let character = characters.next()?;
            if characters.next().is_some() {
                return None;
            }
            Some(VimKey::Char(character))
        }
    }
}
