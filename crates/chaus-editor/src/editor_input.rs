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
        self.text_input
            .marked()
            .map(|range| self.document.char_range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut Context<Self>) {
        self.vim.set_command_preedit(None);
        self.text_input.clear_composition();
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
        if self.modal_sheet_open() {
            // A modal sheet owns the window. Character messages can arrive
            // after the key-down that opened or was consumed by the sheet.
            return;
        }
        if self.text_input.take_suppression() {
            // The OS delivered WM_CHAR for a key the modal engine already
            // consumed. Drop exactly this character message.
            return;
        }
        if !self.accepts_text_input(window, cx) {
            return;
        }
        if self.command_line_active() {
            self.insert_command_text(new_text, cx);
            return;
        }
        let text = normalize_newlines(new_text);
        let range = match range_utf16 {
            Some(range) => self.document.utf16_range_to_char(range),
            None => self
                .text_input
                .marked()
                .unwrap_or_else(|| self.selection.range()),
        };
        let after = Selection::caret(CharOffset(range.start.0 + text.chars().count()));
        let cause = if self.text_input.preedit_active() {
            EditCause::ImeCommit
        } else {
            EditCause::Typing
        };
        let transaction =
            EditTransaction::new(cause, TextEdit::new(range, text), self.selection, after);
        if let Ok(result) = self.document.apply(transaction) {
            self.selection = after;
            self.text_input.clear_composition();
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
        if self.text_input.take_suppression() {
            return;
        }
        if !self.accepts_text_input(window, cx) {
            return;
        }
        if self.command_line_active() {
            self.vim.set_command_preedit(Some(new_text.to_owned()));
            cx.notify();
            return;
        }
        if self.text_input.preedit_active() && self.document.undo().is_some() {
            self.refresh_rows(self.document.last_changed_lines());
        }
        let text = normalize_newlines(new_text);
        let start = self
            .text_input
            .preedit_start()
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
            self.text_input
                .set_preedit(start, CharRange::new(start, after.head));
            self.after_composition_change(&result, cx);
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
        let geometry = self.viewport.geometry()?;
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
        let Some(geometry) = self.viewport.geometry() else {
            return Vec::new();
        };
        let range = self.document.utf16_range_to_char(range_utf16);
        if range.is_empty() {
            return Vec::new();
        }
        let selected_first_line = self.document.line_of(range.start);
        let selected_last_line = self
            .document
            .line_of(CharOffset(range.end.0.saturating_sub(1)));
        let first_visible_row = self.viewport.scroll_row().floor().max(0.0) as usize;
        let last_visible_row = first_visible_row + geometry.visible_rows();
        let visible_first_line = self.line_at_row(first_visible_row);
        let visible_last_line = self.line_at_row(last_visible_row);
        let first_line = selected_first_line.0.max(visible_first_line.0);
        let last_line = selected_last_line.0.min(visible_last_line.0);
        let origin = geometry.text_origin();
        let mut rectangles = Vec::new();
        let mut walked = 0usize;
        let mut skipped = 0usize;
        let mut chars_read = 0usize;
        for line_number in first_line..=last_line {
            let line = LineIndex(line_number);
            let line_start = self.document.line_start(line);
            let source = Self::line_source(&self.document, line);
            let checkpoints = self.checkpoints_for(line);
            let line_prefix = self.viewport.row_prefix(line.0);
            let skip = first_visible_row.saturating_sub(line_prefix);
            let limit = last_visible_row
                .saturating_sub(line_prefix)
                .saturating_sub(skip)
                + 1;
            let (segments, stats) = chaus_layout::segments_window_source(
                &source,
                self.viewport.committed_columns(),
                self.settings.tab_stop,
                self.viewport.policy(),
                chaus_layout::WrapRowIndex::new(skip),
                limit,
                &checkpoints,
            );
            walked += stats.rows_walked;
            skipped += stats.rows_skipped_via_checkpoint;
            chars_read += stats.window_chars;
            for (index, segment) in segments.iter().enumerate() {
                let segment_start = CharOffset(line_start.0 + segment.char_start.get());
                let segment_end = CharOffset(line_start.0 + segment.char_end.get());
                let start = range.start.max(segment_start);
                let end = range.end.min(segment_end);
                if start >= end {
                    continue;
                }
                let row = line_prefix + skip + index;
                let y = origin.y
                    + gpui::px(
                        (row as f32 - self.viewport.scroll_row()) * f32::from(geometry.line_height),
                    );
                if y + geometry.line_height < element_bounds.top() || y > element_bounds.bottom() {
                    continue;
                }
                let columns = self.column_checkpoints_for(line);
                let absolute_start = chaus_layout::column_at_source(
                    &source,
                    chaus_layout::LineCharOffset::new(start.0.saturating_sub(line_start.0)),
                    self.settings.tab_stop,
                    &columns,
                )
                .max(segment.start_column);
                let absolute_end = if range.end >= segment_end {
                    segment.end_column()
                } else {
                    chaus_layout::column_at_source(
                        &source,
                        chaus_layout::LineCharOffset::new(end.0.saturating_sub(line_start.0)),
                        self.settings.tab_stop,
                        &columns,
                    )
                }
                .max(segment.start_column)
                .min(segment.end_column());
                let Some((start_column, end_column)) =
                    chaus_layout::local_selection_columns(segment, absolute_start, absolute_end)
                else {
                    continue;
                };
                let x = origin.x - self.viewport.scroll_x()
                    + gpui::px(start_column.get() as f32 * f32::from(geometry.char_width));
                let width = gpui::px(
                    (end_column.get() - start_column.get()) as f32 * f32::from(geometry.char_width),
                );
                rectangles.push(Bounds::from_corners(
                    Point { x, y },
                    Point {
                        x: x + width.max(gpui::px(1.0)),
                        y: y + geometry.line_height,
                    },
                ));
            }
        }
        self.viewport
            .record_layout_work(chars_read, 0, walked, skipped, 0);
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
        if self.modal_sheet_open() {
            return false;
        }
        // GPUI derives `prefers_ime_for_printable_keys` from this method, so
        // Normal mode sends printable keys through the command grammar while
        // Insert mode and standard editing defer to an active IME. The
        // command line takes text too, but routes it to its own buffer.
        !self.vim.enabled() || self.vim.accepts_text() || self.command_line_active()
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_sheet_open() {
            return;
        }
        if self.command_line_active() {
            if let Some(text) = item.text() {
                self.insert_command_text(&text, cx);
            }
            return;
        }
        if let Some(text) = item.text() {
            self.replace_selection(&text, EditCause::Paste, cx);
            window.notify_text_input_changed(&self.focus);
        }
    }
}

impl Editor {
    /// Whether a Vim `:`, `/` or `?` line owns typed text.
    pub(crate) fn command_line_active(&self) -> bool {
        self.vim.command_line_active()
    }

    /// Typed, pasted or IME-committed text for the command line. It never
    /// reaches the document or its history.
    pub(crate) fn insert_command_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.vim.set_command_preedit(None);
        if let chaus_vim::Outcome::Message(message) = self.vim.insert_command_text(text) {
            self.flash_status(message, cx);
        }
        cx.notify();
    }

    fn bounds_for_offset(&mut self, offset: CharOffset, geometry: Geometry) -> Point<Pixels> {
        let (line, row) = self.row_of_offset(offset);
        let line_start = self.document.line_start(line);
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let columns = self.column_checkpoints_for(line);
        let column = chaus_layout::column_at_source(
            &source,
            chaus_layout::LineCharOffset::new(offset.0.saturating_sub(line_start.0)),
            self.settings.tab_stop,
            &columns,
        );
        let local_row = self.local_row_of(offset, line);
        let scroll_x = self.viewport.scroll_x();
        let segment_start = if self.viewport.committed_columns().is_none() {
            // The unwrapped window starts at the horizontal scroll column;
            // asking layout for a row segment would materialize the line.
            (f32::from(scroll_x) / f32::from(geometry.char_width).max(1.0))
                .floor()
                .max(0.0) as usize
        } else {
            chaus_layout::segment_at_source(
                &source,
                self.viewport.committed_columns(),
                self.settings.tab_stop,
                self.viewport.policy(),
                chaus_layout::WrapRowIndex::new(local_row),
                &checkpoints,
            )
            .map_or(0, |segment| segment.start_column.get())
        };
        let local_column = column.get().saturating_sub(segment_start);
        let origin = geometry.text_origin();
        Point {
            x: origin.x - scroll_x + gpui::px(local_column as f32 * f32::from(geometry.char_width)),
            y: origin.y
                + gpui::px(
                    (row as f32 - self.viewport.scroll_row()) * f32::from(geometry.line_height),
                ),
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
