//! Routing platform key events into the modal engine.
//!
//! Only unmodified keys (plus `Ctrl+R` for redo) reach the modal grammar. That
//! keeps application shortcuts such as `Ctrl+Shift+V` and `Ctrl+S` working while
//! Normal mode is active, and it prevents Normal-mode letters from leaking into
//! the platform text-input service.

use std::time::Duration;

use chaus_core::position::{DocumentRevision, Selection};
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
        self.text_input.clear_suppression();
        if !self.vim.enabled()
            || self.find_bar.is_some()
            || self.close_prompt
            || self.conflict_prompt.is_some()
            || self.recovery.is_open()
        {
            return false;
        }
        if event.prefer_character_input {
            return false;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain_control = modifiers.control
            && !modifiers.shift
            && !modifiers.alt
            && !modifiers.platform
            && !modifiers.function;
        let control_redo = plain_control && keystroke.key.eq_ignore_ascii_case("r");
        // Only the two Vim half-page keys are admitted from Ctrl; every other
        // application shortcut keeps its existing priority.
        let control_scroll = plain_control
            && matches!(keystroke.key.as_str(), "d" | "u")
            && matches!(
                self.vim.mode(),
                Mode::Normal | Mode::VisualChar | Mode::VisualLine
            );
        let clipboard_shortcut = (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && !modifiers.alt
            && matches!(keystroke.key.as_str(), "c" | "x" | "v");
        if clipboard_shortcut && self.vim.mode() == Mode::CommandLine {
            // Paste goes into the open command line; it must stay open.
            return false;
        }
        if clipboard_shortcut {
            // Copy/cut/paste must still see the Visual selection, so cancel a
            // pending Vim prefix without collapsing it.
            self.vim.cancel_prefix();
            cx.notify();
            return false;
        }
        if (modifiers.control || modifiers.alt || modifiers.platform || modifiers.function)
            && !control_redo
            && !control_scroll
        {
            // An application shortcut abandons a partial Vim command rather
            // than leaving it armed for the next keystroke after the action.
            self.cancel_vim_prefix(cx);
            return false;
        }
        let Some(mut key) = translate(keystroke, control_redo, control_scroll) else {
            return false;
        };
        let mode = self.vim.mode();
        if matches!(mode, Mode::Normal | Mode::VisualChar | Mode::VisualLine) {
            key = match key {
                VimKey::Enter => VimKey::Char('j'),
                VimKey::Backspace => VimKey::Char('h'),
                other => other,
            };
        }
        let before = self.document.revision();
        let outcome = self
            .vim
            .handle_key(key, &mut self.document, &mut self.selection);
        let consumed = !matches!(outcome, Outcome::Unhandled);
        self.apply_vim_outcome(outcome, before, window, cx);
        self.arm_mapping_timeout(cx);
        if consumed {
            self.text_input.suppress_next();
        } else {
            self.text_input.clear_suppression();
        }
        consumed
    }

    fn arm_mapping_timeout(&mut self, cx: &mut Context<Self>) {
        if !self.vim.mapping_prefix_pending() {
            return;
        }
        let epoch = self.vim.begin_mapping_timeout();
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(MAPPING_TIMEOUT).await;
            editor
                .update_in(cx, |editor, window, cx| {
                    if !editor.vim.is_current_timeout(epoch) || !editor.vim.mapping_prefix_pending()
                    {
                        return;
                    }
                    let before = editor.document.revision();
                    let outcome = editor
                        .vim
                        .resolve_mapping_timeout(&mut editor.document, &mut editor.selection);
                    editor.apply_vim_outcome(outcome, before, window, cx);
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
        if !self.vim.enabled() {
            return false;
        }
        let before = self.document.revision();
        let outcome = self
            .vim
            .handle_key(VimKey::Escape, &mut self.document, &mut self.selection);
        self.apply_vim_outcome(outcome, before, window, cx);
        true
    }

    fn apply_vim_outcome(
        &mut self,
        outcome: Outcome,
        before: DocumentRevision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The engine normally reports `Changed`, but deriving the revision
        // comparison keeps bookkeeping correct if an edit path forgets to.
        let mutated = self.document.revision() != before || matches!(outcome, Outcome::Changed);
        if mutated {
            self.after_opaque_mutation(cx);
        }
        match outcome {
            Outcome::Unhandled => {}
            Outcome::Changed => {}
            Outcome::Handled => {
                if !mutated {
                    self.scroll_to_caret();
                    cx.notify();
                }
            }
            Outcome::Message(message) => {
                self.flash_status(message, cx);
                cx.notify();
            }
            Outcome::Ex(command) => self.run_ex_command(command, window, cx),
            Outcome::Complete => self.complete_command_path(cx),
            Outcome::Indent {
                outdent,
                first_line,
                last_line,
            } => self.indent_line_range(first_line, last_line, outdent, cx),
            Outcome::Viewport(intent) => self.apply_viewport_intent(intent, cx),
            // `Outcome` is `#[non_exhaustive]`; future variants are treated as
            // informational until routed explicitly.
            _ => {}
        }
    }

    /// Drop a partially typed Vim command when focus leaves the editor.
    pub fn cancel_vim_prefix(&mut self, cx: &mut Context<Self>) {
        if self.vim.enabled() {
            let was_visual = self.vim.visual_mode();
            self.vim.cancel_prefix();
            if was_visual {
                self.selection = Selection::caret(self.selection.head);
            }
            cx.notify();
        }
    }

    /// Corner indicator text for the current modal state.
    #[must_use]
    pub fn vim_mode_label(&self) -> Option<&'static str> {
        self.vim.mode_label()
    }
}

fn translate(keystroke: &Keystroke, control_redo: bool, control_scroll: bool) -> Option<VimKey> {
    if control_redo {
        return Some(VimKey::Control('r'));
    }
    if control_scroll {
        return Some(match keystroke.key.as_str() {
            "d" => VimKey::Control('d'),
            "u" => VimKey::Control('u'),
            _ => return None,
        });
    }
    match keystroke.key.as_str() {
        "escape" => Some(VimKey::Escape),
        "enter" => Some(VimKey::Enter),
        "backspace" => Some(VimKey::Backspace),
        "tab" => Some(VimKey::Tab),
        // The engine turns these into `h`, `l`, `0`, `$`, `x` outside the
        // command line and edits the input line inside it.
        "left" => Some(VimKey::Left),
        "right" => Some(VimKey::Right),
        "up" => Some(VimKey::Up),
        "down" => Some(VimKey::Down),
        "home" => Some(VimKey::Home),
        "end" => Some(VimKey::End),
        "delete" => Some(VimKey::Delete),
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
