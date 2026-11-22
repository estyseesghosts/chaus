//! UTF-16 offset conversion against a whole-text reference.
//!
//! The Fenwick tree must agree with a simple prefix-of-all-chars reference
//! after point edits, splits, merges, undo and redo, including surrogate
//! clamping and runs of empty lines.

use std::time::Instant;

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, Selection, Utf16Offset};

fn utf16_prefix(text: &str) -> Vec<usize> {
    let mut prefix = vec![0usize];
    for ch in text.chars() {
        let next = prefix.last().copied().unwrap_or(0) + ch.len_utf16();
        prefix.push(next);
    }
    prefix
}

fn assert_matches_reference(doc: &Document) {
    let text = doc.rope().to_string();
    let prefix = utf16_prefix(&text);
    let len = text.chars().count();
    for offset in 0..=len {
        let units = doc.char_to_utf16(CharOffset(offset)).0;
        assert_eq!(units, prefix[offset], "char {offset} -> UTF-16");
        assert_eq!(
            doc.utf16_to_char(Utf16Offset(units)).0,
            offset,
            "UTF-16 {units} -> char"
        );
    }
    assert_eq!(doc.utf16_to_char(Utf16Offset(usize::MAX)).0, len);
    // Interior surrogate offsets clamp to the scalar start.
    for offset in 0..len {
        let start = prefix[offset];
        let next = prefix[offset + 1];
        if next > start + 1 {
            assert_eq!(
                doc.utf16_to_char(Utf16Offset(start + 1)).0,
                offset,
                "interior surrogate must clamp left"
            );
        }
    }
}

fn apply(doc: &mut Document, start: usize, end: usize, insert: &str) {
    let before = Selection::caret(CharOffset(start));
    let after = Selection::caret(CharOffset(start + insert.chars().count()));
    let transaction = EditTransaction::new(
        EditCause::Typing,
        TextEdit::new(CharRange::new(CharOffset(start), CharOffset(end)), insert),
        before,
        after,
    );
    doc.apply(transaction).expect("valid edit");
}

#[test]
fn round_trips_through_empty_lines_surrogates_and_combining_marks() {
    let source = "\n\nabc\n\n👩‍💻 e\u{301}\n漢字\n\n\nlast\n";
    let doc = Document::from_source_text(source);
    assert_matches_reference(&doc);
}

#[test]
fn insert_split_merge_undo_and_redo_keep_the_index_consistent() {
    let mut doc = Document::from_source_text("alpha\nbeta\ngamma\n👩‍💻\ndelta\n");
    assert_matches_reference(&doc);

    apply(&mut doc, 0, 0, "👩‍💻");
    assert_matches_reference(&doc);
    apply(&mut doc, 3, 3, "\n");
    assert_matches_reference(&doc);
    apply(&mut doc, 0, 1, "");
    assert_matches_reference(&doc);
    apply(&mut doc, 0, 2, "");
    assert_matches_reference(&doc);
    let end = doc.len_chars();
    apply(&mut doc, end, end, "\n\nnew");
    assert_matches_reference(&doc);

    doc.undo().expect("undo");
    assert_matches_reference(&doc);
    doc.undo().expect("undo");
    assert_matches_reference(&doc);
    doc.redo().expect("redo");
    assert_matches_reference(&doc);
    doc.redo().expect("redo");
    assert_matches_reference(&doc);
}

#[test]
fn a_hundred_thousand_lines_stay_responsive_to_input_conversions() {
    let source = "alpha beta gamma\n".repeat(100_000);
    let mut doc = Document::from_source_text(&source);
    assert_matches_reference(&doc);
    let started = Instant::now();
    for step in 0..1_000 {
        // Same-line edits near the start exercise the Fenwick point update,
        // not a rebuild of every later line.
        let position = step % 5;
        apply(&mut doc, position, position, "x");
        let units = doc.char_to_utf16(CharOffset(position + 1)).0;
        assert_eq!(doc.utf16_to_char(Utf16Offset(units)).0, position + 1);
        apply(&mut doc, position, position + 1, "");
    }
    let elapsed = started.elapsed();
    eprintln!("2000 edits and conversions on 100k lines took {elapsed:?}");
    assert!(
        elapsed.as_secs() < 20,
        "UTF-16 conversions must not rebuild the whole prefix per edit: {elapsed:?}"
    );
}
