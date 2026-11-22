//! Incremental Tree-sitter syntax highlighting.
//!
//! This crate has no GPUI dependency. It detects a language by extension or
//! well-known file name, owns the parser and tree, applies `InputEdit`s built
//! from the document's byte/point ranges, and returns revision-tagged spans in
//! char offsets for the editor to paint.

use chaus_core::document::AppliedEdit;

pub mod highlight;
pub mod language;

pub use tree_sitter::{InputEdit, Tree};

pub use highlight::{
    capture_kind, edit_for_transaction, parse_snapshot, Highlight, HighlightError, ParsedHighlight,
    Span, TokenKind, MAX_HIGHLIGHT_BYTES,
};
pub use language::LanguageId;

/// Build one Tree-sitter `InputEdit` from a single applied document edit.
///
/// Returns `None` for multi-edit transactions: their rebased ranges do not
/// describe one intermediate tree state, so the caller reparses instead of
/// feeding an invalid edit sequence. The document uses LF-normalized text in
/// its Rope and its points count UTF-8 byte columns, matching Tree-sitter.
#[must_use]
pub fn input_edit(edit: &AppliedEdit) -> Option<tree_sitter::InputEdit> {
    Some(tree_sitter::InputEdit {
        start_byte: edit.old_byte_range.start,
        old_end_byte: edit.old_byte_range.end,
        new_end_byte: edit.new_byte_range.end,
        start_position: point(edit.old_start_point),
        old_end_position: point(edit.old_end_point),
        new_end_position: point(edit.new_end_point),
    })
}

fn point(point: chaus_core::document::TextPoint) -> tree_sitter::Point {
    tree_sitter::Point {
        row: point.row,
        column: point.column_bytes,
    }
}
