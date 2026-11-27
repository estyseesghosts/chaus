//! Numeric checkpoints for wrapped rows and horizontal column mapping.
//!
//! A wrapped-row checkpoint and a horizontal column checkpoint are different
//! abstractions. The first says where a visual row begins; the second says
//! which source scalar and absolute display column belong to a horizontal
//! position. Keeping them separate prevents row geometry from being mistaken
//! for a source-to-column index on an unwrapped line.

use crate::{
    column_from, safe_window_end, window_span, LineCharOffset, LineSource, Rows, VisualColumn,
    WINDOW_CHARS, WINDOW_CONTEXT,
};

/// Numeric index into a wrapped line, used to restart the walker without
/// replaying earlier rows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowCheckpoint {
    /// Visual row this checkpoint describes.
    pub row: crate::WrapRowIndex,
    /// Char offset of that row's start within the hard line.
    pub char_offset: LineCharOffset,
    /// Display column at that row's start, tabs expanded.
    pub column: VisualColumn,
}

/// Bounded, numeric-only checkpoints for deep wrapped rows.
///
/// No display text is cached: each entry is three integers, so a multi-MiB
/// line with thousands of rows keeps a small index rather than a per-row
/// `String` cache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrapCheckpoints {
    stride: usize,
    entries: Vec<RowCheckpoint>,
}

impl WrapCheckpoints {
    /// One checkpoint every `stride` visual rows. A zero stride becomes one.
    #[must_use]
    pub fn new(stride: usize) -> Self {
        Self {
            stride: stride.max(1),
            entries: Vec::new(),
        }
    }

    /// Number of stored checkpoints.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no checkpoints have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop every checkpoint. Called when text or wrap inputs change.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Checkpoint at or before `row`, if one exists.
    #[must_use]
    pub fn nearest(&self, row: crate::WrapRowIndex) -> Option<&RowCheckpoint> {
        self.entries
            .iter()
            .take_while(|entry| entry.row <= row)
            .last()
    }

    /// Record a checkpoint. Callers pass rows in increasing order.
    pub fn push(&mut self, entry: RowCheckpoint) {
        if entry.row.get().is_multiple_of(self.stride.max(1)) {
            self.entries.push(entry);
        }
    }

    /// Stride used when building these checkpoints.
    #[must_use]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Recorded entries, in increasing row order.
    #[must_use]
    pub fn entries(&self) -> &[RowCheckpoint] {
        &self.entries
    }
}

/// A horizontal position in an unwrapped hard line.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnCheckpoint {
    /// Unicode scalar offset within the hard line.
    pub char_offset: LineCharOffset,
    /// Absolute display column at that scalar, tabs expanded.
    pub column: VisualColumn,
}

/// Bounded horizontal index for unwrapped lines.
///
/// Entries contain only a scalar offset and an absolute display column. No
/// display string is retained; the index is grown at a fixed scalar stride
/// from bounded source windows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnCheckpoints {
    stride: usize,
    entries: Vec<ColumnCheckpoint>,
}

impl ColumnCheckpoints {
    /// One checkpoint every `stride` source scalars. A zero stride becomes
    /// one, so every scalar is a checkpoint.
    #[must_use]
    pub fn new(stride: usize) -> Self {
        Self {
            stride: stride.max(1),
            entries: Vec::new(),
        }
    }

    /// Number of stored checkpoints.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no checkpoints have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop every checkpoint. Called when text or tab settings change.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Checkpoint at or before `offset`, if one exists.
    #[must_use]
    pub fn nearest_scalar(&self, offset: LineCharOffset) -> Option<&ColumnCheckpoint> {
        self.entries
            .iter()
            .take_while(|entry| entry.char_offset <= offset)
            .last()
    }

    /// Checkpoint at or before `column`, if one exists.
    #[must_use]
    pub fn nearest_column(&self, column: VisualColumn) -> Option<&ColumnCheckpoint> {
        self.entries
            .iter()
            .take_while(|entry| entry.column <= column)
            .last()
    }

    /// Record a checkpoint. Callers pass offsets in increasing order.
    pub fn push(&mut self, entry: ColumnCheckpoint) {
        if entry.char_offset.get().is_multiple_of(self.stride.max(1)) {
            self.entries.push(entry);
        }
    }

    /// Record a checkpoint without the stride filter.
    ///
    /// The builder advances by a fixed *window* stride whose grapheme-safe end
    /// is deliberately offset from the raw window size, so those offsets would
    /// not pass `push`'s modulus test.
    pub fn push_window_end(&mut self, entry: ColumnCheckpoint) {
        self.entries.push(entry);
    }

    /// Stride used when building these checkpoints.
    #[must_use]
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Recorded entries, in increasing scalar order.
    #[must_use]
    pub fn entries(&self) -> &[ColumnCheckpoint] {
        &self.entries
    }
}

/// Work performed by a bounded source-window query, for diagnostics.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowStats {
    /// Rows visited by the shared walker.
    pub rows_walked: usize,
    /// Rows skipped by starting from a checkpoint.
    pub rows_skipped_via_checkpoint: usize,
    /// Chars materialized in all windows combined.
    pub window_chars: usize,
    /// Source chars materialized on the wrapped-row path.
    pub wrapped_chars: usize,
    /// Source chars materialized on the unwrapped horizontal path.
    pub unwrapped_chars: usize,
}

/// Build numeric wrapped-row checkpoints from a Ropey-backed source.
///
/// An unwrapped hard line has exactly one visual row, so no row checkpoints
/// are built and the whole line is never scanned. Horizontal mapping uses
/// [`ColumnCheckpoints`] instead.
#[must_use]
pub fn build_checkpoints(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: crate::WrapPolicy,
    stride: usize,
) -> WrapCheckpoints {
    let mut checkpoints = WrapCheckpoints::new(stride);
    if source.char_len() == 0 || max_columns.is_none() {
        return checkpoints;
    }
    let mut window_chars = WINDOW_CHARS.max(max_columns.unwrap_or(1).saturating_mul(2));
    let mut char_cursor = 0usize;
    let mut column = 0usize;
    let mut row_index = 0usize;
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
        let mut progressed = false;
        for row in rows.by_ref() {
            if row.char_start == row.char_end && row.columns == 0 {
                continue;
            }
            let absolute_end = start + row.char_end.get();
            if row.char_end.get() == 0 || absolute_end > safe_end {
                break;
            }
            if row_index.is_multiple_of(checkpoints.stride()) {
                checkpoints.push(RowCheckpoint {
                    row: crate::WrapRowIndex::new(row_index),
                    char_offset: LineCharOffset::new(start + row.char_start.get()),
                    column: row.start_column,
                });
            }
            row_index += 1;
            next_char = absolute_end;
            next_column = row.end_column().get();
            progressed = true;
        }
        if !progressed {
            window_chars = window_chars.saturating_mul(2).max(WINDOW_CHARS);
            if end >= source.char_len() {
                break;
            }
            continue;
        }
        if next_char <= char_cursor {
            break;
        }
        char_cursor = next_char;
        column = next_column;
    }
    checkpoints
}

/// Build a horizontal index at a fixed scalar stride.
///
/// Each step reads a bounded window of [`WINDOW_CHARS`] scalars and keeps only
/// the scalar/column pair at the window's grapheme-safe end. Display strings
/// are never retained.
#[must_use]
pub fn build_column_checkpoints(
    source: &LineSource<'_>,
    tab_stop: usize,
    stride: usize,
) -> ColumnCheckpoints {
    let mut checkpoints = ColumnCheckpoints::new(stride.max(WINDOW_CONTEXT).max(1));
    if source.char_len() == 0 {
        return checkpoints;
    }
    checkpoints.push(ColumnCheckpoint {
        char_offset: LineCharOffset::new(0),
        column: VisualColumn::new(0),
    });
    let mut char_cursor = 0usize;
    let mut column = 0usize;
    while char_cursor < source.char_len() {
        let (start, end) = window_span(source, char_cursor, WINDOW_CHARS);
        if end <= start {
            break;
        }
        let safe_end = safe_window_end(source, start, end);
        let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
        let local_end = safe_end.saturating_sub(start);
        column = column_from(
            &window,
            LineCharOffset::new(local_end),
            tab_stop,
            VisualColumn::new(column),
        )
        .get();
        if safe_end <= char_cursor {
            break;
        }
        char_cursor = safe_end;
        checkpoints.push_window_end(ColumnCheckpoint {
            char_offset: LineCharOffset::new(char_cursor),
            column: VisualColumn::new(column),
        });
    }
    checkpoints
}
