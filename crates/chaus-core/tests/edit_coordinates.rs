//! Coordinate spaces reported by applied edits.
//!
//! Character ranges use Ropey indices, byte ranges are document-global UTF-8,
//! and point columns are UTF-8 bytes relative to their own hard line.

use chaus_core::document::{Document, EditCause, EditResult, EditTransaction, TextEdit};
use chaus_core::position::{
    ByteOffset, ByteRange, CharOffset, CharRange, LineByteOffset, Selection,
};

fn apply(document: &mut Document, range: CharRange, insert: &str) -> EditResult {
    let after = CharOffset(range.start.0 + insert.chars().count());
    document
        .apply(EditTransaction::new(
            EditCause::Paste,
            TextEdit::new(range, insert),
            Selection::caret(range.start),
            Selection::caret(after),
        ))
        .expect("edit should apply")
}

#[test]
fn point_columns_are_line_relative_utf8_bytes() {
    let document = Document::from_source_text("é😀\n漢字ab");
    assert_eq!(
        document.text_point(CharOffset(1)).column_bytes,
        LineByteOffset(2)
    );
    assert_eq!(
        document.text_point(CharOffset(2)).column_bytes,
        LineByteOffset(6)
    );
    let at_row_one = document.text_point(CharOffset(3));
    assert_eq!(at_row_one.row, 1);
    assert_eq!(at_row_one.column_bytes, LineByteOffset::ZERO);
    assert_eq!(
        document.text_point(CharOffset(5)).column_bytes,
        LineByteOffset(6)
    );
}

#[test]
fn applied_byte_ranges_are_document_global() {
    let mut document = Document::from_source_text("é😀\nxy");
    let result = apply(
        &mut document,
        CharRange::new(CharOffset(3), CharOffset(5)),
        "😀",
    );
    let edit = &result.edits[0];
    assert_eq!(edit.old_range, CharRange::new(CharOffset(3), CharOffset(5)));
    assert_eq!(edit.new_range, CharRange::new(CharOffset(3), CharOffset(4)));
    assert_eq!(
        edit.old_byte_range,
        ByteRange::new(ByteOffset(7), ByteOffset(9))
    );
    assert_eq!(
        edit.new_byte_range,
        ByteRange::new(ByteOffset(7), ByteOffset(11))
    );
    assert_eq!(edit.new_end_point.row, 1);
    assert_eq!(
        edit.new_end_point.column_bytes,
        LineByteOffset(4),
        "the line column is relative to row 1, unlike the document byte range"
    );
}

#[test]
fn multi_edit_byte_ranges_rebase_to_the_final_document() {
    let mut document = Document::from_source_text("abcdef");
    let result = document
        .apply(EditTransaction {
            edits: vec![
                TextEdit::new(CharRange::new(CharOffset(0), CharOffset(1)), "LONG"),
                TextEdit::new(CharRange::new(CharOffset(4), CharOffset(5)), "X"),
            ],
            before_selection: Selection::caret(CharOffset::ZERO),
            after_selection: Selection::caret(CharOffset(8)),
            cause: EditCause::ReplaceAll,
        })
        .expect("multi-edit transaction should apply");
    assert_eq!(document.text(), "LONGbcdXf");

    assert_eq!(
        result.edits[0].new_range,
        CharRange::new(CharOffset(0), CharOffset(4))
    );
    assert_eq!(
        result.edits[0].new_byte_range,
        ByteRange::new(ByteOffset(0), ByteOffset(4))
    );
    assert_eq!(
        result.edits[1].new_range,
        CharRange::new(CharOffset(7), CharOffset(8))
    );
    assert_eq!(
        result.edits[1].new_byte_range,
        ByteRange::new(ByteOffset(7), ByteOffset(8))
    );
}

#[test]
fn byte_range_converts_to_raw_usize_at_the_boundary() {
    let range = ByteRange::new(ByteOffset(3), ByteOffset(9));
    let raw: std::ops::Range<usize> = range.into();
    assert_eq!(raw, 3..9);
    assert_eq!(
        ByteRange::from(2..5),
        ByteRange::new(ByteOffset(2), ByteOffset(5))
    );
    assert_eq!(range.len(), 6);
    assert!(!range.is_empty());
}
