//! Undo history, saved-state tracking and UTF-16 index maintenance.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, Selection, Utf16Offset};

fn type_char(document: &mut Document, at: usize, text: &str) {
    let before = Selection::caret(CharOffset(at));
    let after = Selection::caret(CharOffset(at + text.chars().count()));
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(CharOffset(at), text),
            before,
            after,
        ))
        .expect("typing should apply");
}

#[test]
fn typing_after_save_is_dirty_even_when_it_would_have_coalesced() {
    let mut document = Document::new();
    type_char(&mut document, 0, "a");
    document.mark_saved();
    type_char(&mut document, 1, "b");

    assert_eq!(document.text(), "ab");
    assert!(
        document.is_dirty(),
        "text typed after saving must never inherit the saved state"
    );
    assert_eq!(
        document.undo_depth(),
        2,
        "the saved entry must not absorb later typing"
    );

    document.undo();
    assert_eq!(document.text(), "a");
    assert!(!document.is_dirty(), "undo returns to the saved revision");
}

#[test]
fn undoing_back_to_the_initial_empty_state_is_clean() {
    let mut document = Document::new();
    type_char(&mut document, 0, "a");
    document.undo();

    assert_eq!(document.text(), "");
    assert!(
        !document.is_dirty(),
        "the untitled initial state is the baseline"
    );
}

#[test]
fn editing_after_undo_past_the_saved_state_is_dirty() {
    let mut document = Document::new();
    type_char(&mut document, 0, "a");
    document.mark_saved();
    document.undo();
    type_char(&mut document, 0, "b");

    assert_eq!(document.text(), "b");
    assert!(
        document.is_dirty(),
        "the saved text was on the discarded redo branch"
    );
}

#[test]
fn typing_before_save_still_coalesces() {
    let mut document = Document::new();
    type_char(&mut document, 0, "a");
    type_char(&mut document, 1, "b");
    type_char(&mut document, 2, "c");

    assert_eq!(document.undo_depth(), 1);
    assert!(document.is_dirty());
}

#[test]
fn utf16_offsets_round_trip_with_leading_blank_lines() {
    let document = Document::from_source_text("\n\na😀b\ncafé");
    let text = document.text();
    for offset in 0..=text.chars().count() {
        let units = document.char_to_utf16(CharOffset(offset));
        assert_eq!(
            document.utf16_to_char(units),
            CharOffset(offset),
            "round trip at char {offset}"
        );
    }
    assert_eq!(document.utf16_to_char(Utf16Offset(0)), CharOffset::ZERO);
}

#[test]
fn utf16_index_survives_undo_and_redo_across_lines() {
    let mut document = Document::from_source_text("αβ\nγ😀\nδ");
    let before = Selection::new(CharOffset(2), CharOffset(4));
    document
        .apply(EditTransaction::new(
            EditCause::Paste,
            TextEdit::new(CharRange::new(CharOffset(2), CharOffset(4)), "one\ntwo😀"),
            before,
            Selection::caret(CharOffset(11)),
        ))
        .expect("replace should apply");

    let assert_round_trip = |document: &Document| {
        let text = document.text();
        let mut expected = 0;
        for offset in 0..=text.chars().count() {
            assert_eq!(
                document.char_to_utf16(CharOffset(offset)),
                Utf16Offset(expected),
                "prefix at {offset} in {:?}",
                document.text()
            );
            if let Some(character) = text.chars().nth(offset) {
                expected += character.len_utf16();
            }
        }
        assert_eq!(
            document.char_to_utf16(CharOffset(text.chars().count())).0,
            text.encode_utf16().count()
        );
    };

    assert_round_trip(&document);
    document.undo();
    assert_round_trip(&document);
    document.redo();
    assert_round_trip(&document);
}

#[test]
fn a_transaction_with_several_disjoint_edits_undoes_as_one_step() {
    let mut document = Document::from_source_text("abcdef");
    document
        .apply(EditTransaction {
            edits: vec![
                TextEdit::new(CharRange::new(CharOffset(0), CharOffset(1)), "A"),
                TextEdit::new(CharRange::new(CharOffset(4), CharOffset(5)), "E"),
            ],
            before_selection: Selection::caret(CharOffset::ZERO),
            after_selection: Selection::caret(CharOffset(2)),
            cause: EditCause::ReplaceAll,
        })
        .expect("multi-edit transaction should apply");

    assert_eq!(document.text(), "AbcdEf");
    assert_eq!(document.undo_depth(), 1);
    document.undo();
    assert_eq!(document.text(), "abcdef");
    document.redo();
    assert_eq!(document.text(), "AbcdEf");
}
