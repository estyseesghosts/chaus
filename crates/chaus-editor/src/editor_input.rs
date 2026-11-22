//! The pinned GPUI `EntityInputHandler` bridge.
//!
//! Platform text services speak UTF-16; the document speaks Rope char
//! indices. All conversions go through the document's indexed converters. IME
//! preedit replaces the previous preedit transaction instead of piling up
//! undo entries.

use std::ops::Range;

use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use gpui::{
    Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window,
};

use crate::editor::Editor;
use crate::editor_edit::normalize_newlines;
use crate::geometry::Geometry;

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.document.utf16_range_to_char(range_utf16);
        actual_range.replace(self.document.char_range_to_utf16(range));
        Some(self.document.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.document.char_range_to_utf16(self.selection.range()),
            reversed: self.selection.reversed(),
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.document.utf16_range_to_char(range_utf16);
        self.selection = Selection::new(range.start, range.end);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(
            self.document
                .char_to_utf16(CharOffset(self.document.len_chars()))
                .0,
        )
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .map(|range| self.document.char_range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
        self.preedit_active = false;
        self.preedit_start = None;
        window.notify_text_input_changed(&self.focus);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Windows sends WM_CHAR for printable keys even when the key-down was
        // consumed by the modal engine. In Normal/Visual/Command modes the
        // engine owns those keys; letting the character through would insert
        // `j`, `d`, `:` and so on as literal text.
        if self.suppress_text_input {
            // The OS delivered WM_CHAR for a key the modal engine already
            // consumed. Drop exactly this character message.
            self.suppress_text_input = false;
            return;
        }
        if !self.accepts_text_input(window, cx) {
            return;
        }
        let text = normalize_newlines(new_text);
        let range = match range_utf16 {
            Some(range) => self.document.utf16_range_to_char(range),
            None => self.marked.unwrap_or_else(|| self.selection.range()),
        };
        let after = Selection::caret(CharOffset(range.start.0 + text.chars().count()));
        let cause = if self.preedit_active {
            EditCause::ImeCommit
        } else {
            EditCause::Typing
        };
        let transaction =
            EditTransaction::new(cause, TextEdit::new(range, text), self.selection, after);
        if let Ok(result) = self.document.apply(transaction) {
            self.selection = after;
            self.marked = None;
            self.preedit_active = false;
            self.preedit_start = None;
            self.after_change(&result, cx);
            window.notify_text_input_changed(&self.focus);
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suppress_text_input {
            self.suppress_text_input = false;
            return;
        }
        if !self.accepts_text_input(window, cx) {
            return;
        }
        if self.preedit_active && self.document.undo().is_some() {
            self.refresh_rows(self.document.last_changed_lines());
        }
        let text = normalize_newlines(new_text);
        let start = self
            .preedit_start
            .or_else(|| range_utf16.map(|range| self.document.utf16_range_to_char(range).start))
            .unwrap_or_else(|| self.selection.range().start);
        self.selection = Selection::caret(start);
        let range = CharRange::empty(start);
        let character_count = text.chars().count();
        let after = Selection::caret(CharOffset(start.0 + character_count));
        // The native text service reports the selection within the marked
        // text, not within the document. Convert it through UTF-16 boundaries
        // so a composing window/segments selection is preserved.
        let selected = new_selected_range_utf16
            .and_then(|range| {
                let from = char_index_for_utf16_offset(&text, range.start)?;
                let to = char_index_for_utf16_offset(&text, range.end.max(range.start))?;
                Some(Selection::new(
                    CharOffset(start.0 + from),
                    CharOffset(start.0 + to),
                ))
            })
            .unwrap_or(after);
        let transaction = EditTransaction::new(
            EditCause::ImeCommit,
            TextEdit::new(range, text),
            self.selection,
            selected,
        );
        if let Ok(result) = self.document.apply(transaction) {
            self.selection = selected;
            self.preedit_start = Some(start);
            self.preedit_active = true;
            self.marked = Some(CharRange::new(start, after.head));
            self.after_change(&result, cx);
            window.notify_text_input_changed(&self.focus);
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry?;
        let range = self.document.utf16_range_to_char(range_utf16);
        let start = self.bounds_for_offset(range.start, geometry);
        let end = self.bounds_for_offset(range.end, geometry);
        Some(Bounds::from_corners(
            start,
            Point {
                x: end.x.max(start.x + gpui::px(1.0)),
                y: end.y + geometry.line_height,
            },
        ))
    }

    fn selection_bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Vec<Bounds<Pixels>> {
        let Some(geometry) = self.geometry else {
            return Vec::new();
        };
        let range = self.document.utf16_range_to_char(range_utf16);
        if range.is_empty() {
            return Vec::new();
        }
        let first_line = self.document.line_of(range.start);
        let last_line = self
            .document
            .line_of(CharOffset(range.end.0.saturating_sub(1)));
        let origin = geometry.text_origin();
        let mut rectangles = Vec::new();
        for line_number in first_line.0..=last_line.0 {
            let line = LineIndex(line_number);
            let line_start = self.document.line_start(line);
            let line_text = self.document.line_text(line);
            let segments = self.segments_for(line);
            for segment in &segments {
                let segment_start = CharOffset(line_start.0 + segment.char_start);
                let segment_end = CharOffset(line_start.0 + segment.char_end);
                let start = range.start.max(segment_start);
                let end = range.end.min(segment_end);
                if start >= end {
                    continue;
                }
                let (_, row) = self.row_of_offset(segment_start);
                let y = origin.y
                    + gpui::px((row as f32 - self.scroll_row) * f32::from(geometry.line_height));
                if y + geometry.line_height < element_bounds.top() || y > element_bounds.bottom() {
                    continue;
                }
                let absolute_start = chaus_layout::column_at(
                    &line_text,
                    start.0.saturating_sub(line_start.0),
                    self.settings.tab_stop,
                )
                .max(segment.start_column);
                let absolute_end = if range.end >= segment_end {
                    segment.end_column()
                } else {
                    chaus_layout::column_at(
                        &line_text,
                        end.0.saturating_sub(line_start.0),
                        self.settings.tab_stop,
                    )
                }
                .max(segment.start_column)
                .min(segment.end_column());
                let Some((start_column, end_column)) =
                    chaus_layout::local_selection_columns(segment, absolute_start, absolute_end)
                else {
                    continue;
                };
                let x = origin.x - self.scroll_x
                    + gpui::px(start_column as f32 * f32::from(geometry.char_width));
                let width =
                    gpui::px((end_column - start_column) as f32 * f32::from(geometry.char_width));
                rectangles.push(Bounds::from_corners(
                    Point { x, y },
                    Point {
                        x: x + width.max(gpui::px(1.0)),
                        y: y + geometry.line_height,
                    },
                ));
            }
        }
        rectangles
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_point(point)?;
        Some(self.document.char_to_utf16(offset).0)
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        // GPUI derives `prefers_ime_for_printable_keys` from this method, so
        // Normal mode sends printable keys through the command grammar while
        // Insert mode and standard editing defer to an active IME.
        !self.vim || self.vim_engine.mode().accepts_text()
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.replace_selection(&text, EditCause::Paste, cx);
            window.notify_text_input_changed(&self.focus);
        }
    }
}

impl Editor {
    fn bounds_for_offset(&mut self, offset: CharOffset, geometry: Geometry) -> Point<Pixels> {
        let (line, row) = self.row_of_offset(offset);
        let line_start = self.document.line_start(line);
        let text = self.document.line_text(line);
        let column = chaus_layout::column_at(
            &text,
            offset.0.saturating_sub(line_start.0),
            self.settings.tab_stop,
        );
        let segments = self.segments_for(line);
        let local_row = self.local_row_of(offset, line);
        let segment_start = segments
            .get(local_row)
            .map(|segment| segment.start_column)
            .unwrap_or(0);
        let local_column = column.saturating_sub(segment_start);
        let origin = geometry.text_origin();
        Point {
            x: origin.x - self.scroll_x
                + gpui::px(local_column as f32 * f32::from(geometry.char_width)),
            y: origin.y
                + gpui::px((row as f32 - self.scroll_row) * f32::from(geometry.line_height)),
        }
    }
}

/// Convert a UTF-16 offset within `text` to a char index. Offsets that land
/// inside a surrogate pair round down to that scalar's start.
fn char_index_for_utf16_offset(text: &str, offset: usize) -> Option<usize> {
    let mut utf16 = 0;
    for (index, character) in text.chars().enumerate() {
        if utf16 >= offset {
            return Some(index);
        }
        let next = utf16 + character.len_utf16();
        if offset < next {
            return Some(index);
        }
        utf16 = next;
    }
    (utf16 >= offset).then_some(text.chars().count())
}
