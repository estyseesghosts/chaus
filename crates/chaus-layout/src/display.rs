//! Source-to-display mapping: tabs, grapheme boundaries and byte offsets.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{DisplayByteOffset, LineCharOffset, LineSource, Segment, VisualColumn};

/// One grapheme boundary within a segment's rendered text.
///
/// `char_offset` is the source offset within the hard line; `display_byte` is
/// the UTF-8 byte offset within [`Segment::display`], where a tab has already
/// become spaces. The first entry starts at `(segment.char_start, 0)` and the
/// last ends at `(segment.char_end, segment.display.len())`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBoundary {
    /// Unicode scalar offset within the hard line.
    pub char_offset: LineCharOffset,
    /// UTF-8 byte offset within the expanded display text.
    pub display_byte: DisplayByteOffset,
}

/// Map every grapheme boundary in `segment` to its displayed byte offset.
///
/// This is the authoritative source-to-display mapping used by painting. It
/// reuses the same grapheme segmentation, width measurement and logical-line
/// tab stops as [`crate::segments_with`], so run lengths can never disagree
/// with the display string. Callers must pass the same `line` the segment came
/// from and the same `tab_stop`; previewing a segment under different settings
/// is not a supported use.
#[must_use]
pub fn display_boundaries(line: &str, segment: &Segment, tab_stop: usize) -> Vec<DisplayBoundary> {
    display_boundaries_from_column(line, segment, tab_stop, VisualColumn::new(0))
}

/// [`display_boundaries`] starting from a known column at `segment.char_start`.
///
/// Use this when only a bounded window of the hard line is available: the
/// column recorded at the segment start is used, so a tab in the window still
/// expands against its logical-line tab stop.
#[must_use]
pub fn display_boundaries_from_column(
    line: &str,
    segment: &Segment,
    tab_stop: usize,
    start_column: VisualColumn,
) -> Vec<DisplayBoundary> {
    let tab_stop = tab_stop.max(1);
    let mut boundaries = Vec::new();
    boundaries.push(DisplayBoundary {
        char_offset: segment.char_start,
        display_byte: DisplayByteOffset::new(0),
    });
    let mut char_index = 0usize;
    let mut column = start_column.get();
    let mut display_byte = 0usize;
    for grapheme in line.graphemes(true) {
        let char_end = char_index + grapheme.chars().count();
        if char_end <= segment.char_start.get() {
            column += piece_width(grapheme, column, tab_stop);
            char_index = char_end;
            continue;
        }
        if char_index >= segment.char_end.get() {
            break;
        }
        // `segments_with` only breaks at grapheme boundaries, so a segment
        // never begins inside this grapheme.
        debug_assert!(
            char_index >= segment.char_start.get(),
            "segment {:?} starts inside the grapheme at char {char_index}",
            (segment.char_start, segment.char_end)
        );
        let width = piece_width(grapheme, column, tab_stop);
        if grapheme == "\t" {
            display_byte += width;
        } else {
            display_byte += grapheme.len();
        }
        column += width;
        char_index = char_end;
        boundaries.push(DisplayBoundary {
            char_offset: LineCharOffset::new(char_end),
            display_byte: DisplayByteOffset::new(display_byte),
        });
    }
    debug_assert_eq!(
        boundaries
            .last()
            .map(|boundary| boundary.display_byte.get()),
        Some(segment.display.len()),
        "display mapping must cover the whole segment"
    );
    boundaries
}

/// Display boundaries for a segment held in a Ropey-backed [`LineSource`].
///
/// Only the segment's own grapheme-safe range is materialized, and the column
/// is taken from the segment itself. There is no separate window-relative
/// start offset that could disagree with the column.
#[must_use]
pub fn display_boundaries_source(
    source: &LineSource<'_>,
    segment: &Segment,
    tab_stop: usize,
) -> Vec<DisplayBoundary> {
    let start = segment.char_start.get().min(source.char_len());
    let end = segment.char_end.get().min(source.char_len()).max(start);
    let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    let mut local = segment.clone();
    local.char_start = LineCharOffset::new(0);
    local.char_end = LineCharOffset::new(window.chars().count());
    let boundaries =
        display_boundaries_from_column(&window, &local, tab_stop, segment.start_column);
    boundaries
        .into_iter()
        .map(|boundary| DisplayBoundary {
            char_offset: LineCharOffset::new(start + boundary.char_offset.get()),
            display_byte: boundary.display_byte,
        })
        .collect()
}

/// Clamp absolute logical-line selection columns to this segment and return
/// segment-local columns. Returns `None` when the selection does not cover any
/// cell on this visual row.
#[must_use]
pub fn local_selection_columns(
    segment: &Segment,
    start_column: impl Into<VisualColumn>,
    end_column: impl Into<VisualColumn>,
) -> Option<(VisualColumn, VisualColumn)> {
    let start_column = start_column.into().get();
    let end_column = end_column.into().get();
    let start = start_column
        .max(segment.start_column.get())
        .min(segment.end_column().get())
        .saturating_sub(segment.start_column.get());
    let end = end_column
        .max(segment.start_column.get())
        .min(segment.end_column().get())
        .saturating_sub(segment.start_column.get());
    (end > start).then(|| (VisualColumn::new(start), VisualColumn::new(end)))
}

/// Columns a tab occupies from `column` to the next tab stop.
pub(crate) fn tab_width(column: usize, tab_stop: usize) -> usize {
    let tab_stop = tab_stop.max(1);
    tab_stop - (column % tab_stop)
}

/// Display width of one grapheme at an absolute logical-line column.
pub(crate) fn piece_width(grapheme: &str, column: usize, tab_stop: usize) -> usize {
    if grapheme == "\t" {
        tab_width(column, tab_stop)
    } else {
        grapheme.width()
    }
}
