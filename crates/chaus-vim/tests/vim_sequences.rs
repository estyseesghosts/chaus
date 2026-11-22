//! Table-driven modal sequences against an in-memory document.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{Engine, Key, Mode, Outcome};

fn new_engine() -> Engine {
    Engine::new(Vec::new())
}

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

#[test]
fn dw_deletes_a_word_and_undo_restores_it() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "dw");
    assert_eq!(document.text(), "two three");
    press(&mut engine, &mut document, &mut selection, "u");
    assert_eq!(document.text(), "one two three");
}

#[test]
fn counts_multiply_for_operators_and_motions() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "d2w");
    assert_eq!(document.text(), "three");

    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "2dw");
    assert_eq!(document.text(), "three");
}

#[test]
fn doubled_operators_are_linewise() {
    let mut document = Document::from_source_text("a\nb\nc\n");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "2dd");
    assert_eq!(document.text(), "c\n");
}

#[test]
fn yank_and_put_preserve_linewise_semantics() {
    let mut document = Document::from_source_text("a\nb\n");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "yyp");
    assert_eq!(document.text(), "a\na\nb\n");
}

#[test]
fn quote_text_objects_keep_or_remove_delimiters() {
    let mut document = Document::from_source_text("say \"hi\" now");
    let mut selection = Selection::caret(CharOffset(5));
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "di\"");
    assert_eq!(document.text(), "say \"\" now");

    let mut document = Document::from_source_text("say \"hi\" now");
    let mut selection = Selection::caret(CharOffset(5));
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "da\"");
    assert_eq!(document.text(), "say  now");
}

#[test]
fn change_word_enters_insert_and_clears_the_word() {
    let mut document = Document::from_source_text("one two");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    let outcome = engine.handle_key(Key::Char('c'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    let _ = engine.handle_key(Key::Char('i'), &mut document, &mut selection);
    let _ = engine.handle_key(Key::Char('w'), &mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Insert);
    assert_eq!(document.text(), " two");
}

#[test]
fn escape_returns_to_normal_and_moves_off_the_insertion_point() {
    let mut document = Document::from_source_text("ab");
    let mut selection = Selection::caret(CharOffset(2));
    let mut engine = new_engine();
    let _ = engine.handle_key(Key::Char('i'), &mut document, &mut selection);
    engine.record_inserted("xy");
    let _ = engine.handle_key(Key::Escape, &mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Normal);
    assert_eq!(selection.head, CharOffset(1));
}

#[test]
fn unknown_normal_keys_never_insert_text() {
    let mut document = Document::from_source_text("text");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    let outcome = engine.handle_key(Key::Char('Z'), &mut document, &mut selection);
    assert_eq!(document.text(), "text");
    assert!(matches!(outcome, Outcome::Message(_)));
}

#[test]
fn search_moves_the_caret_and_n_repeats() {
    let mut document = Document::from_source_text("one two one");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    press(&mut engine, &mut document, &mut selection, "/two\n");
    assert_eq!(selection.head, CharOffset(4));
    press(&mut engine, &mut document, &mut selection, "n");
    assert_eq!(selection.head, CharOffset(4), "no second match after two");
}

#[test]
fn toggling_off_cancels_a_pending_operator() {
    let mut document = Document::from_source_text("one two");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    let _ = engine.handle_key(Key::Char('d'), &mut document, &mut selection);
    engine.reset();
    let outcome = engine.handle_key(Key::Char('a'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(engine.mode(), Mode::Insert);
    assert_eq!(document.text(), "one two");
}

#[test]
fn insert_mode_is_unhandled_except_for_escape() {
    let mut document = Document::from_source_text("");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = new_engine();
    let _ = engine.handle_key(Key::Char('i'), &mut document, &mut selection);
    let outcome = engine.handle_key(Key::Char('q'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Unhandled);
}
