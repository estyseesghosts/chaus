//! End-to-end regressions for the iterator-based word scans: counts, Unicode,
//! punctuation, line breaks, operators, Visual mode and dot-repeat all go
//! through the same motion resolver.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{Engine, Key, Mode};

fn press(engine: &mut Engine, document: &mut Document, selection: &mut Selection, keys: &str) {
    for character in keys.chars() {
        let key = match character {
            '\u{1b}' => Key::Escape,
            '\n' => Key::Enter,
            _ => Key::Char(character),
        };
        let _ = engine.handle_key(key, document, selection);
    }
}

fn caret_after(source: &str, caret: usize, keys: &str) -> CharOffset {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    selection.head
}

fn edit(source: &str, caret: usize, keys: &str) -> String {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    document.text()
}

fn type_text(document: &mut Document, selection: &mut Selection, text: &str) {
    let caret = CharOffset(selection.head.0 + text.chars().count());
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(selection.head, text),
            *selection,
            Selection::caret(caret),
        ))
        .expect("insert");
    *selection = Selection::caret(caret);
}

#[test]
fn word_and_big_word_keep_separate_punctuation_semantics() {
    let source = "foo.bar/baz qux";
    assert_eq!(caret_after(source, 0, "w"), CharOffset(3), "w stops at '.'");
    assert_eq!(
        caret_after(source, 0, "2w"),
        CharOffset(4),
        "2w crosses '.'"
    );
    assert_eq!(caret_after(source, 0, "W"), CharOffset(12), "W is a run");
    assert_eq!(caret_after(source, 0, "e"), CharOffset(2));
    assert_eq!(caret_after(source, 0, "E"), CharOffset(10));
    assert_eq!(caret_after(source, 12, "b"), CharOffset(8));
    assert_eq!(caret_after(source, 12, "B"), CharOffset(0));
}

#[test]
fn unicode_letters_underscores_and_punctuation_runs_keep_legacy_positions() {
    let source = "café_Ω 漢字!!";
    assert_eq!(caret_after(source, 0, "w"), CharOffset(7));
    assert_eq!(caret_after(source, 0, "W"), CharOffset(7));
    assert_eq!(caret_after(source, 0, "e"), CharOffset(5));
    assert_eq!(caret_after(source, 11, "b"), CharOffset(9));
    assert_eq!(caret_after(source, 9, "b"), CharOffset(7));
    assert_eq!(caret_after(source, 7, "e"), CharOffset(8));
    assert_eq!(caret_after(source, 7, "2e"), CharOffset(10));
    assert_eq!(caret_after(source, 11, "B"), CharOffset(7));
}

#[test]
fn crlf_and_lf_are_whitespace_boundaries() {
    assert_eq!(caret_after("one\r\ntwo", 0, "w"), CharOffset(4));
    assert_eq!(caret_after("one\r\ntwo", 7, "b"), CharOffset(4));
    assert_eq!(caret_after("one\n\ntwo", 0, "W"), CharOffset(5));
    assert_eq!(caret_after("one\n\ntwo", 0, "2W"), CharOffset(7));
}

#[test]
fn counts_beyond_the_document_clamp_at_the_end() {
    assert_eq!(caret_after("one two three", 0, "10000w"), CharOffset(12));
    assert_eq!(caret_after("one two three", 0, "10000W"), CharOffset(12));
    // `e` at the last char of a run stays there until punctuation follows it.
    assert_eq!(caret_after("one two three", 0, "10000e"), CharOffset(2));
    assert_eq!(caret_after("one two three", 12, "10000b"), CharOffset::ZERO);
    assert_eq!(caret_after("one two three", 12, "10000B"), CharOffset::ZERO);
    assert_eq!(
        caret_after("one two three", 12, "10000ge"),
        CharOffset::ZERO
    );
}

#[test]
fn empty_document_and_edges_do_not_move_past_bounds() {
    assert_eq!(caret_after("", 0, "wWeEbBge"), CharOffset::ZERO);
    assert_eq!(caret_after("a", 0, "w"), CharOffset::ZERO);
    assert_eq!(caret_after("a", 0, "e"), CharOffset::ZERO);
    assert_eq!(caret_after("a", 0, "b"), CharOffset::ZERO);
    assert_eq!(caret_after("a", 0, "ge"), CharOffset::ZERO);
    assert_eq!(
        caret_after("   ", 0, "w"),
        CharOffset(2),
        "clamped onto the line"
    );
    assert_eq!(caret_after("   ", 0, "B"), CharOffset::ZERO);
}

#[test]
fn operator_motions_use_the_iterator_scanned_ranges() {
    assert_eq!(edit("one two three", 0, "dw"), "two three");
    assert_eq!(edit("foo.bar/baz qux", 0, "dW"), "qux");
    assert_eq!(edit("foo.bar/baz qux", 0, "d2w"), "bar/baz qux");
    assert_eq!(
        edit("one two three four", 0, "dge"),
        "one two three four",
        "ge at the document start has nothing to delete"
    );

    let mut document = Document::from_source_text("foo.bar baz");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "cW");
    assert_eq!(engine.mode(), Mode::Insert);
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "X baz");
}

#[test]
fn visual_word_motions_select_through_the_iterator_scanned_targets() {
    assert_eq!(
        edit("one two three", 0, "vwd"),
        "wo three",
        "Visual is inclusive"
    );
    assert_eq!(
        edit("foo.bar/baz qux", 0, "vWd"),
        "ux",
        "Visual is inclusive"
    );
    assert_eq!(edit("one two three", 0, "v2wd"), "hree");
    assert_eq!(edit("one two three", 8, "vBd"), "one hree");
}

#[test]
fn dot_repeat_replays_the_word_motion_range() {
    assert_eq!(edit("one two three four", 0, "dw."), "three four");
    assert_eq!(edit("foo.bar baz qux", 0, "dW."), "qux");
    assert_eq!(
        edit("one two three four", 0, "2dw."),
        "",
        "the repeat reaches EOF"
    );
}
