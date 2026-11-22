//! Soft-wrap geometry independent of GPUI.
//!
//! A hard line is split into visual rows at either a Unicode line-break
//! opportunity (the default, word-preferred policy) or the furthest grapheme
//! that fits. Tabs expand to the next tab stop, wide CJK/emoji graphemes occupy
//! two columns and combining sequences/zero-width joiners stay attached to
//! their base. One shared row walker drives [`segments`], [`row_count`], caret
//! placement, hit testing and selection, so those can never disagree.
//!
//! Tab stops are measured from the start of the logical line, never from the
//! start of a visual row. Resizing the window therefore changes where a line
//! wraps but never how wide a tab is, and [`column_at`] stays consistent with
//! the columns recorded in each [`Segment`]. A grapheme or an expanded tab is
//! never split across rows; when a tab does not fit it moves whole to the next
//! row, which can leave that row one tab stop wider than requested.
//!
//! Wrapping is visual only: the source text and every char offset are kept
//! exactly as they were.

use std::collections::VecDeque;

use unicode_linebreak::linebreaks;
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

/// How a hard line is split when it does not fit the available width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapPolicy {
    /// Break at the furthest grapheme that fits.
    Character,
    /// Break at the last Unicode line-break opportunity that fits, and fall
    /// back to grapheme boundaries when no opportunity fits (an oversized
    /// token, URL or path).
    Word,
}

/// One visual row of a hard line.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Text to shape, with tabs replaced by spaces. Contains no newline.
    pub display: String,
    /// Char offset of the segment start within the hard line.
    pub char_start: usize,
    /// Char offset one past the segment end within the hard line.
    pub char_end: usize,
    /// Visual columns before this segment (wrapping included).
    pub start_column: usize,
    /// Number of visual columns covered by this segment.
    pub columns: usize,
}

impl Segment {
    #[must_use]
    pub fn end_column(&self) -> usize {
        self.start_column + self.columns
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.char_start == self.char_end
    }
}

/// One row produced by [`rows`] without allocating its display string.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualRow {
    /// Char offset of the row start within the hard line.
    pub char_start: usize,
    /// Char offset one past the row end within the hard line.
    pub char_end: usize,
    /// Visual columns before this row (wrapping included).
    pub start_column: usize,
    /// Number of visual columns covered by this row.
    pub columns: usize,
    grapheme_start: usize,
    grapheme_end: usize,
}

impl VisualRow {
    #[must_use]
    pub fn end_column(&self) -> usize {
        self.start_column + self.columns
    }
}

/// One grapheme boundary within a segment's rendered text.
///
/// `char_offset` is the source offset within the hard line; `display_byte` is
/// the UTF-8 byte offset within [`Segment::display`], where a tab has already
/// become spaces. The first entry starts at `(segment.char_start, 0)` and the
/// last ends at `(segment.char_end, segment.display.len())`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBoundary {
    pub char_offset: usize,
    pub display_byte: usize,
}

/// Map every grapheme boundary in `segment` to its displayed byte offset.
///
/// This is the authoritative source-to-display mapping used by painting. It
/// reuses the same grapheme segmentation, width measurement and logical-line
/// tab stops as [`segments_with`], so run lengths can never disagree with the
/// display string. Callers must pass the same `line` the segment came from and
/// the same `tab_stop`; previewing a segment under different settings is not a
/// supported use.
#[must_use]
pub fn display_boundaries(line: &str, segment: &Segment, tab_stop: usize) -> Vec<DisplayBoundary> {
    let tab_stop = tab_stop.max(1);
    let mut boundaries = Vec::new();
    boundaries.push(DisplayBoundary {
        char_offset: segment.char_start,
        display_byte: 0,
    });
    let mut char_index = 0usize;
    let mut column = 0usize;
    let mut display_byte = 0usize;
    for grapheme in line.graphemes(true) {
        let char_end = char_index + grapheme.chars().count();
        if char_end <= segment.char_start {
            column += piece_width(grapheme, column, tab_stop);
            char_index = char_end;
            continue;
        }
        if char_index >= segment.char_end {
            break;
        }
        // `segments_with` only breaks at grapheme boundaries, so a segment
        // never begins inside this grapheme.
        debug_assert!(
            char_index >= segment.char_start,
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
            char_offset: char_end,
            display_byte,
        });
    }
    debug_assert_eq!(
        boundaries.last().map(|boundary| boundary.display_byte),
        Some(segment.display.len()),
        "display mapping must cover the whole segment"
    );
    boundaries
}

/// Clamp absolute logical-line selection columns to this segment and return
/// segment-local columns. Returns `None` when the selection does not cover any
/// cell on this visual row.
#[must_use]
pub fn local_selection_columns(
    segment: &Segment,
    start_column: usize,
    end_column: usize,
) -> Option<(usize, usize)> {
    let start = start_column
        .max(segment.start_column)
        .min(segment.end_column())
        .saturating_sub(segment.start_column);
    let end = end_column
        .max(segment.start_column)
        .min(segment.end_column())
        .saturating_sub(segment.start_column);
    (end > start).then_some((start, end))
}

fn tab_width(column: usize, tab_stop: usize) -> usize {
    let tab_stop = tab_stop.max(1);
    tab_stop - (column % tab_stop)
}

fn piece_width(grapheme: &str, column: usize, tab_stop: usize) -> usize {
    if grapheme == "\t" {
        tab_width(column, tab_stop)
    } else {
        grapheme.width()
    }
}

/// Preferred floating find-bar width for a window of `available_width`.
///
/// The bar keeps a 10px inset on both sides and never grows past 470 logical
/// pixels, so the 420px minimum window always leaves usable fields.
#[must_use]
pub fn find_bar_width(available_width: f32) -> f32 {
    (available_width - 20.0).clamp(0.0, 470.0)
}

#[derive(Clone, Copy)]
struct Grapheme<'a> {
    text: &'a str,
    char_start: usize,
    char_end: usize,
    /// A Unicode line-break opportunity immediately before this grapheme.
    break_before: bool,
}

/// Iterator over the visual rows of one hard line.
///
/// Graphemes are pulled from the source one at a time and kept in a small
/// buffer only for the row being decided, so a multi-MiB unwrapped line does
/// not allocate a vector proportional to its length.
pub struct Rows<'a> {
    source: GraphemeIndices<'a>,
    breaks: Box<dyn Iterator<Item = usize> + 'a>,
    next_break: Option<usize>,
    /// Graphemes of the row most recently returned stay at the front until the
    /// next call drains them, so [`Rows::display_for_last_row`] can build the
    /// display string without copying.
    buffer: VecDeque<Grapheme<'a>>,
    consumed: usize,
    char_cursor: usize,
    policy: WrapPolicy,
    max_columns: Option<usize>,
    tab_stop: usize,
    column: usize,
    source_done: bool,
    finished: bool,
}

impl<'a> Rows<'a> {
    fn new(text: &'a str, max_columns: Option<usize>, tab_stop: usize, policy: WrapPolicy) -> Self {
        let max_columns = max_columns.filter(|columns| *columns > 0);
        let mut breaks: Box<dyn Iterator<Item = usize> + 'a> =
            Box::new(linebreaks(text).map(|(byte, _)| byte));
        let next_break = breaks.next();
        Self {
            source: text.grapheme_indices(true),
            breaks,
            next_break,
            buffer: VecDeque::new(),
            consumed: 0,
            char_cursor: 0,
            policy,
            max_columns,
            tab_stop,
            column: 0,
            source_done: false,
            finished: false,
        }
    }

    /// Pull one grapheme and attach its line-break opportunity.
    fn pull(&mut self) -> Option<Grapheme<'a>> {
        if self.source_done {
            return None;
        }
        let Some((byte_start, text)) = self.source.next() else {
            self.source_done = true;
            return None;
        };
        // `linebreaks` yields increasing byte offsets, the same order as the
        // graphemes, so one forward cursor replaces a byte->char table.
        while self.next_break.is_some_and(|at| at < byte_start) {
            self.next_break = self.breaks.next();
        }
        let break_before = self.next_break == Some(byte_start);
        if break_before {
            self.next_break = self.breaks.next();
        }
        let char_start = self.char_cursor;
        self.char_cursor += text.chars().count();
        Some(Grapheme {
            text,
            char_start,
            char_end: self.char_cursor,
            break_before,
        })
    }

    fn fill_to(&mut self, len: usize) {
        while self.buffer.len() < len {
            let Some(grapheme) = self.pull() else {
                break;
            };
            self.buffer.push_back(grapheme);
        }
    }

    fn fill_rest(&mut self) {
        while let Some(grapheme) = self.pull() {
            self.buffer.push_back(grapheme);
        }
    }

    /// True when everything after the first `consumed` buffered graphemes,
    /// including the source not pulled yet, is whitespace.
    fn rest_is_whitespace(&self, consumed: usize) -> bool {
        let is_space = |text: &str| text.chars().all(char::is_whitespace);
        self.buffer
            .iter()
            .skip(consumed)
            .all(|grapheme| is_space(grapheme.text))
            && self.source.clone().all(|(_, text)| is_space(text))
    }

    /// Width of the whitespace run after the first `from` buffered graphemes.
    ///
    /// Returns `None` when the run holds anything else, would exceed `limit`
    /// columns, or is so long that absorbing it would stop being bounded.
    fn whitespace_run_width(
        &self,
        from: usize,
        start_column: usize,
        limit: usize,
    ) -> Option<usize> {
        let mut column = start_column;
        let mut width = 0usize;
        let mut examined = 0usize;
        let buffered = self.buffer.iter().skip(from).map(|grapheme| grapheme.text);
        for text in buffered.chain(self.source.clone().map(|(_, text)| text)) {
            if !text.chars().all(char::is_whitespace) {
                return None;
            }
            let piece = piece_width(text, column, self.tab_stop);
            column += piece;
            width += piece;
            if width > limit {
                return None;
            }
            examined += 1;
            // Zero-width whitespace never trips the column limit; cap the
            // count so a pathological cluster run stays bounded too.
            if examined > limit.saturating_add(64) {
                return None;
            }
        }
        Some(width)
    }

    fn width_between(&self, start_column: usize, from: usize, to: usize) -> usize {
        let mut column = start_column;
        for grapheme in self.buffer.iter().take(to).skip(from) {
            column += piece_width(grapheme.text, column, self.tab_stop);
        }
        column - start_column
    }

    /// Display string for the row most recently returned by [`Iterator::next`].
    fn display_for_last_row(&self, start_column: usize) -> String {
        let mut display = String::new();
        let mut column = start_column;
        for grapheme in self.buffer.iter().take(self.consumed) {
            let width = piece_width(grapheme.text, column, self.tab_stop);
            if grapheme.text == "\t" {
                display.extend(std::iter::repeat_n(' ', width));
            } else {
                display.push_str(grapheme.text);
            }
            column += width;
        }
        display
    }
}

impl Iterator for Rows<'_> {
    type Item = VisualRow;

    fn next(&mut self) -> Option<VisualRow> {
        if self.finished {
            return None;
        }
        if self.consumed > 0 {
            self.buffer.drain(..self.consumed);
            self.consumed = 0;
        }
        if self.buffer.is_empty() {
            self.fill_to(1);
        }
        if self.buffer.is_empty() {
            self.finished = true;
            return Some(VisualRow {
                char_start: 0,
                char_end: 0,
                start_column: 0,
                columns: 0,
                grapheme_start: 0,
                grapheme_end: 0,
            });
        }

        let start_column = self.column;
        let mut end = 0usize;
        let mut last_break = None;
        let mut width = 0usize;

        loop {
            self.fill_to(end + 1);
            let Some(grapheme) = self.buffer.get(end).copied() else {
                break;
            };
            if self.policy == WrapPolicy::Word
                && grapheme.break_before
                && end > 0
                && self.max_columns.is_none_or(|max| width <= max)
            {
                last_break = Some(end);
            }
            let piece = piece_width(grapheme.text, start_column + width, self.tab_stop);
            if let Some(max) = self.max_columns {
                if width > 0 && width + piece > max {
                    end = last_break.unwrap_or(end);
                    break;
                }
            }
            width += piece;
            end += 1;
        }

        // Keep a short trailing whitespace run on the final row instead of
        // emitting a blank row for it, but never let an arbitrarily long run
        // widen the row past the wrap width. A run that does not fit is
        // wrapped normally; every character stays in a row range.
        if self.policy == WrapPolicy::Word && end > 0 {
            let used = self.width_between(start_column, 0, end);
            let absorb = match self.max_columns {
                Some(max) => self
                    .whitespace_run_width(end, start_column + used, max)
                    .is_some(),
                None => self.rest_is_whitespace(end),
            };
            if absorb {
                self.fill_rest();
                end = self.buffer.len();
            }
        }
        if end == 0 {
            // Defensive: a zero-width first grapheme must still make progress.
            end = 1;
        }

        let char_start = self.buffer[0].char_start;
        let char_end = self.buffer[end - 1].char_end;
        let columns = self.width_between(start_column, 0, end);
        self.consumed = end;
        self.column = start_column + columns;
        if self.buffer.len() == end && self.source_done {
            self.finished = true;
        }
        Some(VisualRow {
            char_start,
            char_end,
            start_column,
            columns,
            grapheme_start: 0,
            grapheme_end: end,
        })
    }
}

/// Iterate over the visual rows of `text`. `max_columns = None` means no soft
/// wrap. No display strings are allocated.
#[must_use]
pub fn rows(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
) -> Rows<'_> {
    Rows::new(text, max_columns, tab_stop, policy)
}

/// Split `text` into visual rows with the given policy.
#[must_use]
pub fn segments_with(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
) -> Vec<Segment> {
    segments_window(text, max_columns, tab_stop, policy, 0, usize::MAX)
}

/// Visual rows `skip..skip + limit` of `text`, without materializing the rest.
///
/// A giant hard line can have thousands of rows; painting only needs the
/// visible ones. The walker still scans the skipped rows (there is no random
/// access into a soft-wrapped line), but no display strings are built for them.
#[must_use]
pub fn segments_window(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    skip: usize,
    limit: usize,
) -> Vec<Segment> {
    let mut iter = Rows::new(text, max_columns, tab_stop, policy);
    let mut remaining = skip;
    let mut result = Vec::new();
    if limit == 0 {
        return result;
    }
    while let Some(row) = iter.next() {
        if remaining > 0 {
            remaining -= 1;
            continue;
        }
        result.push(Segment {
            display: iter.display_for_last_row(row.start_column),
            char_start: row.char_start,
            char_end: row.char_end,
            start_column: row.start_column,
            columns: row.columns,
        });
        if result.len() >= limit {
            break;
        }
    }
    result
}

/// Visual rows occupied by a hard line under `policy`.
///
/// This uses the same walker as [`segments_with`] without constructing display
/// strings, so a rewrap of a large document does not allocate once per row.
#[must_use]
pub fn row_count_with(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
) -> usize {
    Rows::new(text, max_columns, tab_stop, policy).count()
}

/// Split `text` into visual rows at grapheme boundaries.
///
/// This is the character policy kept as the default of the older API; callers
/// that want Unicode word-preferred wrapping use [`segments_with`].
#[must_use]
pub fn segments(text: &str, max_columns: Option<usize>, tab_stop: usize) -> Vec<Segment> {
    segments_with(text, max_columns, tab_stop, WrapPolicy::Character)
}

/// Visual rows occupied by a hard line at grapheme boundaries.
#[must_use]
pub fn row_count(text: &str, max_columns: Option<usize>, tab_stop: usize) -> usize {
    row_count_with(text, max_columns, tab_stop, WrapPolicy::Character)
}

/// Column of a char offset within a hard line, tab stops included.
#[must_use]
pub fn column_at(text: &str, char_offset: usize, tab_stop: usize) -> usize {
    let mut column = 0;
    let mut index = 0;
    for grapheme in text.graphemes(true) {
        if index >= char_offset {
            break;
        }
        if grapheme == "\t" {
            column += tab_width(column, tab_stop);
        } else {
            column += grapheme.width();
        }
        index += grapheme.chars().count();
    }
    column
}

/// Nearest char offset at or before `column`.
#[must_use]
pub fn char_at_column(text: &str, column: usize, tab_stop: usize) -> usize {
    let mut current = 0;
    let mut index = 0;
    for grapheme in text.graphemes(true) {
        let width = if grapheme == "\t" {
            tab_width(current, tab_stop)
        } else {
            grapheme.width()
        };
        if current + width > column {
            return index;
        }
        current += width;
        index += grapheme.chars().count();
    }
    index
}
