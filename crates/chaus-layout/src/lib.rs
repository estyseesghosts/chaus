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

use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;
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

struct Grapheme<'a> {
    text: &'a str,
    char_start: usize,
    char_end: usize,
    /// A Unicode line-break opportunity immediately before this grapheme.
    break_before: bool,
}

/// Iterator over the visual rows of one hard line.
pub struct Rows<'a> {
    graphemes: Vec<Grapheme<'a>>,
    policy: WrapPolicy,
    max_columns: Option<usize>,
    tab_stop: usize,
    next_grapheme: usize,
    next_char: usize,
    column: usize,
    finished: bool,
}

impl<'a> Rows<'a> {
    fn new(text: &'a str, max_columns: Option<usize>, tab_stop: usize, policy: WrapPolicy) -> Self {
        let max_columns = max_columns.filter(|columns| *columns > 0);
        let mut graphemes = Vec::new();
        let mut char_index = 0usize;
        for grapheme in text.graphemes(true) {
            let char_start = char_index;
            char_index += grapheme.chars().count();
            graphemes.push(Grapheme {
                text: grapheme,
                char_start,
                char_end: char_index,
                break_before: false,
            });
        }
        if policy == WrapPolicy::Word && !graphemes.is_empty() {
            mark_break_opportunities(text, &mut graphemes);
        }
        Self {
            graphemes,
            policy,
            max_columns,
            tab_stop,
            next_grapheme: 0,
            next_char: 0,
            column: 0,
            finished: false,
        }
    }

    fn width_between(&self, start_column: usize, from: usize, to: usize) -> usize {
        let mut column = start_column;
        for grapheme in &self.graphemes[from..to] {
            column += piece_width(grapheme.text, column, self.tab_stop);
        }
        column - start_column
    }

    fn display_between(&self, from: usize, to: usize, start_column: usize) -> String {
        let mut display = String::new();
        let mut column = start_column;
        for grapheme in &self.graphemes[from..to] {
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

/// Map every UAX #14 break opportunity to a grapheme boundary.
///
/// `unicode-linebreak` reports UTF-8 byte offsets. A grapheme boundary is the
/// only safe place to break, so an opportunity that falls inside a grapheme is
/// ignored rather than splitting a combining sequence or ZWJ emoji.
fn mark_break_opportunities(text: &str, graphemes: &mut [Grapheme<'_>]) {
    // Byte offset -> char offset, valid at every char boundary.
    let mut byte_to_char = vec![0usize; text.len() + 1];
    let mut char_index = 0usize;
    for (byte, _) in text.char_indices() {
        byte_to_char[byte] = char_index;
        char_index += 1;
    }
    byte_to_char[text.len()] = char_index;

    let mut grapheme = 0usize;
    for (byte, _) in linebreaks(text) {
        let char_offset = byte_to_char[byte];
        while grapheme + 1 < graphemes.len() && graphemes[grapheme + 1].char_start <= char_offset {
            grapheme += 1;
        }
        if graphemes[grapheme].char_start == char_offset {
            graphemes[grapheme].break_before = true;
        }
    }
}

impl Iterator for Rows<'_> {
    type Item = VisualRow;

    fn next(&mut self) -> Option<VisualRow> {
        if self.finished {
            return None;
        }
        if self.graphemes.is_empty() {
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

        let start = self.next_grapheme;
        let start_char = self.next_char;
        let start_column = self.column;
        let mut end = start;
        let mut last_break = None;
        let mut width = 0usize;

        while end < self.graphemes.len() {
            let grapheme = &self.graphemes[end];
            if self.policy == WrapPolicy::Word
                && grapheme.break_before
                && end > start
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

        // Never emit a blank row made only of trailing whitespace: keep it on
        // the final row so the visible document has no phantom line. All
        // characters remain in row ranges and no source text is removed.
        if self.policy == WrapPolicy::Word
            && end < self.graphemes.len()
            && end > start
            && self.graphemes[end..]
                .iter()
                .all(|grapheme| grapheme.text.chars().all(char::is_whitespace))
        {
            end = self.graphemes.len();
        }
        if end == start {
            // Defensive: a zero-width first grapheme must still make progress.
            end = start + 1;
        }

        let end_char = self.graphemes[end - 1].char_end;
        let columns = self.width_between(start_column, start, end);
        self.next_grapheme = end;
        self.next_char = end_char;
        self.column = start_column + columns;
        if end >= self.graphemes.len() {
            self.finished = true;
        }
        Some(VisualRow {
            char_start: start_char,
            char_end: end_char,
            start_column,
            columns,
            grapheme_start: start,
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
    let mut iter = Rows::new(text, max_columns, tab_stop, policy);
    let mut result = Vec::new();
    while let Some(row) = iter.next() {
        result.push(Segment {
            display: iter.display_between(row.grapheme_start, row.grapheme_end, row.start_column),
            char_start: row.char_start,
            char_end: row.char_end,
            start_column: row.start_column,
            columns: row.columns,
        });
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
