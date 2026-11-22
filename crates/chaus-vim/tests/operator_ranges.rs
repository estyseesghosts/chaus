//! Half-open operator ranges for inclusive/exclusive motions.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{Engine, Key};

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

fn delete(source: &str, caret: usize, keys: &str) -> (String, CharOffset) {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    (document.text(), selection.head)
}

#[test]
fn l_is_an_exclusive_motion_for_operators() {
    assert_eq!(delete("abcd", 0, "dl"), ("bcd".to_owned(), CharOffset(0)));
    assert_eq!(delete("abcd", 0, "d2l"), ("cd".to_owned(), CharOffset(0)));
    assert_eq!(delete("abcdef", 0, "2d3l"), (String::new(), CharOffset(0)));
    assert_eq!(delete("abcd", 1, "dh"), ("bcd".to_owned(), CharOffset(0)));
}

#[test]
fn l_at_the_last_character_does_not_join_the_next_line() {
    assert_eq!(
        delete("abc\ndef", 2, "dl"),
        ("ab\ndef".to_owned(), CharOffset(1))
    );
    assert_eq!(
        delete("ab\ncd", 1, "dl"),
        ("a\ncd".to_owned(), CharOffset(0))
    );
}

#[test]
fn inclusive_word_end_and_line_end_ranges_are_unchanged() {
    assert_eq!(
        delete("one two", 0, "de"),
        (" two".to_owned(), CharOffset(0))
    );
    assert_eq!(delete("abc", 0, "d$"), (String::new(), CharOffset(0)));
}

#[test]
fn change_with_l_replaces_exactly_one_grapheme() {
    let mut document = Document::from_source_text("abcd");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "cl");
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "Xbcd");
}

#[test]
fn yank_with_l_copies_exactly_one_grapheme() {
    let mut document = Document::from_source_text("abcd");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "ylp");
    assert_eq!(document.text(), "aabcd");
}

#[test]
fn multi_codepoint_graphemes_count_as_one_l_motion() {
    let mut document = Document::from_source_text("a😀b");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "dl");
    assert_eq!(document.text(), "ab");
}

#[test]
fn empty_line_and_single_character_lines_are_safe() {
    assert_eq!(
        delete("\nnext", 0, "dl"),
        ("\nnext".to_owned(), CharOffset(0))
    );
    assert_eq!(delete("a", 0, "dl"), (String::new(), CharOffset(0)));
}
