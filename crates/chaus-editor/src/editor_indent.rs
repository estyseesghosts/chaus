//! Selection-aware indentation and whitespace transforms.
//!
//! Every operation builds one [`EditTransaction`], so a five-line indent is a
//! single undo step. Selection anchor and head are remapped through the same
//! edits that change the text, which keeps a reversed selection reversed and
//! leaves the selected text selected.

use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use gpui::Context;

use crate::editor::Editor;

impl Editor {
    /// The hard lines covered by the current selection. A non-empty selection
    /// ending exactly at a line start does not include that next line.
    fn selected_lines(&self) -> (LineIndex, LineIndex) {
        let range = self.selection.range();
        let start = self.document.line_of(range.start);
        if range.is_empty() {
            return (start, start);
        }
        let last = CharOffset(range.end.0.saturating_sub(1));
        (start, self.document.line_of(last))
    }

    /// Indent every selected hard line by one tab stop.
    pub fn indent_selection(&mut self, cx: &mut Context<Self>) {
        let (first, last) = self.selected_lines();
        let edits = self.indent_edits(first, last);
        let after = remap_selection(self.selection, &edits);
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
    }

    /// Remove one indentation level from every selected hard line.
    pub fn outdent_selection(&mut self, cx: &mut Context<Self>) {
        let (first, last) = self.selected_lines();
        let edits = self.outdent_edits(first, last);
        if edits.is_empty() {
            return;
        }
        let after = remap_selection(self.selection, &edits);
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
    }

    /// Indent or outdent a known hard-line range without reading the current
    /// selection as the range. Vim `>>`/`<<` and Visual `>`/`<` use this so
    /// indentation policy stays in the editor.
    pub(crate) fn indent_line_range(
        &mut self,
        first: LineIndex,
        last: LineIndex,
        outdent: bool,
        cx: &mut Context<Self>,
    ) {
        let first = LineIndex(first.0.min(self.document.len_lines().saturating_sub(1)));
        let last = LineIndex(last.0.min(self.document.len_lines().saturating_sub(1)));
        if first > last {
            return;
        }
        let edits = if outdent {
            self.outdent_edits(first, last)
        } else {
            self.indent_edits(first, last)
        };
        if edits.is_empty() {
            let target = chaus_vim::first_non_space(&self.document, first);
            self.selection = Selection::caret(target);
            self.preferred_column = None;
            self.scroll_to_caret();
            cx.notify();
            return;
        }
        let after = Selection::caret(self.document.line_start(first));
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
        let target = chaus_vim::first_non_space(&self.document, first);
        self.selection = Selection::caret(target);
        self.preferred_column = None;
        self.scroll_to_caret();
    }

    fn indent_edits(&self, first: LineIndex, last: LineIndex) -> Vec<TextEdit> {
        let indent = if self.settings.expand_tabs {
            " ".repeat(self.settings.tab_stop.max(1))
        } else {
            "\t".to_owned()
        };
        let mut edits = Vec::with_capacity(last.0 - first.0 + 1);
        for line in first.0..=last.0 {
            let start = self.document.line_start(LineIndex(line));
            edits.push(TextEdit::insertion(start, indent.clone()));
        }
        edits
    }

    fn outdent_edits(&self, first: LineIndex, last: LineIndex) -> Vec<TextEdit> {
        let mut edits = Vec::new();
        for line in first.0..=last.0 {
            let text = self.document.line_text(LineIndex(line));
            let remove = if text.starts_with('\t') {
                1
            } else {
                text.chars()
                    .take(self.settings.tab_stop.max(1))
                    .take_while(|ch| *ch == ' ')
                    .count()
            };
            if remove == 0 {
                continue;
            }
            let start = self.document.line_start(LineIndex(line));
            let end = CharOffset(start.0 + remove);
            edits.push(TextEdit::new(CharRange::new(start, end), ""));
        }
        edits
    }

    /// Remove spaces or tabs at the end of every line in the document.
    pub fn trim_trailing_whitespace(&mut self, cx: &mut Context<Self>) {
        let mut edits = Vec::new();
        for line in 0..self.document.len_lines() {
            let text = self.document.line_text(LineIndex(line));
            let trailing = text
                .chars()
                .rev()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count();
            if trailing > 0 {
                let start = self.document.line_start(LineIndex(line));
                let end = CharOffset(start.0 + text.chars().count());
                let remove = CharOffset(end.0 - trailing);
                edits.push(TextEdit::new(CharRange::new(remove, end), ""));
            }
        }
        if edits.is_empty() {
            self.flash_status("No trailing whitespace", cx);
            return;
        }
        let after = remap_selection(self.selection, &edits);
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
    }

    /// Convert leading tabs to spaces using the configured tab stop.
    pub fn convert_indentation_to_spaces(&mut self, cx: &mut Context<Self>) {
        let tab_stop = self.settings.tab_stop.max(1);
        let mut edits = Vec::new();
        for line in 0..self.document.len_lines() {
            let text = self.document.line_text(LineIndex(line));
            let prefix: String = text
                .chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .collect();
            if !prefix.contains('\t') {
                continue;
            }
            let expanded = expand_tabs(&prefix, tab_stop);
            if expanded == prefix {
                continue;
            }
            let start = self.document.line_start(LineIndex(line));
            let end = CharOffset(start.0 + prefix.chars().count());
            edits.push(TextEdit::new(CharRange::new(start, end), expanded));
        }
        if edits.is_empty() {
            self.flash_status("Indentation already uses spaces", cx);
            return;
        }
        let after = remap_selection(self.selection, &edits);
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
    }

    /// Convert leading runs of `tab_stop` spaces to tabs where they align.
    pub fn convert_indentation_to_tabs(&mut self, cx: &mut Context<Self>) {
        if self.settings.expand_tabs {
            self.flash_status("expandtab is enabled; convert would be undone by Tab", cx);
            return;
        }
        let tab_stop = self.settings.tab_stop.max(1);
        let mut edits = Vec::new();
        for line in 0..self.document.len_lines() {
            let text = self.document.line_text(LineIndex(line));
            let prefix: String = text
                .chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .collect();
            let compacted = compact_spaces(&prefix, tab_stop);
            if compacted == prefix {
                continue;
            }
            let start = self.document.line_start(LineIndex(line));
            let end = CharOffset(start.0 + prefix.chars().count());
            edits.push(TextEdit::new(CharRange::new(start, end), compacted));
        }
        if edits.is_empty() {
            self.flash_status("Indentation already uses tabs", cx);
            return;
        }
        let after = remap_selection(self.selection, &edits);
        self.apply_whitespace_edits(edits, after, EditCause::Indent, cx);
    }

    fn apply_whitespace_edits(
        &mut self,
        edits: Vec<TextEdit>,
        after: Selection,
        cause: EditCause,
        cx: &mut Context<Self>,
    ) {
        let transaction = EditTransaction {
            edits,
            before_selection: self.selection,
            after_selection: after,
            cause,
        };
        if let Ok(result) = self.document.apply(transaction) {
            self.selection = after.clamp(self.document.len_chars());
            self.after_change(&result, cx);
            cx.notify();
        }
    }
}

/// Expand each leading tab to the next tab stop, measured from the line start.
fn expand_tabs(prefix: &str, tab_stop: usize) -> String {
    let mut column = 0usize;
    let mut expanded = String::with_capacity(prefix.len());
    for character in prefix.chars() {
        if character == '\t' {
            let width = tab_stop - (column % tab_stop);
            expanded.extend(std::iter::repeat_n(' ', width));
            column += width;
        } else {
            expanded.push(character);
            column += 1;
        }
    }
    expanded
}

/// Replace leading whitespace with the shortest tab/space spelling of the
/// same visual width. Remainder spaces and mixed prefixes are preserved as
/// columns rather than dropped.
fn compact_spaces(prefix: &str, tab_stop: usize) -> String {
    let tab_stop = tab_stop.max(1);
    let width = expand_tabs(prefix, tab_stop).chars().count();
    let tabs = width / tab_stop;
    let spaces = width % tab_stop;
    let mut compacted = String::with_capacity(tabs + spaces);
    compacted.extend(std::iter::repeat_n('\t', tabs));
    compacted.extend(std::iter::repeat_n(' ', spaces));
    compacted
}

/// Remap a selection through an ordered set of non-overlapping edits.
fn remap_selection(selection: Selection, edits: &[TextEdit]) -> Selection {
    Selection::new(
        remap_offset(selection.anchor, edits),
        remap_offset(selection.head, edits),
    )
}

fn remap_offset(offset: CharOffset, edits: &[TextEdit]) -> CharOffset {
    let mut adjusted = offset;
    for edit in edits {
        let start = edit.range.start;
        let end = edit.range.end;
        let removed = end.0.saturating_sub(start.0);
        let inserted = edit.inserted_chars();
        if offset >= end {
            adjusted = CharOffset(adjusted.0 + inserted - removed);
        } else if offset > start {
            adjusted = CharOffset(adjusted.0.saturating_sub(offset.0 - start.0));
        } else {
            break;
        }
    }
    adjusted
}
