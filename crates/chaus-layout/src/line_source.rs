//! Bounded input for the shared row walker.

use std::borrow::Cow;

use ropey::RopeSlice;

use crate::LineCharOffset;

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

    /// Materialize only the Unicode scalar range `start..end` as text.
    ///
    /// `start` and `end` are **scalar indices**. `str::char_indices()` yields
    /// **byte offsets**, so converting one to the other is explicit here.
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
                let byte_start = text
                    .char_indices()
                    .nth(start)
                    .map_or(text.len(), |(index, _)| index);
                let byte_end = text
                    .char_indices()
                    .nth(end)
                    .map_or(text.len(), |(index, _)| index);
                Cow::Borrowed(&text[byte_start..byte_end])
            }
            Self::Rope(rope) => Cow::Owned(rope.slice(start..end).to_string()),
        }
    }
}
