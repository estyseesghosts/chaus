//! Routing platform key events into the modal engine.
//!
//! Only unmodified keys (plus `Ctrl+R` for redo) reach the modal grammar. That
//! keeps application shortcuts such as `Ctrl+Shift+V` and `Ctrl+S` working while
//! Normal mode is active, and it prevents Normal-mode letters from leaking into
//! the platform text-input service.

use chaus_core::position::Selection;
use chaus_vim::{Key as VimKey, Mode, Outcome};
use gpui::{Context, KeyDownEvent, Keystroke, Window};

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
        let Some(key) = translate(keystroke, control_redo) else {
            return false;
        };
        let outcome = self
            .vim_engine
            .handle_key(key, &mut self.document, &mut self.selection);
        let consumed = !matches!(outcome, Outcome::Unhandled);
        self.apply_vim_outcome(outcome, window, cx);
        consumed
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
                self.rebuild_rows();
                self.scroll_to_caret();
                self.close_confirmed = false;
                self.force_next_save = false;
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
        let mut words = command.split_whitespace();
        let Some(name) = words.next() else {
            return;
        };
        match name {
            "w" => {
                self.save_inner(cx);
            }
            "q" => {
                if self.confirm_close() {
                    window.remove_window();
                } else {
                    cx.notify();
                }
            }
            "q!" => window.remove_window(),
            "wq" => {
                if self.save_inner(cx) {
                    window.remove_window();
                }
            }
            "e" => {
                self.set_status(":e {path} is not implemented yet; use Ctrl+O to open a file");
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

fn translate(keystroke: &Keystroke, control_redo: bool) -> Option<VimKey> {
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
