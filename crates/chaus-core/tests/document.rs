//! Document transactions, history, EOL metadata and offset conversions.

use chaus_core::document::{Document, EditCause, EditError, EditResult, EditTransaction, TextEdit};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{
    CharOffset, CharRange, DocumentRevision, LineIndex, Selection, Utf16Offset,
};

fn doc(text: &str) -> Document {
    Document::from_source_text(text)
}

fn insert(document: &mut Document, at: usize, text: &str) -> EditResult {
    let selection = Selection::caret(CharOffset(at));
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(CharOffset(at), text),
            selection,
            Selection::caret(CharOffset(at + text.chars().count())),
        ))
        .expect("insert should apply")
}

#[test]
fn insert_and_delete_round_trip() {
    let mut document = doc("hello world");
    insert(&mut document, 5, ",");
    assert_eq!(document.text(), "hello, world");
    let result = document
        .apply(EditTransaction::new(
            EditCause::Delete,
            TextEdit::deletion(CharRange::new(CharOffset(0), CharOffset(6))),
            Selection::caret(CharOffset(6)),
            Selection::caret(CharOffset::ZERO),
        ))
        .expect("delete should apply");
    assert_eq!(document.text(), " world");
    assert_eq!(result.affected_lines.start, LineIndex(0));
}

#[test]
fn utf16_conversion_handles_astral_and_multiline() {
    let document = doc("a😀b\ncafé\n");
    let emoji_end = CharOffset(2);
    assert_eq!(document.char_to_utf16(emoji_end), Utf16Offset(3));
    assert_eq!(
        document.utf16_to_char(Utf16Offset(2)),
        CharOffset(1),
        "inside a surrogate pair clamps to the scalar boundary"
    );
    let len = document.text().encode_utf16().count();
    assert_eq!(
        document.char_to_utf16(CharOffset(document.len_chars())).0,
        len
    );
    for offset in 0..=document.len_chars() {
        let units = document.char_to_utf16(CharOffset(offset));
        assert_eq!(
            document.utf16_to_char(units),
            CharOffset(offset),
            "round trip at char {offset}"
        );
    }
}

#[test]
fn grapheme_navigation_crosses_combining_and_zwj() {
    let document = doc("e\u{301}x👨‍👩‍👧‍👦y");
    let after_e = document.next_grapheme_boundary(CharOffset(0));
    assert_eq!(after_e, CharOffset(2), "combining accent joins the base");
    let after_family = document.next_grapheme_boundary(CharOffset(3));
    assert_eq!(after_family, CharOffset(10));
    assert_eq!(document.prev_grapheme_boundary(after_family), CharOffset(3));
}

#[test]
fn undo_and_redo_restore_text_selection_and_eol() {
    let mut document = doc("a\r\nb");
    let before = Selection::caret(CharOffset(3));
    let after = Selection::caret(CharOffset(7));
    document
        .apply(EditTransaction::new(
            EditCause::Paste,
            TextEdit::insertion(CharOffset(3), "\nnew"),
            before,
            after,
        ))
        .expect("paste should apply");
    assert_eq!(document.text(), "a\nb\nnew");
    assert_eq!(document.text_with_eol(), "a\r\nb\r\nnew");
    assert!(document.is_dirty());
    document.undo();
    assert_eq!(document.text_with_eol(), "a\r\nb");
    assert!(!document.is_dirty());
    document.redo();
    assert_eq!(document.text_with_eol(), "a\r\nb\r\nnew");
}

#[test]
fn typing_coalesces_into_one_undo_entry() {
    let mut document = Document::new();
    for (index, ch) in "abc".chars().enumerate() {
        let at = CharOffset(index);
        document
            .apply(EditTransaction::new(
                EditCause::Typing,
                TextEdit::insertion(at, ch.to_string()),
                Selection::caret(at),
                Selection::caret(CharOffset(index + 1)),
            ))
            .expect("typing should apply");
    }
    assert_eq!(document.undo_depth(), 1);
    document.undo();
    assert_eq!(document.text(), "");
}

#[test]
fn newline_starts_a_new_undo_entry() {
    let mut document = Document::new();
    insert(&mut document, 0, "a");
    insert(&mut document, 1, "\n");
    assert_eq!(document.undo_depth(), 2);
}

#[test]
fn no_op_transactions_do_not_add_history_or_dirty_the_document() {
    let mut document = doc("abc");
    let error = document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(CharOffset(1), ""),
            Selection::caret(CharOffset(1)),
            Selection::caret(CharOffset(1)),
        ))
        .expect_err("empty edit should be rejected");
    assert!(matches!(error, EditError::EmptyTransaction));
    assert_eq!(document.text(), "abc");
    assert_eq!(document.undo_depth(), 0);
    assert!(!document.is_dirty());
    assert_eq!(document.revision(), DocumentRevision::ZERO);
}

#[test]
fn out_of_bounds_edits_are_rejected_without_mutation() {
    let mut document = doc("abc");
    let error = document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(CharOffset(99), "x"),
            Selection::caret(CharOffset(99)),
            Selection::caret(CharOffset(99)),
        ))
        .expect_err("should reject");
    assert!(matches!(error, EditError::OutOfBounds { .. }));
    assert_eq!(document.text(), "abc");
    assert_eq!(document.revision(), DocumentRevision::ZERO);
}

#[test]
fn overlapping_edits_are_rejected() {
    let mut document = doc("abcdef");
    let error = document
        .apply(EditTransaction {
            edits: vec![
                TextEdit::new(CharRange::new(CharOffset(0), CharOffset(3)), "x"),
                TextEdit::new(CharRange::new(CharOffset(2), CharOffset(4)), "y"),
            ],
            before_selection: Selection::caret(CharOffset::ZERO),
            after_selection: Selection::caret(CharOffset::ZERO),
            cause: EditCause::ReplaceAll,
        })
        .expect_err("should reject overlap");
    assert!(matches!(error, EditError::OverlappingEdits { .. }));
}

#[test]
fn mixed_eol_survives_edits_to_other_lines() {
    let mut document = doc("one\r\ntwo\nthree\r");
    insert(&mut document, 3, "!");
    assert_eq!(document.text_with_eol(), "one!\r\ntwo\nthree\r");
    document.undo();
    assert_eq!(document.text_with_eol(), "one\r\ntwo\nthree\r");
}

#[test]
fn deleted_breaks_remove_the_right_eol_entries() {
    let mut document = doc("a\r\nb\nc");
    document
        .apply(EditTransaction::new(
            EditCause::Delete,
            TextEdit::deletion(CharRange::new(CharOffset(1), CharOffset(2))),
            Selection::caret(CharOffset(2)),
            Selection::caret(CharOffset(1)),
        ))
        .expect("delete should apply");
    assert_eq!(document.text(), "ab\nc");
    assert_eq!(document.text_with_eol(), "ab\nc");
    document.undo();
    assert_eq!(document.text_with_eol(), "a\r\nb\nc");
}

#[test]
fn reload_clears_history_and_dirty_state() {
    let mut document = doc("old");
    insert(&mut document, 0, "!");
    assert!(document.is_dirty());
    document.reset("new".to_string(), EolMap::default(), Eol::Lf);
    assert_eq!(document.text(), "new");
    assert!(!document.is_dirty());
    assert!(!document.can_undo());
}

#[test]
fn utf16_index_stays_correct_after_edits() {
    let mut document = doc("αβ\nγ😀\n");
    insert(&mut document, 3, "😀");
    let text = document.text();
    let mut expected = 0;
    for offset in 0..=text.chars().count() {
        assert_eq!(
            document.char_to_utf16(CharOffset(offset)).0,
            expected,
            "prefix at {offset}"
        );
        if let Some(ch) = text.chars().nth(offset) {
            expected += ch.len_utf16();
        }
    }
}

#[test]
fn multi_edit_transaction_invalidates_lines_shifted_by_a_lower_edit() {
    let mut document = doc("a\nb\nc");
    let result = document
        .apply(EditTransaction {
            edits: vec![
                TextEdit::insertion(CharOffset::ZERO, "\n\n"),
                TextEdit::new(CharRange::new(CharOffset(4), CharOffset(5)), "X"),
            ],
            before_selection: Selection::caret(CharOffset::ZERO),
            after_selection: Selection::caret(CharOffset::ZERO),
            cause: EditCause::ReplaceAll,
        })
        .expect("multi-edit transaction should apply");

    assert_eq!(document.text(), "\n\na\nb\nX");
    assert_eq!(document.len_lines(), 5);
    assert_eq!(
        result.affected_lines.end,
        LineIndex(document.len_lines()),
        "the changed line moved below the pre-shift range"
    );
}

#[test]
fn disjoint_edits_of_different_lengths_undo_and_redo_in_order() {
    let mut document = doc("abcdef");
    document
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
    document.undo();
    assert_eq!(document.text(), "abcdef");
    document.redo();
    assert_eq!(document.text(), "LONGbcdXf");
    for offset in 0..=document.len_chars() {
        let units = document.char_to_utf16(CharOffset(offset));
        assert_eq!(document.utf16_to_char(units), CharOffset(offset));
    }
}

#[test]
fn undo_restores_replaced_text_not_just_insertions() {
    let mut document = doc("one two");
    document
        .apply(EditTransaction::new(
            EditCause::ReplaceAll,
            TextEdit::new(CharRange::new(CharOffset(4), CharOffset(7)), "three"),
            Selection::caret(CharOffset(7)),
            Selection::caret(CharOffset(9)),
        ))
        .expect("replace should apply");
    assert_eq!(document.text(), "one three");
    document.undo();
    assert_eq!(document.text(), "one two");
    document.redo();
    assert_eq!(document.text(), "one three");
}
