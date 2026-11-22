//! Routing platform key events into the modal engine.
//!
//! Only unmodified keys (plus `Ctrl+R` for redo) reach the modal grammar. That
//! keeps application shortcuts such as `Ctrl+Shift+V` and `Ctrl+S` working while
//! Normal mode is active, and it prevents Normal-mode letters from leaking into
//! the platform text-input service.

use chaus_vim::{Key as VimKey, Outcome};
use gpui::{Context, KeyDownEvent, Window};

use crate::editor::Editor;

impl Editor {
    /// Feed a key event to the modal engine. Returns `true` when the event was
    /// consumed and must not reach text input or other handlers.
    pub(crate) fn handle_vim_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.vim {
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
        if (modifiers.control || modifiers.alt || modifiers.platform || modifiers.function)
            && !control_redo
        {
            return false;
        }
        let Some(key) = translate(keystroke, control_redo) else {
            return false;
        };
        let outcome = self
            .vim_engine
            .handle_key(key, &mut self.document, &mut self.selection);
        match outcome {
            Outcome::Unhandled => false,
            Outcome::Changed => {
                self.rebuild_rows();
                self.scroll_to_caret();
                self.close_confirmed = false;
                cx.notify();
                true
            }
            Outcome::Handled => {
                self.scroll_to_caret();
                cx.notify();
                true
            }
            Outcome::Message(message) => {
                self.set_status(message);
                cx.notify();
                true
            }
            Outcome::Command(command) => {
                self.run_ex_command(&command, window, cx);
                true
            }
            _ => false,
        }
    }

    /// Cancel a pending modal sequence from the Escape action.
    pub(crate) fn handle_vim_escape(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.vim {
            return false;
        }
        let outcome =
            self.vim_engine
                .handle_key(VimKey::Escape, &mut self.document, &mut self.selection);
        match outcome {
            Outcome::Changed => {
                self.rebuild_rows();
                self.scroll_to_caret();
                cx.notify();
            }
            Outcome::Handled => {
                self.scroll_to_caret();
                cx.notify();
            }
            Outcome::Message(message) => {
                self.set_status(message);
                cx.notify();
            }
            _ => {}
        }
        true
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
        let mut words = command.split_whitespace();
        let Some(name) = words.next() else {
            return;
        };
        match name {
            "w" => {
                self.save_inner(false, cx);
            }
            "q" => {
                if self.confirm_close() {
                    window.remove_window();
                } else {
                    let message = self
                        .status
                        .clone()
                        .unwrap_or_else(|| "Unsaved changes: close again to discard".to_string());
                    self.set_status(message);
                    cx.notify();
                }
            }
            "q!" => window.remove_window(),
            "wq" => {
                if self.save_inner(false, cx) {
                    window.remove_window();
                }
            }
            "e" => {
                self.set_status(":e needs a native open dialog (not wired up yet)");
                cx.notify();
            }
            "set" => {
                self.set_status("Change settings in _chausrc");
                cx.notify();
            }
            _ => {
                self.set_status(format!("unknown Ex command ':{name}'"));
                cx.notify();
            }
        }
    }
}

fn translate(keystroke: &gpui::Keystroke, control_redo: bool) -> Option<VimKey> {
    if control_redo {
        return Some(VimKey::Control('r'));
    }
    match keystroke.key.as_str() {
        "escape" => Some(VimKey::Escape),
        "enter" => Some(VimKey::Enter),
        "backspace" => Some(VimKey::Backspace),
        "tab" => Some(VimKey::Tab),
        _ => {
            if let Some(text) = &keystroke.key_char {
                return text.chars().next().map(VimKey::Char);
            }
            let mut characters = keystroke.key.chars();
            let character = characters.next()?;
            if characters.next().is_some() {
                return None;
            }
            Some(VimKey::Char(character))
        }
    }
}
