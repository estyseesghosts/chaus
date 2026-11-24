//! Normal-mode edit aliases and the special `cw`/`cW` word range.

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

fn type_text(document: &mut Document, selection: &mut Selection, text: &str) {
    let before = *selection;
    let caret = CharOffset(selection.head.0 + text.chars().count());
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(selection.head, text),
            before,
            Selection::caret(caret),
        ))
        .expect("insert");
    *selection = Selection::caret(caret);
}

fn edit(source: &str, caret: usize, keys: &str) -> String {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    document.text()
}

fn change(source: &str, caret: usize, command: &str, typed: &str) -> (String, usize) {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, command);
    assert_eq!(engine.mode(), Mode::Insert, "command {command:?}");
    type_text(&mut document, &mut selection, typed);
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    let depth = document.undo_depth();
    (document.text(), depth)
}

#[test]
fn d_and_c_go_to_the_end_of_the_line_with_counts() {
    assert_eq!(edit("abc", 1, "D"), "a");
    assert_eq!(edit("abc\ndef\nghi", 1, "2D"), "a\nghi");
    assert_eq!(edit("abc", 1, "C\u{1b}"), "a");
    assert_eq!(edit("abc\ndef\nghi", 1, "2C\u{1b}"), "a\nghi");
}

#[test]
fn c_and_s_are_one_undo_unit_with_their_inserted_text() {
    let (text, depth) = change("abc", 1, "C", "X");
    assert_eq!(text, "aX");
    assert_eq!(depth, 1, "C and its Insert session are one undo entry");
    let (text, depth) = change("abcd", 1, "s", "X");
    assert_eq!(text, "aXcd");
    assert_eq!(depth, 1);
    let (text, depth) = change("  one\ntwo\nthree", 4, "S", "X");
    assert_eq!(text, "  X\ntwo\nthree");
    assert_eq!(depth, 1);
    let (text, depth) = change("  one\ntwo\nthree", 4, "2S", "X");
    assert_eq!(text, "  X\nthree");
    assert_eq!(depth, 1);
}

#[test]
fn substitute_counts_stop_at_the_line_end() {
    let (text, _) = change("abcd", 1, "s", "X");
    assert_eq!(text, "aXcd");
    let (text, _) = change("abcd", 1, "3s", "X");
    assert_eq!(text, "aX");
    let (text, _) = change("ab\ncd", 1, "5s", "X");
    assert_eq!(text, "aX\ncd");
}

#[test]
fn x_deletes_backward_inside_the_line_only() {
    assert_eq!(edit("abcd", 2, "X"), "acd");
    assert_eq!(edit("abcd", 3, "3X"), "d");
    assert_eq!(edit("abcd", 0, "X"), "abcd");
    assert_eq!(edit("ab\ncd", 3, "9X"), "ab\ncd");
}

#[test]
fn y_is_a_linewise_yank_and_pastes_as_lines() {
    assert_eq!(edit("one\ntwo\nthree", 5, "Yp"), "one\ntwo\ntwo\nthree");
    assert_eq!(
        edit("one\ntwo\nthree", 5, "2Yp"),
        "one\ntwo\ntwo\nthree\nthree"
    );
}

#[test]
fn cw_changes_to_the_end_of_the_word_unlike_dw() {
    let (text, _) = change("one two three", 1, "cw", "X");
    assert_eq!(text, "oX two three");
    let (text, _) = change("one two three", 0, "cw", "X");
    assert_eq!(text, "X two three");
    let (text, _) = change("one two three", 0, "2cw", "X");
    assert_eq!(text, "X three");
}

#[test]
fn cw_on_whitespace_changes_the_whitespace_run() {
    let (text, _) = change("one two", 3, "cw", "X");
    assert_eq!(text, "oneXtwo");
}

#[test]
fn cw_leaves_dw_unchanged() {
    assert_eq!(edit("one two", 0, "dw"), "two");
    assert_eq!(edit("one two", 1, "dw"), "otwo");
}

#[test]
fn cw_handles_end_of_line_and_empty_lines() {
    let (text, _) = change("one", 2, "cw", "X");
    assert_eq!(text, "onX");
    let (text, _) = change("\nnext", 0, "cw", "X");
    assert_eq!(text, "Xnext");
}

#[test]
fn big_word_change_stops_at_the_word_boundary() {
    let (text, _) = change("foo.bar baz", 0, "cW", "X");
    assert_eq!(text, "X baz");
}

#[test]
fn dot_repeats_each_alias_with_its_count_and_text() {
    assert_eq!(edit("abcdef\nghijkl", 1, "Dj."), "a\n");
    assert_eq!(edit("abcdef", 1, "2D."), "");

    let mut document = Document::from_source_text("one two three\nfour five six");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "cw");
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    press(&mut engine, &mut document, &mut selection, "w.");
    assert_eq!(document.text(), "X X three\nfour five six");

    let mut document = Document::from_source_text("abc\ndef");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "s");
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    press(&mut engine, &mut document, &mut selection, "l.");
    assert_eq!(document.text(), "XXc\ndef");
}

#[test]
fn undo_after_change_aliases_restores_the_whole_operation() {
    let mut document = Document::from_source_text("abcdef");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "C");
    type_text(&mut document, &mut selection, "XYZ");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "aXYZ");
    document.undo();
    assert_eq!(document.text(), "abcdef");
}

#[test]
fn empty_line_and_eol_behaviour_is_deterministic() {
    assert_eq!(edit("", 0, "D"), "");
    assert_eq!(edit("abc", 3, "D"), "abc");
    assert_eq!(edit("abc", 2, "X"), "ac");
    let (text, _) = change("", 0, "C", "X");
    assert_eq!(text, "X");
    let (text, _) = change("", 0, "S", "X");
    assert_eq!(text, "X");
}
