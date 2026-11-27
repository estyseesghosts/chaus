//! The shared visual-row walker for wrapped and unwrapped hard lines.

use std::collections::VecDeque;

use unicode_linebreak::linebreaks;
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::{LineCharOffset, VisualColumn};

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
    pub char_start: LineCharOffset,
    /// Char offset one past the segment end within the hard line.
    pub char_end: LineCharOffset,
    /// Visual columns before this segment (wrapping included).
    pub start_column: VisualColumn,
    /// Number of visual columns covered by this segment.
    pub columns: VisualColumn,
}

impl Segment {
    #[must_use]
    pub fn end_column(&self) -> VisualColumn {
        VisualColumn::new(self.start_column.get() + self.columns.get())
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
    pub char_start: LineCharOffset,
    /// Char offset one past the row end within the hard line.
    pub char_end: LineCharOffset,
    /// Visual columns before this row (wrapping included).
    pub start_column: VisualColumn,
    /// Number of visual columns covered by this row.
    pub columns: VisualColumn,
    grapheme_start: usize,
    grapheme_end: usize,
}

impl VisualRow {
    #[must_use]
    pub fn end_column(&self) -> VisualColumn {
        VisualColumn::new(self.start_column.get() + self.columns.get())
    }
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
/// not allocate a vector proportional to its length. Production unwrapped
/// painting uses the dedicated horizontal-window API instead of this walker.
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
        Self::new_with_column(text, max_columns, tab_stop, policy, 0)
    }

    /// Start measuring at a known column. Used when a window begins at a
    /// checkpoint instead of at the hard line's first grapheme.
    pub(crate) fn new_with_column(
        text: &'a str,
        max_columns: Option<usize>,
        tab_stop: usize,
        policy: WrapPolicy,
        start_column: usize,
    ) -> Self {
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
            column: start_column,
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
            let piece = crate::piece_width(text, column, self.tab_stop);
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
            column += crate::piece_width(grapheme.text, column, self.tab_stop);
        }
        column - start_column
    }

    /// Display string for the row most recently returned by [`Iterator::next`].
    pub(crate) fn display_for_last_row(&self, start_column: VisualColumn) -> String {
        let mut display = String::new();
        let mut column = start_column.get();
        for grapheme in self.buffer.iter().take(self.consumed) {
            let width = crate::piece_width(grapheme.text, column, self.tab_stop);
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
                char_start: LineCharOffset::new(0),
                char_end: LineCharOffset::new(0),
                start_column: VisualColumn::new(0),
                columns: VisualColumn::new(0),
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
            let piece = crate::piece_width(grapheme.text, start_column + width, self.tab_stop);
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
            char_start: LineCharOffset::new(char_start),
            char_end: LineCharOffset::new(char_end),
            start_column: VisualColumn::new(start_column),
            columns: VisualColumn::new(columns),
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
/// Pass a Ropey-backed [`crate::LineSource`] to
/// [`crate::segments_window_source`] when the skipped prefix must not be walked
/// on every repaint.
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

/// Result of locating a character offset among a hard line's visual rows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowLookup {
    /// Index of the visual row containing the offset.
    pub row: crate::WrapRowIndex,
    /// Rows the walker had to visit. Equal to `row + 1` for a match, so the
    /// work stops at the target instead of materializing every later row.
    pub rows_visited: usize,
}

/// Locate the visual row containing `char_offset` without building segments.
#[must_use]
pub fn row_of_char_with_stats(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    char_offset: impl Into<LineCharOffset>,
) -> RowLookup {
    let char_offset = char_offset.into().get();
    let rows = Rows::new(text, max_columns, tab_stop, policy);
    let mut visited = 0usize;
    for row in rows {
        visited += 1;
        if char_offset < row.char_end.get() {
            return RowLookup {
                row: crate::WrapRowIndex::new(visited - 1),
                rows_visited: visited,
            };
        }
    }
    RowLookup {
        row: crate::WrapRowIndex::new(visited.saturating_sub(1)),
        rows_visited: visited,
    }
}

/// Locate the visual row containing `char_offset`.
#[must_use]
pub fn row_of_char(
    text: &str,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    char_offset: impl Into<LineCharOffset>,
) -> crate::WrapRowIndex {
    row_of_char_with_stats(text, max_columns, tab_stop, policy, char_offset).row
}

/// Visual rows occupied by a hard line under `policy`.
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
pub fn column_at(
    text: &str,
    char_offset: impl Into<LineCharOffset>,
    tab_stop: usize,
) -> VisualColumn {
    column_from(text, char_offset.into(), tab_stop, VisualColumn::new(0))
}

/// Column of `char_offset` when `start_column` is the column at scalar zero of
/// `text`. Tabs are measured against absolute logical-line columns.
pub(crate) fn column_from(
    text: &str,
    char_offset: LineCharOffset,
    tab_stop: usize,
    start_column: VisualColumn,
) -> VisualColumn {
    let mut column = start_column.get();
    let mut index = 0usize;
    for grapheme in text.graphemes(true) {
        if index >= char_offset.get() {
            break;
        }
        if grapheme == "\t" {
            column += crate::tab_width(column, tab_stop);
        } else {
            column += grapheme.width();
        }
        index += grapheme.chars().count();
    }
    VisualColumn::new(column)
}

/// Nearest char offset at or before `column`.
#[must_use]
pub fn char_at_column(
    text: &str,
    column: impl Into<VisualColumn>,
    tab_stop: usize,
) -> LineCharOffset {
    let column = column.into().get();
    let mut current = 0usize;
    let mut index = 0usize;
    for grapheme in text.graphemes(true) {
        let width = if grapheme == "\t" {
            crate::tab_width(current, tab_stop)
        } else {
            grapheme.width()
        };
        if current + width > column {
            return LineCharOffset::new(index);
        }
        current += width;
        index += grapheme.chars().count();
    }
    LineCharOffset::new(index)
}
