//! History bookkeeping after extracting the `History` owner.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, Selection, Utf16Offset};

fn insert(document: &mut Document, at: usize, text: &str, cause: EditCause) {
    let after = CharOffset(at + text.chars().count());
    document
        .apply(EditTransaction::new(
            cause,
            TextEdit::insertion(CharOffset(at), text),
            Selection::caret(CharOffset(at)),
            Selection::caret(after),
        ))
        .expect("insert should apply");
}

#[test]
fn a_non_typing_edit_breaks_the_typing_coalescing_chain() {
    let mut document = Document::new();
    insert(&mut document, 0, "a", EditCause::Typing);
    insert(&mut document, 1, "b", EditCause::Paste);
    insert(&mut document, 2, "c", EditCause::Typing);

    assert_eq!(document.text(), "abc");
    assert_eq!(document.undo_depth(), 3);
    document.undo();
    assert_eq!(document.text(), "ab");
}

#[test]
fn editing_after_undo_discards_the_redo_branch() {
    let mut document = Document::from_source_text("a");
    insert(&mut document, 1, "b", EditCause::Paste);
    assert!(document.can_undo());

    document.undo().expect("undo");
    assert!(document.can_redo());
    insert(&mut document, 1, "c", EditCause::Typing);

    assert_eq!(document.text(), "ac");
    assert!(!document.can_redo());
    assert!(document.redo().is_none());
    assert_eq!(document.undo_depth(), 1);
}

#[test]
fn nested_user_operations_undo_as_one_step() {
    let mut document = Document::new();
    document.begin_user_operation();
    insert(&mut document, 0, "a", EditCause::Paste);
    document.begin_user_operation();
    insert(&mut document, 1, "b", EditCause::Paste);
    document.end_user_operation();
    insert(&mut document, 2, "c", EditCause::Paste);
    document.end_user_operation();

    assert_eq!(document.undo_depth(), 1);
    document.undo().expect("undo");
    assert_eq!(document.text(), "");
    document.redo().expect("redo");
    assert_eq!(document.text(), "abc");
}

#[test]
fn cancelling_an_operation_stops_further_merging() {
    let mut document = Document::new();
    document.begin_user_operation();
    insert(&mut document, 0, "a", EditCause::Paste);
    document.cancel_user_operation();
    insert(&mut document, 1, "b", EditCause::Paste);

    document.undo().expect("undo");
    assert_eq!(document.text(), "a");
}

#[test]
fn mixed_endings_survive_undo_and_redo() {
    let mut document = Document::from_source_text("a\r\nb\nc\rd");
    let saved_token = document.state_token();
    document
        .apply(EditTransaction::new(
            EditCause::Paste,
            TextEdit::new(CharRange::empty(CharOffset(3)), "\nX\n"),
            Selection::caret(CharOffset(3)),
            Selection::caret(CharOffset(6)),
        ))
        .expect("insert should apply");
    let edited = "a\r\nb\r\nX\r\n\nc\rd";
    assert_eq!(document.text_with_eol(), edited);

    document.undo().expect("undo");
    assert_eq!(document.text_with_eol(), "a\r\nb\nc\rd");
    assert_eq!(document.state_token(), saved_token);
    document.redo().expect("redo");
    assert_eq!(document.text_with_eol(), edited);
}

#[test]
fn surrogate_pair_conversion_stays_clamped_across_replay() {
    let mut document = Document::new();
    insert(&mut document, 0, "😀", EditCause::Typing);
    assert_eq!(document.char_to_utf16(CharOffset(1)), Utf16Offset(2));
    assert_eq!(document.utf16_to_char(Utf16Offset(1)), CharOffset::ZERO);

    document.undo().expect("undo");
    assert_eq!(document.char_to_utf16(CharOffset::ZERO), Utf16Offset(0));
    document.redo().expect("redo");
    assert_eq!(document.utf16_to_char(Utf16Offset(1)), CharOffset::ZERO);
    assert_eq!(document.char_to_utf16(CharOffset(1)), Utf16Offset(2));
}
