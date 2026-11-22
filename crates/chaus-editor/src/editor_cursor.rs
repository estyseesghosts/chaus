//! Caret movement, word boundaries and scroll-follow.

use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use gpui::{px, Context, Window};

use crate::editor::Editor;
use crate::editor_keys::{
    DocumentEnd, DocumentStart, Down, End, Home, Left, Right, SelectDocumentEnd,
    SelectDocumentStart, SelectDown, SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp,
    SelectWordLeft, SelectWordRight, Up, WordLeft, WordRight,
};

impl Editor {
    pub(crate) fn move_to(&mut self, offset: CharOffset, extend: bool, cx: &mut Context<Self>) {
        let offset = offset.min(CharOffset(self.document.len_chars()));
        self.selection = if extend {
            Selection::new(self.selection.anchor, offset)
        } else {
            Selection::caret(offset)
        };
        self.preferred_column = None;
        self.scroll_to_caret();
        cx.notify();
    }

    fn move_horizontal(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let head = self.selection.head;
        if !extend && !self.selection.is_empty() {
            let target = if forward {
                self.selection.range().end
            } else {
                self.selection.range().start
            };
            self.move_to(target, false, cx);
            return;
        }
        let target = if forward {
            self.document.next_grapheme_boundary(head)
        } else {
            self.document.prev_grapheme_boundary(head)
        };
        self.move_to(target, extend, cx);
    }

    fn move_word(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let head = self.selection.head;
        let target = if forward {
            self.word_right(head)
        } else {
            self.word_left(head)
        };
        self.move_to(target, extend, cx);
    }

    fn move_vertical(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        self.ensure_prefix();
        let (line, row_in_line) = self.row_of_offset(self.selection.head);
        let base_row = self.prefix_rows.get(line.0).copied().unwrap_or(0) + row_in_line;
        let total = self.total_rows();
        let target_row = (base_row as isize + delta).clamp(0, total as isize - 1) as usize;

        // Preferred x is relative to the current visual row, not an absolute
        // column in the hard line. Otherwise a wrapped row would land on the
        // wrong segment once `offset_at` spans the whole logical line.
        let (_, current_span) = self.row_span(base_row);
        let x = self.preferred_column.unwrap_or_else(|| {
            self.caret_column(self.selection.head)
                .saturating_sub(current_span.start)
        });
        self.preferred_column = Some(x);

        let (target_line, span) = self.row_span(target_row);
        let column = span.start.saturating_add(x).min(span.end);
        let offset = self.offset_at(target_line, column);
        self.selection = if extend {
            Selection::new(self.selection.anchor, offset)
        } else {
            Selection::caret(offset)
        };
        self.scroll_to_caret();
        cx.notify();
    }

    pub(crate) fn line_at_row(&mut self, row: usize) -> LineIndex {
        self.ensure_prefix();
        let index = match self.prefix_rows.binary_search(&row) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        LineIndex(index.min(self.line_rows.len().saturating_sub(1)))
    }

    fn move_line_boundary(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let head = self.selection.head;
        let line = self.document.line_of(head);
        let start = self.document.line_start(line);
        let content_end = self.document.line_content_end(line);
        let target = if end {
            content_end
        } else {
            let text = self.document.line_text(line);
            let first_non_space = text
                .char_indices()
                .find(|(_, ch)| *ch != ' ' && *ch != '\t')
                .map(|(byte, _)| text[..byte].chars().count())
                .unwrap_or(0);
            let smart = CharOffset(start.0 + first_non_space);
            if head == smart {
                start
            } else {
                smart
            }
        };
        self.move_to(target, extend, cx);
    }

    fn move_document_boundary(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let offset = if end {
            CharOffset(self.document.len_chars())
        } else {
            CharOffset::ZERO
        };
        self.move_to(offset, extend, cx);
    }

    pub(crate) fn word_left(&self, offset: CharOffset) -> CharOffset {
        let rope = self.document.rope();
        let mut index = offset.0.min(self.document.len_chars());
        while index > 0 && !is_word_char(rope.char(index - 1)) {
            index -= 1;
        }
        while index > 0 && is_word_char(rope.char(index - 1)) {
            index -= 1;
        }
        CharOffset(index)
    }

    pub(crate) fn word_right(&self, offset: CharOffset) -> CharOffset {
        let rope = self.document.rope();
        let length = self.document.len_chars();
        let mut index = offset.0.min(length);
        while index < length && !is_word_char(rope.char(index)) {
            index += 1;
        }
        while index < length && is_word_char(rope.char(index)) {
            index += 1;
        }
        CharOffset(index)
    }

    pub(crate) fn word_bounds(&self, offset: CharOffset) -> CharRange {
        let rope = self.document.rope();
        let length = self.document.len_chars();
        if length == 0 {
            return CharRange::empty(offset);
        }
        let mut start = offset.0.min(length - 1);
        let mut end = start;
        if is_word_char(rope.char(start)) {
            while start > 0 && is_word_char(rope.char(start - 1)) {
                start -= 1;
            }
            while end < length && is_word_char(rope.char(end)) {
                end += 1;
            }
        } else {
            while start > 0 && !is_word_char(rope.char(start - 1)) && rope.char(start - 1) != '\n' {
                start -= 1;
            }
            while end < length && !is_word_char(rope.char(end)) && rope.char(end) != '\n' {
                end += 1;
            }
            if end == start && end < length {
                end += 1;
            }
        }
        CharRange::new(CharOffset(start), CharOffset(end))
    }

    pub(crate) fn scroll_to_caret(&mut self) {
        let Some(geometry) = self.geometry else {
            return;
        };
        let (_, row) = self.row_of_offset(self.selection.head);
        let visible = geometry.visible_rows();
        let first = self.scroll_row.floor().max(0.0) as usize;
        if row < first {
            self.scroll_row = row as f32;
        } else if row >= first + visible {
            self.scroll_row = (row + 1).saturating_sub(visible) as f32;
        }
        self.clamp_scroll();

        if self.rows_columns.is_none() {
            let column = self.caret_column(self.selection.head);
            let caret_x = px(column as f32 * f32::from(geometry.char_width));
            let width = geometry.bounds.size.width - geometry.padding() * 2.0;
            if caret_x - self.scroll_x > width {
                self.scroll_x = caret_x - width;
            }
            if caret_x < self.scroll_x {
                self.scroll_x = caret_x;
            }
            self.scroll_x = self.scroll_x.max(px(0.));
        } else {
            self.scroll_x = px(0.);
        }
    }

    pub(crate) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, false, cx);
    }

    pub(crate) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, false, cx);
    }

    pub(crate) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, true, cx);
    }

    pub(crate) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, true, cx);
    }

    pub(crate) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, false, cx);
    }

    pub(crate) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, false, cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1, true, cx);
    }

    pub(crate) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1, true, cx);
    }

    pub(crate) fn word_left_action(
        &mut self,
        _: &WordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_word(false, false, cx);
    }

    pub(crate) fn word_right_action(
        &mut self,
        _: &WordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_word(true, false, cx);
    }

    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_word(false, true, cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_word(true, true, cx);
    }

    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line_boundary(false, false, cx);
    }

    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line_boundary(true, false, cx);
    }

    pub(crate) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line_boundary(false, true, cx);
    }

    pub(crate) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line_boundary(true, true, cx);
    }

    pub(crate) fn document_start(
        &mut self,
        _: &DocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_document_boundary(false, false, cx);
    }

    pub(crate) fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_document_boundary(true, false, cx);
    }

    pub(crate) fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_document_boundary(false, true, cx);
    }

    pub(crate) fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_document_boundary(true, true, cx);
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
