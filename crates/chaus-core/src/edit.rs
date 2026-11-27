//! Vocabulary of the single document transaction path.
//!
//! [`TextEdit`] ranges and [`AppliedEdit::old_range`]/[`AppliedEdit::new_range`]
//! use Ropey character indices. `*_byte_range` fields are document-global UTF-8
//! byte ranges. [`TextPoint`] rows are hard-line indices; its columns are UTF-8
//! bytes relative to that line, not to the document.

use crate::position::{
    ByteRange, CharOffset, CharRange, DocumentRevision, LineByteOffset, LineIndex, Selection,
};

/// The origin of an edit. History grouping and dirty tracking depend on it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCause {
    Typing,
    Delete,
    Paste,
    ImeCommit,
    Vim,
    Formatter,
    ReplaceAll,
    Indent,
}

/// One replacement in original-document character coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: CharRange,
    pub insert: String,
}

impl TextEdit {
    #[must_use]
    pub fn new(range: CharRange, insert: impl Into<String>) -> Self {
        Self {
            range,
            insert: insert.into(),
        }
    }

    #[must_use]
    pub fn insertion(at: CharOffset, text: impl Into<String>) -> Self {
        Self::new(CharRange::empty(at), text)
    }

    #[must_use]
    pub fn deletion(range: CharRange) -> Self {
        Self::new(range, String::new())
    }

    #[must_use]
    pub fn inserted_chars(&self) -> usize {
        self.insert.chars().count()
    }
}

/// A set of disjoint edits applied as one user operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    pub edits: Vec<TextEdit>,
    pub before_selection: Selection,
    pub after_selection: Selection,
    pub cause: EditCause,
}

impl EditTransaction {
    #[must_use]
    pub fn new(cause: EditCause, edit: TextEdit, before: Selection, after: Selection) -> Self {
        Self {
            edits: vec![edit],
            before_selection: before,
            after_selection: after,
            cause,
        }
    }

    /// A single insertion replacing `selection`.
    #[must_use]
    pub fn replace_selection(
        cause: EditCause,
        selection: Selection,
        text: impl Into<String>,
    ) -> Self {
        let text = text.into();
        let end = CharOffset(selection.range().start.0 + text.chars().count());
        Self::new(
            cause,
            TextEdit::new(selection.range(), text),
            selection,
            Selection::caret(end),
        )
    }
}

/// A row/UTF-8-byte-column point, the unit Tree-sitter consumes.
///
/// `row` is a zero-based hard-line index. `column_bytes` counts UTF-8 bytes
/// from the start of that row, so converting it to a document byte offset
/// requires adding the row's own starting byte offset.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPoint {
    pub row: usize,
    pub column_bytes: LineByteOffset,
}

/// The effect of one applied edit, in pre-edit and final post-edit
/// coordinates. Multiple edits in one transaction are rebased after all edits
/// so every range refers to the final document.
///
/// `old_range` and `new_range` are Ropey character ranges. `old_byte_range`
/// and `new_byte_range` are half-open document-global UTF-8 byte ranges in the
/// pre-edit and final documents respectively. The points use hard-line rows
/// with line-relative UTF-8 byte columns.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct AppliedEdit {
    pub old_range: CharRange,
    pub new_range: CharRange,
    pub old_byte_range: ByteRange,
    pub new_byte_range: ByteRange,
    pub old_start_point: TextPoint,
    pub old_end_point: TextPoint,
    pub new_end_point: TextPoint,
    pub removed_text: String,
    pub inserted_text: String,
}

/// Result of a transaction, used to invalidate caches and update parsers.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct EditResult {
    pub revision: DocumentRevision,
    pub edits: Vec<AppliedEdit>,
    pub affected_lines: LineRange,
}

/// A half-open span of hard lines.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineRange {
    pub start: LineIndex,
    pub end: LineIndex,
}

impl LineRange {
    #[must_use]
    pub fn new(start: LineIndex, end: LineIndex) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }

    #[must_use]
    pub fn contains(self, line: LineIndex) -> bool {
        self.start <= line && line < self.end
    }
}

/// Reasons a transaction may be rejected. The document is left untouched on
/// every error.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    OutOfBounds { offset: CharOffset, length: usize },
    OverlappingEdits { first: CharRange, second: CharRange },
    EmptyTransaction,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::OutOfBounds { offset, length } => {
                write!(
                    formatter,
                    "offset {offset} is beyond document length {length}"
                )
            }
            EditError::OverlappingEdits { first, second } => write!(
                formatter,
                "edits overlap: {}..{} and {}..{}",
                first.start, first.end, second.start, second.end
            ),
            EditError::EmptyTransaction => {
                write!(formatter, "transaction contains no text changes")
            }
        }
    }
}

impl std::error::Error for EditError {}
