//! Soft-wrap geometry independent of GPUI.
//!
//! Lines are split at column boundaries measured in grapheme clusters. Tabs
//! expand to the next tab stop, wide CJK/emoji graphemes occupy two columns and
//! combining sequences/zero-width joiners stay attached to their base. The
//! same functions drive painting, caret placement and hit testing so those
//! three can never disagree.
//!
//! Tab stops are measured from the start of the logical line, never from the
//! start of a visual row. Resizing the window therefore changes where a line
//! wraps but never how wide a tab is, and [`column_at`] stays consistent with
//! the columns recorded in each [`Segment`]. A grapheme or an expanded tab is
//! never split across rows; when a tab does not fit it moves whole to the next
//! row, which can leave that row one tab stop wider than requested.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

fn tab_width(column: usize, tab_stop: usize) -> usize {
    let tab_stop = tab_stop.max(1);
    tab_stop - (column % tab_stop)
}

/// Split `text` into visual rows. `max_columns = None` means no soft wrap.
#[must_use]
pub fn segments(text: &str, max_columns: Option<usize>, tab_stop: usize) -> Vec<Segment> {
    let max_columns = max_columns.filter(|columns| *columns > 0);
    let mut result = Vec::new();
    let mut display = String::new();
    let mut char_start = 0usize;
    let mut char_index = 0usize;
    let mut start_column = 0usize;
    let mut width = 0usize;

    for grapheme in text.graphemes(true) {
        let is_tab = grapheme == "\t";
        let piece_width = if is_tab {
            tab_width(start_column + width, tab_stop)
        } else {
            grapheme.width()
        };
        let piece = if is_tab {
            " ".repeat(piece_width)
        } else {
            grapheme.to_string()
        };

        if let Some(max) = max_columns {
            if width > 0 && width + piece_width > max {
                result.push(Segment {
                    display: std::mem::take(&mut display),
                    char_start,
                    char_end: char_index,
                    start_column,
                    columns: width,
                });
                char_start = char_index;
                start_column += width;
                width = 0;
            }
        }

        display.push_str(&piece);
        width += piece_width;
        char_index += grapheme.chars().count();
    }

    result.push(Segment {
        display,
        char_start,
        char_end: char_index,
        start_column,
        columns: width,
    });
    result
}

/// Visual rows occupied by a hard line.
#[must_use]
pub fn row_count(text: &str, max_columns: Option<usize>, tab_stop: usize) -> usize {
    segments(text, max_columns, tab_stop).len()
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
