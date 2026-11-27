//! Soft-wrap geometry independent of GPUI.
//!
//! A hard line is split into visual rows at either a Unicode line-break
//! opportunity (the default, word-preferred policy) or the furthest grapheme
//! that fits. Tabs expand to the next tab stop of the **logical line**, wide
//! CJK/emoji graphemes occupy two columns and combining sequences/ZWJ emoji
//! stay attached to their base. One shared row walker drives [`segments`],
//! [`row_count`], caret placement, hit testing and selection, so those can
//! never disagree.
//!
//! [`LineSource`] is the Ropey-friendly input to that walker. A caller with a
//! `RopeSlice` asks for a bounded [`LineSource::window`] instead of
//! materializing a whole hard line. Wrapped rows use [`WrapCheckpoints`];
//! unwrapped horizontal mapping uses the separate [`ColumnCheckpoints`] and
//! [`unwrapped_window`]. No display string is cached in either index.
//!
//! This crate owns no editor UI geometry. Column and row values are distinct
//! newtypes; conversion to `usize` happens only at external API boundaries.

mod checkpoints;
mod display;
mod line_source;
mod rows;
mod source;
mod units;

pub use checkpoints::{
    build_checkpoints, build_column_checkpoints, ColumnCheckpoint, ColumnCheckpoints,
    RowCheckpoint, WindowStats, WrapCheckpoints,
};
pub use display::{
    display_boundaries, display_boundaries_from_column, display_boundaries_source,
    local_selection_columns, DisplayBoundary,
};
pub use line_source::LineSource;
pub use rows::{
    char_at_column, column_at, row_count, row_count_with, row_of_char, row_of_char_with_stats,
    rows, segments, segments_window, segments_with, RowLookup, Rows, Segment, VisualRow,
    WrapPolicy,
};
pub use source::{
    char_at_column_source, column_at_source, line_columns, row_count_source, row_of_char_source,
    segment_at_source, segments_window_source, unwrapped_window, UnwrappedWindow,
};
pub use units::{
    DisplayByteOffset, LineCharOffset, RowColumn, VisualColumn, WrapRowIndex, WrapWidthColumns,
};

pub(crate) use rows::column_from;

/// Chars fetched at a time from a non-contiguous [`LineSource`].
///
/// A window plus its context gets at most this many chars; a row that does not
/// fit is allowed to grow the window, but a document is never flattened.
pub(crate) const WINDOW_CHARS: usize = 4096;

/// Chars of lookahead kept so a grapheme at a window edge is never cut.
pub(crate) const WINDOW_CONTEXT: usize = 128;

/// Width of the whitespace-free horizontal overscan used by painting.
pub const OVERSCAN_COLUMNS: usize = 64;

/// Scalar stride at which horizontal column checkpoints are recorded.
pub const COLUMN_CHECKPOINT_STRIDE: usize = 4096;

/// Columns a tab occupies from `column` to the next tab stop.
pub(crate) fn tab_width(column: usize, tab_stop: usize) -> usize {
    display::tab_width(column, tab_stop)
}

/// Display width of one grapheme at an absolute logical-line column.
pub(crate) fn piece_width(grapheme: &str, column: usize, tab_stop: usize) -> usize {
    display::piece_width(grapheme, column, tab_stop)
}

pub(crate) fn window_span(
    source: &LineSource<'_>,
    start: usize,
    window_chars: usize,
) -> (usize, usize) {
    source::window_span(source, start, window_chars)
}

pub(crate) fn safe_window_end(source: &LineSource<'_>, start: usize, end: usize) -> usize {
    source::safe_window_end(source, start, end)
}
