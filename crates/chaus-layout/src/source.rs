//! Bounded queries over Ropey-backed hard lines.
//!
//! Wrapped lines keep their row-checkpoint path. Unwrapped lines use a
//! separate horizontal path: there is exactly one row, so nothing is discovered
//! by walking it, and painting reads only the visible source window plus a
//! fixed overscan.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    column_from, piece_width, tab_width, ColumnCheckpoints, LineCharOffset, LineSource, RowLookup,
    Rows, Segment, VisualColumn, WindowStats, WrapCheckpoints, WrapPolicy, WrapRowIndex,
    WINDOW_CHARS, WINDOW_CONTEXT,
};

/// Char window `[start, start + window_chars)` clamped to the source.
pub(crate) fn window_span(
    source: &LineSource<'_>,
    start: usize,
    window_chars: usize,
) -> (usize, usize) {
    let end = (start + window_chars).min(source.char_len());
    (start, end)
}

/// Keep a grapheme at the window edge for the next window.
pub(crate) fn safe_window_end(source: &LineSource<'_>, start: usize, end: usize) -> usize {
    if end < source.char_len() {
        end.saturating_sub(WINDOW_CONTEXT).max(start)
    } else {
        end
    }
}

/// Rows `skip..skip + limit` from a Ropey-backed source, starting at the
/// nearest checkpoint instead of replaying earlier rows.
#[must_use]
pub fn segments_window_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    skip: WrapRowIndex,
    limit: usize,
    checkpoints: &WrapCheckpoints,
) -> (Vec<Segment>, WindowStats) {
    let mut stats = WindowStats::default();
    if limit == 0 {
        return (Vec::new(), stats);
    }
    if source.char_len() == 0 {
        return (
            (skip.get() == 0)
                .then(|| Segment {
                    display: String::new(),
                    char_start: LineCharOffset::new(0),
                    char_end: LineCharOffset::new(0),
                    start_column: VisualColumn::new(0),
                    columns: VisualColumn::new(0),
                })
                .into_iter()
                .collect(),
            stats,
        );
    }
    let (mut row_index, mut char_cursor, mut column) = checkpoints
        .nearest(skip)
        .map(|checkpoint| {
            stats.rows_skipped_via_checkpoint = checkpoint.row.get();
            (
                checkpoint.row.get(),
                checkpoint.char_offset.get(),
                checkpoint.column.get(),
            )
        })
        .unwrap_or((0, 0, 0));
    let end_row = skip.get().saturating_add(limit);
    let mut result = Vec::new();
    let mut window_chars = WINDOW_CHARS.max(max_columns.unwrap_or(1).saturating_mul(2));
    while char_cursor < source.char_len() && row_index < end_row && result.len() < limit {
        let (start, end) = window_span(source, char_cursor, window_chars);
        if end <= start {
            break;
        }
        stats.window_chars += end - start;
        stats.wrapped_chars += end - start;
        let safe_end = safe_window_end(source, start, end);
        let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
        let mut rows = Rows::new_with_column(&window, max_columns, tab_stop, policy, column);
        let mut next_char = char_cursor;
        let mut next_column = column;
        let mut carried = false;
        while let Some(row) = rows.next() {
            if row.char_start == row.char_end && row.columns == 0 {
                continue;
            }
            let absolute_end = start + row.char_end.get();
            if row.char_end.get() == 0 || absolute_end > safe_end {
                carried = true;
                break;
            }
            stats.rows_walked += 1;
            if row_index >= skip.get() && result.len() < limit {
                let display = rows.display_for_last_row(row.start_column);
                result.push(Segment {
                    display,
                    char_start: LineCharOffset::new(start + row.char_start.get()),
                    char_end: LineCharOffset::new(absolute_end),
                    start_column: row.start_column,
                    columns: row.columns,
                });
            }
            row_index += 1;
            next_char = absolute_end;
            next_column = row.end_column().get();
        }
        if next_char <= char_cursor {
            if !carried || end >= source.char_len() {
                break;
            }
            // A single row is wider than the safe window: grow it and retry.
            window_chars = window_chars.saturating_mul(2).max(WINDOW_CHARS);
            continue;
        }
        char_cursor = next_char;
        column = next_column;
    }
    (result, stats)
}

/// One visual row from a Ropey-backed source, using checkpoints.
#[must_use]
pub fn segment_at_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    row: WrapRowIndex,
    checkpoints: &WrapCheckpoints,
) -> Option<Segment> {
    segments_window_source(source, max_columns, tab_stop, policy, row, 1, checkpoints)
        .0
        .into_iter()
        .next()
}

/// Visual row count from a Ropey-backed source without building display text.
///
/// An unwrapped hard line is one row by definition, so the path returns
/// immediately without scanning or materializing any source.
#[must_use]
pub fn row_count_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    checkpoints: &WrapCheckpoints,
) -> usize {
    if source.char_len() == 0 || max_columns.is_none() {
        return 1;
    }
    let mut count = 0usize;
    let mut char_cursor = 0usize;
    let mut column = 0usize;
    let mut window_chars = WINDOW_CHARS.max(max_columns.unwrap_or(1).saturating_mul(2));
    while char_cursor < source.char_len() {
        let (start, end) = window_span(source, char_cursor, window_chars);
        if end <= start {
            break;
        }
        let safe_end = safe_window_end(source, start, end);
        let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
        let rows = Rows::new_with_column(&window, max_columns, tab_stop, policy, column);
        let mut next_char = char_cursor;
        let mut next_column = column;
        let mut progressed = false;
        for row in rows {
            if row.char_start == row.char_end && row.columns == 0 {
                continue;
            }
            let absolute_end = start + row.char_end.get();
            if row.char_end.get() == 0 || absolute_end > safe_end {
                break;
            }
            count += 1;
            next_char = absolute_end;
            next_column = row.end_column().get();
            progressed = true;
        }
        if !progressed {
            if end >= source.char_len() {
                break;
            }
            window_chars = window_chars.saturating_mul(2).max(WINDOW_CHARS);
            continue;
        }
        if next_char <= char_cursor {
            break;
        }
        char_cursor = next_char;
        column = next_column;
    }
    let _ = checkpoints;
    count
}

/// Locate a char offset in a Ropey-backed source, starting at a checkpoint.
///
/// An unwrapped line has a single row; the offset is not looked up by walking.
#[must_use]
pub fn row_of_char_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    char_offset: LineCharOffset,
    checkpoints: &WrapCheckpoints,
) -> RowLookup {
    if max_columns.is_none() {
        return RowLookup {
            row: WrapRowIndex::new(0),
            rows_visited: 1,
        };
    }
    let (mut row_index, mut char_cursor, mut column) = checkpoints
        .nearest(WrapRowIndex::new(usize::MAX))
        .map(|checkpoint| {
            (
                checkpoint.row.get(),
                checkpoint.char_offset.get(),
                checkpoint.column.get(),
            )
        })
        .unwrap_or((0, 0, 0));
    // Pick the last checkpoint before the target char, not merely the last
    // row checkpoint, so a deep offset does not replay from row zero.
    if let Some(checkpoint) = checkpoints
        .entries()
        .iter()
        .take_while(|entry| entry.char_offset <= char_offset)
        .last()
    {
        row_index = checkpoint.row.get();
        char_cursor = checkpoint.char_offset.get();
        column = checkpoint.column.get();
    }
    let mut visited = 0usize;
    let mut window_chars = WINDOW_CHARS.max(max_columns.unwrap_or(1).saturating_mul(2));
    while char_cursor < source.char_len() {
        let (start, end) = window_span(source, char_cursor, window_chars);
        if end <= start {
            break;
        }
        let safe_end = safe_window_end(source, start, end);
        let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
        let mut rows = Rows::new_with_column(&window, max_columns, tab_stop, policy, column);
        let mut next_char = char_cursor;
        let mut next_column = column;
        let mut carried = false;
        for row in rows.by_ref() {
            if row.char_start == row.char_end && row.columns == 0 {
                continue;
            }
            let absolute_end = start + row.char_end.get();
            if row.char_end.get() == 0 || absolute_end > safe_end {
                carried = true;
                break;
            }
            visited += 1;
            if char_offset.get() < absolute_end {
                return RowLookup {
                    row: WrapRowIndex::new(row_index),
                    rows_visited: visited,
                };
            }
            row_index += 1;
            next_char = absolute_end;
            next_column = row.end_column().get();
        }
        if next_char <= char_cursor {
            if !carried || end >= source.char_len() {
                break;
            }
            window_chars = window_chars.saturating_mul(2).max(WINDOW_CHARS);
            continue;
        }
        char_cursor = next_char;
        column = next_column;
    }
    RowLookup {
        row: WrapRowIndex::new(row_index.saturating_sub(1)),
        rows_visited: visited,
    }
}

/// Column of a char offset through a bounded window of a Ropey-backed source.
///
/// The search starts at the nearest scalar checkpoint at or before the
/// requested offset, so a deep offset does not replay the whole line.
#[must_use]
pub fn column_at_source(
    source: &LineSource<'_>,
    char_offset: LineCharOffset,
    tab_stop: usize,
    checkpoints: &ColumnCheckpoints,
) -> VisualColumn {
    let checkpoint = checkpoints.nearest_scalar(char_offset);
    let (start, column) = checkpoint.map_or((0usize, 0usize), |entry| {
        (entry.char_offset.get(), entry.column.get())
    });
    let end = (char_offset.get() + WINDOW_CONTEXT)
        .min(source.char_len())
        .max(start);
    let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    column_from(
        &window,
        LineCharOffset::new(char_offset.get().saturating_sub(start)),
        tab_stop,
        VisualColumn::new(column),
    )
}

/// Nearest char offset at or before a column through a Ropey-backed source.
///
/// The search starts at the nearest column checkpoint and continues across
/// as many bounded windows as needed. It never silently stops because one
/// window ended: reaching EOF is the only terminal condition besides a match.
#[must_use]
pub fn char_at_column_source(
    source: &LineSource<'_>,
    column: VisualColumn,
    tab_stop: usize,
    checkpoints: &ColumnCheckpoints,
) -> LineCharOffset {
    let (mut char_cursor, mut current_column) = checkpoints
        .nearest_column(column)
        .map_or((0usize, 0usize), |entry| {
            (entry.char_offset.get(), entry.column.get())
        });
    loop {
        if char_cursor >= source.char_len() {
            return LineCharOffset::new(source.char_len());
        }
        let (start, end) = window_span(source, char_cursor, WINDOW_CHARS);
        let safe_end = safe_window_end(source, start, end);
        let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
        let local_end = safe_end.saturating_sub(start);
        let mut index = 0usize;
        let mut cursor_column = current_column;
        let mut found = None;
        for grapheme in window.graphemes(true) {
            if index >= local_end {
                break;
            }
            let width = if grapheme == "\t" {
                tab_width(cursor_column, tab_stop)
            } else {
                grapheme.width()
            };
            if cursor_column.saturating_add(width) > column.get() {
                found = Some(char_cursor + index);
                break;
            }
            cursor_column += width;
            index += grapheme.chars().count();
        }
        if let Some(offset) = found {
            return LineCharOffset::new(offset.min(source.char_len()));
        }
        let next = char_cursor + index;
        if next <= char_cursor {
            if end >= source.char_len() {
                return LineCharOffset::new(source.char_len());
            }
            // A zero-width grapheme run cannot advance by itself; take the
            // whole window so the next iteration still makes progress.
            let next = end;
            char_cursor = next;
            continue;
        }
        char_cursor = next;
        current_column = cursor_column;
    }
}

/// Total display columns of a hard line, computed from the last horizontal
/// checkpoint plus one bounded window. No whole-line materialization.
#[must_use]
pub fn line_columns(
    source: &LineSource<'_>,
    tab_stop: usize,
    checkpoints: &ColumnCheckpoints,
) -> VisualColumn {
    if source.char_len() == 0 {
        return VisualColumn::new(0);
    }
    let (start, column) = checkpoints
        .entries()
        .last()
        .map_or((0usize, 0usize), |entry| {
            (
                entry.char_offset.get().min(source.char_len()),
                entry.column.get(),
            )
        });
    if start >= source.char_len() {
        return VisualColumn::new(column);
    }
    let end = (start + WINDOW_CHARS).min(source.char_len());
    let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    column_from(
        &window,
        LineCharOffset::new(source.char_len() - start),
        tab_stop,
        VisualColumn::new(column),
    )
}

/// The bounded source window required to paint part of an unwrapped line.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwrappedWindow<'a> {
    /// Raw source text of the window, grapheme-safe at both ends.
    pub text: Cow<'a, str>,
    /// Absolute scalar offset of `text` within the hard line.
    pub char_start: LineCharOffset,
    /// Absolute scalar offset one past `text`.
    pub char_end: LineCharOffset,
    /// Absolute display column at `char_start`, tabs expanded.
    pub start_column: VisualColumn,
    /// Tab-expanded display text to hand to the shaper.
    pub display: String,
    /// Source chars materialized for this window, for diagnostics.
    pub source_chars: usize,
}

impl UnwrappedWindow<'_> {
    /// A segment view whose local offsets start at zero.
    ///
    /// Tab expansion already used the absolute `start_column`, so syntax runs
    /// must also be computed with that column, not with `Segment::start_column`.
    #[must_use]
    pub fn as_segment(&self) -> Segment {
        Segment {
            display: self.display.clone(),
            char_start: LineCharOffset::new(0),
            char_end: LineCharOffset::new(self.text.chars().count()),
            start_column: VisualColumn::new(0),
            columns: VisualColumn::new(self.display.chars().count()),
        }
    }
}

/// Window of an unwrapped hard line covering a visible horizontal range.
///
/// `first_column` and `visible_columns` describe the viewport; `overscan`
/// keeps a fixed, bounded margin so small scrolls do not reshape every glyph.
#[must_use]
pub fn unwrapped_window<'a>(
    source: &LineSource<'a>,
    first_column: VisualColumn,
    visible_columns: VisualColumn,
    overscan_columns: VisualColumn,
    tab_stop: usize,
    checkpoints: &ColumnCheckpoints,
) -> UnwrappedWindow<'a> {
    let before = first_column.get().saturating_sub(overscan_columns.get());
    let start = char_at_column_source(source, VisualColumn::new(before), tab_stop, checkpoints)
        .get()
        .min(source.char_len());
    let after = first_column
        .get()
        .saturating_add(visible_columns.get())
        .saturating_add(overscan_columns.get());
    let end = char_at_column_source(source, VisualColumn::new(after), tab_stop, checkpoints)
        .get()
        .saturating_add(WINDOW_CONTEXT)
        .min(source.char_len())
        .max(start);
    let text = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    let start_column = column_at_source(source, LineCharOffset::new(start), tab_stop, checkpoints);
    let mut display = String::with_capacity(text.len());
    let mut column = start_column.get();
    for grapheme in text.graphemes(true) {
        let width = piece_width(grapheme, column, tab_stop);
        if grapheme == "\t" {
            display.extend(std::iter::repeat_n(' ', width));
        } else {
            display.push_str(grapheme);
        }
        column += width;
    }
    UnwrappedWindow {
        text,
        char_start: LineCharOffset::new(start),
        char_end: LineCharOffset::new(end),
        start_column,
        display,
        source_chars: end - start,
    }
}
