//! Incremental invalidation ranges reported by `Document`.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};

fn document(text: &str) -> Document {
    Document::from_source_text(text)
}

fn replace(document: &mut Document, start: usize, end: usize, text: &str) {
    let range = CharRange::new(CharOffset(start), CharOffset(end));
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::new(range, text),
            Selection::caret(range.start),
            Selection::caret(CharOffset(start + text.chars().count())),
        ))
        .expect("the edit should apply");
}

#[test]
fn an_edit_reports_only_the_lines_it_touches() {
    let mut document = document("one\n two\n three\n four\n");
    let start = document.line_start(LineIndex(1));
    replace(&mut document, start.0, start.0, "X");

    let affected = document.last_changed_lines();
    assert!(
        affected.contains(LineIndex(1)),
        "the edited line must be reported: {affected:?}"
    );
    assert!(
        !affected.contains(LineIndex(3)),
        "far-away lines must not be invalidated: {affected:?}"
    );
}

#[test]
fn inserting_a_break_widens_the_range_to_the_new_line() {
    let mut document = document("alpha\nbeta\n");
    let break_at = CharOffset(5);
    replace(&mut document, break_at.0, break_at.0, "\n");

    let affected = document.last_changed_lines();
    assert!(
        affected.contains(LineIndex(0)) && affected.contains(LineIndex(1)),
        "both sides of a new break must be invalidated: {affected:?}"
    );
}

#[test]
fn undo_and_redo_report_the_lines_they_restore() {
    let mut document = document("one\n two\n three\n");
    let start = document.line_start(LineIndex(1));
    replace(&mut document, start.0, start.0, "X");
    let edited = document.last_changed_lines();

    document.undo();
    let undone = document.last_changed_lines();
    assert!(
        undone.contains(LineIndex(1)),
        "undo must report its own range, not the previous edit: {undone:?}"
    );

    document.redo();
    let redone = document.last_changed_lines();
    assert_eq!(
        redone, edited,
        "redo must touch the same lines as the original edit"
    );
}
