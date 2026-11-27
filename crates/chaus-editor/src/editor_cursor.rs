//! Caret movement, word boundaries and scroll-follow.

use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use gpui::{Context, Window};
use ropey::iter::Chars;
use ropey::Rope;

use crate::editor::Editor;
use crate::editor_keys::{
    DocumentEnd, DocumentStart, Down, End, Home, Left, Right, SelectDocumentEnd,
    SelectDocumentStart, SelectDown, SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp,
    SelectWordLeft, SelectWordRight, Up, WordLeft, WordRight,
};

#[cfg(test)]
mod word_scan_tests;

impl Editor {
    pub(crate) fn move_to(&mut self, offset: CharOffset, extend: bool, cx: &mut Context<Self>) {
        let offset = offset.min(CharOffset(self.document.len_chars()));
        self.selection = if extend {
            Selection::new(self.selection.anchor, offset)
        } else {
            Selection::caret(offset)
        };
        self.viewport.set_preferred_column(None);
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
        let (_, base_row) = self.row_of_offset(self.selection.head);
        let total = self.total_rows();
        let target_row = (base_row as isize + delta).clamp(0, total as isize - 1) as usize;

        // Preferred x is relative to the current visual row, not an absolute
        // column in the hard line. Otherwise a wrapped row would land on the
        // wrong segment once `offset_at` spans the whole logical line.
        let (_, current_span) = self.row_span(base_row);
        let x = self.viewport.preferred_column().unwrap_or_else(|| {
            chaus_layout::RowColumn::new(
                self.caret_column(self.selection.head)
                    .saturating_sub(current_span.start.get()),
            )
        });
        self.viewport.set_preferred_column(Some(x));

        let (target_line, span) = self.row_span(target_row);
        let column = span.start.get().saturating_add(x.get()).min(span.end.get());
        let offset = self.offset_at(target_line, chaus_layout::VisualColumn::new(column));
        self.selection = if extend {
            Selection::new(self.selection.anchor, offset)
        } else {
            Selection::caret(offset)
        };
        self.scroll_to_caret();
        cx.notify();
    }

    pub(crate) fn line_at_row(&self, row: usize) -> LineIndex {
        self.viewport.line_at_row(row)
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
        scan_word_left(self.document.rope(), offset)
    }

    pub(crate) fn word_right(&self, offset: CharOffset) -> CharOffset {
        scan_word_right(self.document.rope(), offset)
    }

    pub(crate) fn word_bounds(&self, offset: CharOffset) -> CharRange {
        scan_word_bounds(self.document.rope(), offset)
    }

    pub(crate) fn scroll_to_caret(&mut self) {
        let head = self.selection.head;
        self.viewport
            .scroll_to_caret(&self.document, self.settings.tab_stop, head);
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

/// A forward cursor over Rope chars. One buffered char avoids repeated tree
/// lookups while walking a long word or whitespace run.
struct ForwardChars<'a> {
    chars: Chars<'a>,
    buffered: Option<char>,
    index: usize,
    length: usize,
}

impl<'a> ForwardChars<'a> {
    fn new(rope: &'a Rope, from: usize) -> Self {
        let index = from.min(rope.len_chars());
        Self {
            chars: rope.chars_at(index),
            buffered: None,
            index,
            length: rope.len_chars(),
        }
    }

    fn peek(&mut self) -> Option<char> {
        if self.buffered.is_none() && self.index < self.length {
            self.buffered = self.chars.next();
        }
        self.buffered
    }

    fn skip(&mut self) {
        if self.peek().is_some() {
            self.buffered = None;
            self.index += 1;
        }
    }

    fn index(&self) -> usize {
        self.index
    }
}

/// A backward cursor over Rope chars, built from Ropey's reversed iterator.
struct BackwardChars<'a> {
    chars: Chars<'a>,
    buffered: Option<char>,
    index: usize,
}

impl<'a> BackwardChars<'a> {
    fn new(rope: &'a Rope, from: usize) -> Self {
        let index = from.min(rope.len_chars());
        Self {
            chars: rope.chars_at(index).reversed(),
            buffered: None,
            index,
        }
    }

    fn peek_prev(&mut self) -> Option<char> {
        if self.buffered.is_none() && self.index > 0 {
            self.buffered = self.chars.next();
        }
        self.buffered
    }

    fn skip_prev(&mut self) {
        if self.peek_prev().is_some() {
            self.buffered = None;
            self.index -= 1;
        }
    }

    fn index(&self) -> usize {
        self.index
    }
}

fn scan_word_left(rope: &Rope, offset: CharOffset) -> CharOffset {
    let mut scan = BackwardChars::new(rope, offset.0);
    while scan.peek_prev().is_some_and(|ch| !is_word_char(ch)) {
        scan.skip_prev();
    }
    while scan.peek_prev().is_some_and(is_word_char) {
        scan.skip_prev();
    }
    CharOffset(scan.index())
}

fn scan_word_right(rope: &Rope, offset: CharOffset) -> CharOffset {
    let mut scan = ForwardChars::new(rope, offset.0);
    while scan.peek().is_some_and(|ch| !is_word_char(ch)) {
        scan.skip();
    }
    while scan.peek().is_some_and(is_word_char) {
        scan.skip();
    }
    CharOffset(scan.index())
}

/// Double-click word range. Unlike Vim, this word model has only word and
/// non-word characters, and it stops a non-word run at a hard line break.
fn scan_word_bounds(rope: &Rope, offset: CharOffset) -> CharRange {
    let length = rope.len_chars();
    if length == 0 {
        return CharRange::empty(offset);
    }
    let pivot = offset.0.min(length - 1);
    if ForwardChars::new(rope, pivot)
        .peek()
        .is_some_and(is_word_char)
    {
        let mut back = BackwardChars::new(rope, pivot);
        while back.peek_prev().is_some_and(is_word_char) {
            back.skip_prev();
        }
        let start = back.index();
        let mut forward = ForwardChars::new(rope, pivot);
        while forward.peek().is_some_and(is_word_char) {
            forward.skip();
        }
        return CharRange::new(CharOffset(start), CharOffset(forward.index()));
    }
    let mut back = BackwardChars::new(rope, pivot);
    while back
        .peek_prev()
        .is_some_and(|ch| !is_word_char(ch) && ch != '\n')
    {
        back.skip_prev();
    }
    let start = back.index();
    let mut forward = ForwardChars::new(rope, pivot);
    while forward
        .peek()
        .is_some_and(|ch| !is_word_char(ch) && ch != '\n')
    {
        forward.skip();
    }
    let mut end = forward.index();
    if end == start && end < length {
        end += 1;
    }
    CharRange::new(CharOffset(start), CharOffset(end))
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}
