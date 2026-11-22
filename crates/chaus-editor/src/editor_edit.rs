//! Editing operations and the actions that trigger them.

use chaus_core::document::{EditCause, EditResult, EditTransaction, LineRange, TextEdit};
use chaus_core::position::{CharOffset, Selection};
use gpui::{ClipboardItem, Context, Window};

use crate::editor::Editor;
use crate::editor_keys::{
    Backspace, ConvertIndentationToSpaces, ConvertIndentationToTabs, Copy, Cut, Delete,
    DeleteWordLeft, DeleteWordRight, Escape, InsertNewline, InsertTab, Outdent, Paste, Redo,
    SelectAll, ToggleVim, TrimTrailingWhitespace, Undo,
};

impl Editor {
    /// Complete an ordinary edit. Every document-changing path routes through
    /// here so layout, recovery, highlighting and the forced-overwrite decision
    /// cannot drift apart.
    pub(crate) fn after_change(&mut self, result: &EditResult, cx: &mut Context<Self>) {
        self.after_document_mutation(result.affected_lines, cx);
        self.clear_composition();
        self.after_highlight_edit(result, cx);
    }

    /// Complete an IME preedit update without clearing the marked range that
    /// the composition just installed.
    pub(crate) fn after_composition_change(&mut self, result: &EditResult, cx: &mut Context<Self>) {
        self.after_document_mutation(result.affected_lines, cx);
        self.after_highlight_edit(result, cx);
    }

    /// Side effects shared by every mutation: standard edits, Vim, undo/redo,
    /// IME, find/replace, formatting and indentation. Recovery is debounced and
    /// epoch-tagged, so a clean document retires the snapshot instead of
    /// writing a stale one.
    pub(crate) fn after_document_mutation(&mut self, affected: LineRange, cx: &mut Context<Self>) {
        self.force_next_save = false;
        self.refresh_rows(affected);
        self.scroll_to_caret();
        self.schedule_recovery(cx);
    }

    pub(crate) fn clear_composition(&mut self) {
        self.marked = None;
        self.preedit_start = None;
        self.preedit_active = false;
    }

    /// Complete an undo/redo. `EditResult` is not available because history
    /// replays whole transactions, so highlighting reparses from the current
    /// revision.
    pub(crate) fn after_history_change(&mut self, cx: &mut Context<Self>) {
        self.after_document_mutation(self.document.last_changed_lines(), cx);
        self.clear_composition();
        self.preferred_column = None;
        self.schedule_highlight(None, cx);
        cx.notify();
    }

    pub(crate) fn replace_selection(
        &mut self,
        text: &str,
        cause: EditCause,
        cx: &mut Context<Self>,
    ) {
        let text = normalize_newlines(text);
        let range = self.selection.range();
        if range.is_empty() && text.is_empty() {
            return;
        }
        let after = Selection::caret(CharOffset(range.start.0 + text.chars().count()));
        let transaction =
            EditTransaction::new(cause, TextEdit::new(range, text), self.selection, after);
        if let Ok(result) = self.document.apply(transaction) {
            self.selection = after.clamp(self.document.len_chars());
            self.after_change(&result, cx);
            cx.notify();
        }
    }

    pub(crate) fn delete_selection(&mut self, cause: EditCause, cx: &mut Context<Self>) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        self.replace_selection("", cause, cx);
        true
    }

    pub(crate) fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.delete_selection(EditCause::Delete, cx) {
            return;
        }
        let head = self.selection.head;
        if head.0 == 0 {
            return;
        }
        let start = self.document.prev_grapheme_boundary(head);
        self.selection = Selection::new(start, head);
        self.replace_selection("", EditCause::Delete, cx);
    }

    pub(crate) fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.delete_selection(EditCause::Delete, cx) {
            return;
        }
        let head = self.selection.head;
        if head.0 >= self.document.len_chars() {
            return;
        }
        let end = self.document.next_grapheme_boundary(head);
        self.selection = Selection::new(head, end);
        self.replace_selection("", EditCause::Delete, cx);
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.delete_selection(EditCause::Delete, cx) {
            return;
        }
        let head = self.selection.head;
        let start = self.word_left(head);
        if start == head {
            return;
        }
        self.selection = Selection::new(start, head);
        self.replace_selection("", EditCause::Delete, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.delete_selection(EditCause::Delete, cx) {
            return;
        }
        let head = self.selection.head;
        let end = self.word_right(head);
        if end == head {
            return;
        }
        self.selection = Selection::new(head, end);
        self.replace_selection("", EditCause::Delete, cx);
    }

    pub(crate) fn insert_newline(
        &mut self,
        _: &InsertNewline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let head = self.selection.head;
        let line = self.document.line_of(head);
        let line_start = self.document.line_start(line);
        let text = self.document.line_text(line);
        let indent_length = text
            .chars()
            .take_while(|ch| *ch == ' ' || *ch == '\t')
            .count();
        let indent: String = if head.0 >= line_start.0 + indent_length {
            text.chars().take(indent_length).collect()
        } else {
            String::new()
        };
        self.replace_selection(&format!("\n{indent}"), EditCause::Typing, cx);
    }

    pub(crate) fn insert_tab(&mut self, _: &InsertTab, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.selection.range();
        let multiline = !range.is_empty()
            && self.document.line_of(range.start) != self.document.line_of(range.end);
        if multiline {
            self.indent_selection(cx);
            return;
        }
        if self.settings.expand_tabs {
            let column = self.caret_column(self.selection.head);
            let tab_stop = self.settings.tab_stop.max(1);
            let spaces = tab_stop - (column % tab_stop);
            self.replace_selection(&" ".repeat(spaces), EditCause::Typing, cx);
        } else {
            self.replace_selection("\t", EditCause::Typing, cx);
        }
    }

    pub(crate) fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        self.outdent_selection(cx);
    }

    pub(crate) fn trim_trailing_whitespace_action(
        &mut self,
        _: &TrimTrailingWhitespace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.trim_trailing_whitespace(cx);
    }

    pub(crate) fn convert_indentation_to_spaces_action(
        &mut self,
        _: &ConvertIndentationToSpaces,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_indentation_to_spaces(cx);
    }

    pub(crate) fn convert_indentation_to_tabs_action(
        &mut self,
        _: &ConvertIndentationToTabs,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.convert_indentation_to_tabs(cx);
    }

    pub(crate) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = Selection::new(CharOffset::ZERO, CharOffset(self.document.len_chars()));
        self.preferred_column = None;
        cx.notify();
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            let text = self.document.slice(self.selection.range());
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            self.flash_status("Copied", cx);
            cx.notify();
        }
    }

    pub(crate) fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        let text = self.document.slice(self.selection.range());
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.replace_selection("", EditCause::Delete, cx);
    }

    pub(crate) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_selection(&text, EditCause::Paste, cx);
        }
    }

    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = self.document.undo() {
            self.selection = selection;
            self.after_history_change(cx);
        }
    }

    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = self.document.redo() {
            self.selection = selection;
            self.after_history_change(cx);
        }
    }

    pub(crate) fn toggle_vim(&mut self, _: &ToggleVim, _: &mut Window, cx: &mut Context<Self>) {
        // A modal sheet, find bar or menu owns the keyboard; do not toggle the
        // document mode behind it.
        if self.find_bar.is_some()
            || self.close_prompt
            || self.conflict_prompt.is_some()
            || !self.recovery_queue.is_empty()
        {
            return;
        }
        self.vim = !self.vim;
        self.document.cancel_user_operation();
        self.vim_engine.reset();
        if !self.vim {
            self.selection = Selection::caret(self.selection.head);
        }
        let message = if self.vim {
            "Vim mode on"
        } else {
            "Vim mode off"
        };
        self.flash_status(message, cx);
        cx.notify();
    }

    pub(crate) fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if !self.recovery_queue.is_empty() {
            self.dismiss_recovery(window, cx);
            return;
        }
        if self.find_bar.is_some() {
            self.close_find(window, cx);
            return;
        }
        let had_composition = self.marked.is_some() || self.preedit_active;
        let handled = self.handle_vim_escape(window, cx);
        self.clear_composition_state();
        if had_composition {
            window.notify_text_input_changed(&self.focus);
        }
        if handled {
            return;
        }
        self.selection = Selection::caret(self.selection.head);
        cx.notify();
    }

    fn clear_composition_state(&mut self) {
        self.marked = None;
        self.preedit_active = false;
        self.preedit_start = None;
    }
}

pub(crate) fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
