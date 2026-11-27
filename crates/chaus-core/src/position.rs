//! Position newtypes.
//!
//! Every interface that consumes an offset names its unit in the type system:
//! Ropey character indices, UTF-8 bytes (Tree-sitter, shaping), UTF-16 code
//! units (platform text services), hard lines and shaped visual rows are not
//! interchangeable.

use std::fmt;
use std::ops::Range;

macro_rules! index_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);

        impl $name {
            pub const ZERO: Self = Self(0);

            pub fn get(self) -> usize {
                self.0
            }

            pub fn checked_add(self, n: usize) -> Option<Self> {
                self.0.checked_add(n).map(Self)
            }

            pub fn checked_sub(self, n: usize) -> Option<Self> {
                self.0.checked_sub(n).map(Self)
            }

            pub fn saturating_sub(self, n: usize) -> Self {
                Self(self.0.saturating_sub(n))
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

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

index_type!(
    CharOffset,
    "A Unicode scalar index into the Ropey document. Canonical edit unit."
);
index_type!(
    ByteOffset,
    "A UTF-8 byte offset from the start of the document (Tree-sitter, shaped text runs)."
);
index_type!(
    LineByteOffset,
    "A UTF-8 byte column from the start of a hard line (Tree-sitter points)."
);
index_type!(
    Utf16Offset,
    "A UTF-16 code unit index (platform input/accessibility)."
);
index_type!(LineIndex, "A zero-based hard line index in the document.");
index_type!(VisualRow, "A zero-based visual row after soft wrapping.");
index_type!(
    GraphemeColumn,
    "A caret boundary counted in grapheme clusters within a hard line."
);
index_type!(
    DocumentRevision,
    "A monotonically increasing document revision."
);

impl DocumentRevision {
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A half-open range of Ropey character indices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CharRange {
    pub start: CharOffset,
    pub end: CharOffset,
}

impl CharRange {
    #[must_use]
    pub fn new(start: CharOffset, end: CharOffset) -> Self {
        // Construction normalizes direction; callers that care about anchor
        // and head use `Selection`, not `CharRange`.
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    #[must_use]
    pub fn empty(at: CharOffset) -> Self {
        Self { start: at, end: at }
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.end.0 - self.start.0
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    #[must_use]
    pub fn contains(self, offset: CharOffset) -> bool {
        self.start <= offset && offset < self.end
    }

    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A half-open range of document-global UTF-8 byte offsets.
///
/// Unlike [`LineByteOffset`], these offsets are measured from the start of the
/// document, so they can be passed to Tree-sitter or Ropey's byte APIs after
/// converting each end with [`ByteOffset::get`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ByteRange {
    pub start: ByteOffset,
    pub end: ByteOffset,
}

impl ByteRange {
    #[must_use]
    pub fn new(start: ByteOffset, end: ByteOffset) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub fn len(self) -> usize {
        self.end.0 - self.start.0
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl From<Range<usize>> for ByteRange {
    fn from(value: Range<usize>) -> Self {
        Self::new(ByteOffset(value.start), ByteOffset(value.end))
    }
}

impl From<ByteRange> for Range<usize> {
    fn from(value: ByteRange) -> Self {
        value.start.0..value.end.0
    }
}

/// A document selection with an anchor (where the selection began) and a head
/// (where the caret is). Direction is preserved for Shift+Arrow, dragging and
/// Visual mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: CharOffset,
    pub head: CharOffset,
}

impl Selection {
    #[must_use]
    pub fn caret(at: CharOffset) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    #[must_use]
    pub fn new(anchor: CharOffset, head: CharOffset) -> Self {
        Self { anchor, head }
    }

    #[must_use]
    pub fn range(self) -> CharRange {
        CharRange::new(self.anchor, self.head)
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    #[must_use]
    pub fn reversed(self) -> bool {
        self.head < self.anchor
    }

    #[must_use]
    pub fn clamp(self, len: usize) -> Self {
        let max = CharOffset(len);
        Self {
            anchor: self.anchor.min(max),
            head: self.head.min(max),
        }
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::caret(CharOffset::ZERO)
    }
}
