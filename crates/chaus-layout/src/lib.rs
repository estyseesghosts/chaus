//! Soft-wrap geometry independent of GPUI.
//!
//! A hard line is split into visual rows at either a Unicode line-break
//! opportunity (the default, word-preferred policy) or the furthest grapheme
//! that fits. Tabs expand to the next tab stop, wide CJK/emoji graphemes occupy
//! two columns and combining sequences/zero-width joiners stay attached to
//! their base. One shared row walker drives [`segments`], [`row_count`], caret
//! placement, hit testing and selection, so those can never disagree.
//!
//! [`LineSource`] is the Ropey-friendly input to that walker. A caller with a
//! `RopeSlice` asks for a bounded [`LineSource::window`] instead of
//! materializing a whole hard line; [`WrapCheckpoints`] supplies the char
//! offset and column at a known visual row so a repaint deep inside a wrapped
//! line does not walk every earlier row. The same walker and the same
//! grapheme/tab/line-break rules cover both contiguous and Ropey input.
//!
//! Tab stops are measured from the start of the logical line, never from the
//! start of a visual row. Resizing the window therefore changes where a line
//! wraps but never how wide a tab is, and [`column_at`] stays consistent with
//! the columns recorded in each [`Segment`].

use std::borrow::Cow;
use std::collections::VecDeque;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub};

use ropey::RopeSlice;
use unicode_linebreak::linebreaks;
use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

/// Chars fetched at a time from a non-contiguous [`LineSource`].
///
/// A window plus its context gets at most this many chars; a row that does not
/// fit is allowed to grow the window, but a document is never flattened.
const WINDOW_CHARS: usize = 4096;

/// Chars of lookahead kept so a grapheme at a window edge is never cut.
const WINDOW_CONTEXT: usize = 128;

macro_rules! layout_index {
    ($name:ident, $description:expr) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);

        impl $name {
            /// Construct the index from its underlying count.
            #[must_use]
            pub const fn new(value: usize) -> Self {
                Self(value)
            }

            /// Underlying count.
            #[must_use]
            pub const fn get(self) -> usize {
                self.0
            }
        }

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl PartialEq<usize> for $name {
            fn eq(&self, other: &usize) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$name> for usize {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl PartialOrd<usize> for $name {
            fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
                self.0.partial_cmp(other)
            }
        }

        impl PartialOrd<$name> for usize {
            fn partial_cmp(&self, other: &$name) -> Option<std::cmp::Ordering> {
                self.partial_cmp(&other.0)
            }
        }

        impl Add<usize> for $name {
            type Output = usize;

            fn add(self, rhs: usize) -> Self::Output {
                self.0 + rhs
            }
        }

        impl Sub<usize> for $name {
            type Output = usize;

            fn sub(self, rhs: usize) -> Self::Output {
                self.0 - rhs
            }
        }

        impl AddAssign<usize> for $name {
            fn add_assign(&mut self, rhs: usize) {
                self.0 += rhs;
            }
        }

        impl Sum<$name> for usize {
            fn sum<I: Iterator<Item = $name>>(iter: I) -> Self {
                iter.map(|value| value.0).sum()
            }
        }
    };
}

layout_index!(
    LineCharOffset,
    "Unicode scalar offset inside one hard line."
);
layout_index!(
    VisualColumn,
    "Display column inside a hard line, tabs expanded."
);
layout_index!(WrapRowIndex, "Visual row index inside one hard line.");
layout_index!(
    DisplayByteOffset,
    "UTF-8 byte offset inside expanded display text."
);

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

/// One grapheme boundary within a segment's rendered text.
///
/// `char_offset` is the source offset within the hard line; `display_byte` is
/// the UTF-8 byte offset within [`Segment::display`], where a tab has already
/// become spaces. The first entry starts at `(segment.char_start, 0)` and the
/// last ends at `(segment.char_end, segment.display.len())`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayBoundary {
    pub char_offset: LineCharOffset,
    pub display_byte: DisplayByteOffset,
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
    display_boundaries_from_column(line, segment, tab_stop, VisualColumn::new(0))
}

/// [`display_boundaries`] starting from a known column at `segment.char_start`.
///
/// Use this when only a bounded window of the hard line is available: pass the
/// column recorded at the window start, not zero, so a tab in the window still
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
/// Only the requested char range is materialized. The column at the range
/// start comes from a [`RowCheckpoint`] or the caller's line prefix sum.
#[must_use]
pub fn display_boundaries_source(
    source: &LineSource<'_>,
    segment: &Segment,
    tab_stop: usize,
    start_column: VisualColumn,
) -> Vec<DisplayBoundary> {
    let start = segment.char_start.get().saturating_sub(WINDOW_CONTEXT);
    let end = segment.char_end.get().min(source.char_len());
    let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    let mut local = segment.clone();
    local.char_start = LineCharOffset::new(segment.char_start.get() - start);
    local.char_end = LineCharOffset::new(segment.char_end.get().saturating_sub(start));
    let mut column = start_column;
    for grapheme in window.chars().take(start) {
        let piece = if grapheme == '\t' {
            tab_width(column.get(), tab_stop)
        } else {
            unicode_width::UnicodeWidthChar::width(grapheme).unwrap_or(0)
        };
        column += piece;
    }
    let boundaries = display_boundaries_from_column(&window, &local, tab_stop, column);
    boundaries
        .into_iter()
        .map(|boundary| DisplayBoundary {
            char_offset: LineCharOffset::new(boundary.char_offset.get() + start),
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

/// Contiguous or Ropey input for the shared row walker.
///
/// `Text` borrows an existing string. `Rope` materializes only the bounded
/// window a caller asks for, never the whole hard line.
#[derive(Clone, Copy, Debug)]
pub enum LineSource<'a> {
    /// A contiguous string, typically a small line from a test or a preview.
    Text(&'a str),
    /// A Ropey slice over one hard line of a large document.
    Rope(RopeSlice<'a>),
}

impl<'a> LineSource<'a> {
    /// Borrow a contiguous string.
    #[must_use]
    pub const fn from_str(text: &'a str) -> Self {
        Self::Text(text)
    }

    /// Borrow a Ropey slice.
    #[must_use]
    pub const fn from_rope(slice: RopeSlice<'a>) -> Self {
        Self::Rope(slice)
    }

    /// Unicode scalar length of the hard line.
    #[must_use]
    pub fn char_len(&self) -> usize {
        match self {
            Self::Text(text) => text.chars().count(),
            Self::Rope(rope) => rope.len_chars(),
        }
    }

    /// UTF-8 byte length of the hard line, including expanded source bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Rope(rope) => rope.len_bytes(),
        }
    }

    /// Materialize only `start..end` as text.
    ///
    /// For contiguous input this borrows from the original string; for Ropey
    /// input it allocates a bounded `String`. Callers include lookahead in the
    /// range so a grapheme at the edge is not split.
    #[must_use]
    pub fn window(&self, start: LineCharOffset, end: LineCharOffset) -> Cow<'a, str> {
        let total = self.char_len();
        let start = start.get().min(total);
        let end = end.get().clamp(start, total);
        match *self {
            Self::Text(text) => {
                let mut byte_start = 0usize;
                for (index, _) in text.char_indices() {
                    if index == start {
                        break;
                    }
                    byte_start = index;
                }
                let mut byte_end = text.len();
                for (seen, (index, _)) in (start..).zip(text.char_indices()) {
                    if seen >= end {
                        byte_end = index;
                        break;
                    }
                    byte_end = text.len();
                }
                Cow::Borrowed(&text[byte_start..byte_end])
            }
            Self::Rope(rope) => Cow::Owned(rope.slice(start..end).to_string()),
        }
    }
}

/// Numeric index into a wrapped line, used to restart the walker without
/// replaying earlier rows.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowCheckpoint {
    /// Visual row this checkpoint describes.
    pub row: WrapRowIndex,
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
    pub fn nearest(&self, row: WrapRowIndex) -> Option<&RowCheckpoint> {
        self.entries
            .iter()
            .take_while(|entry| entry.row <= row)
            .last()
    }

    /// Record a checkpoint. Callers pass rows in increasing order.
    pub fn push(&mut self, entry: RowCheckpoint) {
        if entry.row.get().is_multiple_of(self.stride) {
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
        Self::new_with_column(text, max_columns, tab_stop, policy, 0)
    }

    /// Start measuring at a known column. Used when a window begins at a
    /// checkpoint instead of at the hard line's first grapheme.
    fn new_with_column(
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
    fn display_for_last_row(&self, start_column: VisualColumn) -> String {
        let mut display = String::new();
        let mut column = start_column.get();
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
/// Pass a Ropey-backed [`LineSource`] to [`segments_window_source`] when the
/// skipped prefix must not be walked on every repaint.
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
    pub row: WrapRowIndex,
    /// Rows the walker had to visit. Equal to `row + 1` for a match, so the
    /// work stops at the target instead of materializing every later row.
    pub rows_visited: usize,
}

/// Locate the visual row containing `char_offset` without building segments.
///
/// Uses the same row walker as painting and hit testing, so caret navigation
/// cannot disagree with the rendered rows. A long unwrapped line visits one
/// row, and a wrapped line stops at the row holding the offset.
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
                row: WrapRowIndex::new(visited - 1),
                rows_visited: visited,
            };
        }
    }
    RowLookup {
        row: WrapRowIndex::new(visited.saturating_sub(1)),
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
) -> WrapRowIndex {
    row_of_char_with_stats(text, max_columns, tab_stop, policy, char_offset).row
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
pub fn column_at(
    text: &str,
    char_offset: impl Into<LineCharOffset>,
    tab_stop: usize,
) -> VisualColumn {
    column_from(text, char_offset.into(), tab_stop, VisualColumn::new(0))
}

fn column_from(
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
            column += tab_width(column, tab_stop);
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
            tab_width(current, tab_stop)
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

// ---- Ropey/chunk-friendly path --------------------------------------------

fn window_span(source: &LineSource<'_>, start: usize, window_chars: usize) -> (usize, usize) {
    let end = (start + window_chars).min(source.char_len());
    (start, end)
}

fn safe_window_end(source: &LineSource<'_>, start: usize, end: usize) -> usize {
    if end < source.char_len() {
        end.saturating_sub(WINDOW_CONTEXT).max(start)
    } else {
        end
    }
}

/// Build numeric checkpoints from a Ropey-backed source.
///
/// The source is read in bounded windows. Only complete rows are accepted, so
/// a grapheme that straddles a window edge is re-read with the next window.
/// No display strings are retained.
#[must_use]
pub fn build_checkpoints(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    stride: usize,
) -> WrapCheckpoints {
    let mut checkpoints = WrapCheckpoints::new(stride);
    if source.char_len() == 0 {
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
                    row: WrapRowIndex::new(row_index),
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
/// This walks bounded windows and keeps only the numeric row cursor, so a
/// million-row hard line does not allocate a string per row.
#[must_use]
pub fn row_count_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    checkpoints: &WrapCheckpoints,
) -> usize {
    if source.char_len() == 0 {
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
#[must_use]
pub fn row_of_char_source(
    source: &LineSource<'_>,
    max_columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    char_offset: LineCharOffset,
    checkpoints: &WrapCheckpoints,
) -> RowLookup {
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
        .entries
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
#[must_use]
pub fn column_at_source(
    source: &LineSource<'_>,
    char_offset: LineCharOffset,
    tab_stop: usize,
    checkpoints: &WrapCheckpoints,
) -> VisualColumn {
    let start = checkpoints
        .entries
        .iter()
        .take_while(|entry| entry.char_offset <= char_offset)
        .last()
        .map_or(0, |entry| entry.char_offset.get());
    let column = checkpoints
        .entries
        .iter()
        .take_while(|entry| entry.char_offset <= char_offset)
        .last()
        .map_or(0, |entry| entry.column.get());
    let end = (char_offset.get() + WINDOW_CONTEXT).min(source.char_len());
    let window = source.window(LineCharOffset::new(start), LineCharOffset::new(end));
    column_from(
        &window,
        LineCharOffset::new(char_offset.get() - start),
        tab_stop,
        VisualColumn::new(column),
    )
}

/// Nearest char offset at or before a column through a bounded source window.
#[must_use]
pub fn char_at_column_source(
    source: &LineSource<'_>,
    column: VisualColumn,
    tab_stop: usize,
    checkpoints: &WrapCheckpoints,
) -> LineCharOffset {
    let checkpoint = checkpoints
        .entries
        .iter()
        .take_while(|entry| entry.column.get() <= column.get())
        .last()
        .map_or((0usize, 0usize), |entry| {
            (entry.char_offset.get(), entry.column.get())
        });
    let end = (checkpoint.0 + WINDOW_CHARS).min(source.char_len());
    let window = source.window(LineCharOffset::new(checkpoint.0), LineCharOffset::new(end));
    let local = char_at_column(&window, column.get().saturating_sub(checkpoint.1), tab_stop);
    LineCharOffset::new(checkpoint.0 + local.get())
}
