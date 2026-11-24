//! Line joining (`J`/`gJ`) and the `*`/`#` literal word searches.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{Engine, Key, Outcome};

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

fn edit(source: &str, caret: usize, keys: &str) -> String {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    document.text()
}

fn caret_after(source: &str, caret: usize, keys: &str) -> CharOffset {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    selection.head
}

#[test]
fn j_joins_with_one_space_and_trims_the_next_indent() {
    assert_eq!(edit("one\ntwo", 0, "J"), "one two");
    assert_eq!(edit("one\n   two", 0, "J"), "one two");
    assert_eq!(edit("one  \n   two", 0, "J"), "one  two");
}

#[test]
fn j_does_not_add_a_space_at_an_empty_line_boundary() {
    assert_eq!(edit("one\n\ntwo", 0, "J"), "one\ntwo");
    assert_eq!(edit("\ntwo", 0, "J"), "two");
    assert_eq!(edit("one\n", 0, "J"), "one");
}

#[test]
fn gj_removes_only_the_newline() {
    assert_eq!(edit("one\n   two", 0, "gJ"), "one   two");
    assert_eq!(edit("one  \n\ttwo", 0, "gJ"), "one  \ttwo");
}

#[test]
fn counted_joins_stop_at_eof() {
    assert_eq!(
        edit("one\ntwo\nthree\nfour", 0, "2J"),
        "one two\nthree\nfour"
    );
    assert_eq!(
        edit("one\ntwo\nthree\nfour", 0, "3J"),
        "one two three\nfour"
    );
    assert_eq!(edit("one\ntwo\nthree", 0, "99J"), "one two three");
    assert_eq!(edit("one\ntwo\nthree", 0, "2gJ"), "onetwo\nthree");
}

#[test]
fn join_is_one_undo_unit_and_dot_repeatable() {
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "3J");
    assert_eq!(document.text(), "one two three");
    assert_eq!(document.undo_depth(), 1);
    document.undo();
    assert_eq!(document.text(), "one\ntwo\nthree");

    let mut document = Document::from_source_text("a\nb\nc\nd");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "Jj.");
    assert_eq!(document.text(), "a b\nc d");
}

#[test]
fn j_documents_the_join_point_and_the_closing_paren_rule() {
    assert_eq!(caret_after("one\ntwo", 0, "J"), CharOffset(3));
    assert_eq!(caret_after("one\ntwo", 0, "gJ"), CharOffset(3));
    assert_eq!(caret_after("one\ntwo\nthree", 0, "3J"), CharOffset(3));
    assert_eq!(edit("call(\n)", 0, "J"), "call()");
}

#[test]
fn joining_a_crlf_document_keeps_its_line_ending_kind() {
    let mut document = Document::from_source_text("one\r\ntwo\r\n");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "J");
    assert_eq!(document.text(), "one two\n");
    assert_eq!(
        document.text_with_eol(),
        "one two\r\n",
        "the remaining hard break keeps its CRLF kind"
    );
}

#[test]
fn join_on_the_last_line_reports_without_changing_text() {
    let mut document = Document::from_source_text("one\ntwo");
    let mut selection = Selection::caret(CharOffset(4));
    let mut engine = Engine::new(Vec::new());
    let outcome = engine.handle_key(Key::Char('J'), &mut document, &mut selection);
    assert!(matches!(outcome, Outcome::Message(_)));
    assert_eq!(document.text(), "one\ntwo");
}

#[test]
fn star_searches_the_word_under_the_caret_and_n_repeats_it() {
    let source = "one two one two";
    assert_eq!(caret_after(source, 0, "*"), CharOffset(8));
    assert_eq!(caret_after(source, 0, "**"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "*n"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "*N"), CharOffset(0));
}

#[test]
fn hash_searches_backward_and_n_keeps_that_direction() {
    let source = "one two one two";
    assert_eq!(caret_after(source, 12, "#"), CharOffset(4));
    assert_eq!(caret_after(source, 12, "#n"), CharOffset(12));
    assert_eq!(caret_after(source, 12, "#N"), CharOffset(12));
}

#[test]
fn star_uses_the_nearest_word_from_punctuation_and_whitespace() {
    let source = "foo.bar baz";
    assert_eq!(caret_after(source, 3, "*"), CharOffset(4));
    assert_eq!(caret_after(source, 3, "#"), CharOffset(4));
    let source = "bar foo bar foo bar";
    assert_eq!(caret_after(source, 8, "*"), CharOffset(16));
    assert_eq!(caret_after(source, 8, "#"), CharOffset(0));
}

#[test]
fn a_single_occurrence_stays_in_place() {
    let source = "only once";
    assert_eq!(caret_after(source, 0, "*"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "#"), CharOffset(0));
}

#[test]
fn star_handles_unicode_words_and_punctuation_runs() {
    let source = "éé noir éé";
    assert_eq!(caret_after(source, 0, "*"), CharOffset(8));
    let source = "a+b a+b";
    assert_eq!(caret_after(source, 1, "*"), CharOffset(6));
}

#[test]
fn star_stores_a_literal_search_for_n_and_n() {
    let mut document = Document::from_source_text("x.y xzy x.y");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "*");
    assert_eq!(
        selection.head,
        CharOffset(4),
        "the word is `x`, not a regex"
    );
    press(&mut engine, &mut document, &mut selection, "n");
    assert_eq!(selection.head, CharOffset(8));
    press(&mut engine, &mut document, &mut selection, "N");
    assert_eq!(selection.head, CharOffset(4));
}
