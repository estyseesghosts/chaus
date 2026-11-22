//! Search, dot-repeat, mapping resolution and count edge cases.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, Selection};
use chaus_core::settings::RawMapping;
use chaus_vim::{parse_mappings, Engine, Key, Outcome};

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

#[test]
fn search_starts_after_the_caret_and_n_wraps() {
    let mut document = Document::from_source_text("one two one");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "/one\n");
    assert_eq!(selection.head, CharOffset(8), "skip the match at the caret");

    press(&mut engine, &mut document, &mut selection, "n");
    assert_eq!(
        selection.head,
        CharOffset::ZERO,
        "n wraps to the first match"
    );

    press(&mut engine, &mut document, &mut selection, "N");
    assert_eq!(
        selection.head,
        CharOffset(8),
        "N searches backwards and wraps"
    );
}

#[test]
fn dot_repeats_delete_with_its_motion() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "dw");
    assert_eq!(document.text(), "two three");

    press(&mut engine, &mut document, &mut selection, ".");
    assert_eq!(document.text(), "three");
}

#[test]
fn dot_repeats_change_with_inserted_text() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "ciw");
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "X two three");

    press(&mut engine, &mut document, &mut selection, "w.");
    assert_eq!(document.text(), "X X three");
}

#[test]
fn dot_repeat_keeps_the_original_count() {
    let mut document = Document::from_source_text("abcdef");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "2x");
    assert_eq!(document.text(), "cdef");
    press(&mut engine, &mut document, &mut selection, ".");
    assert_eq!(document.text(), "ef");
}

#[test]
fn paste_honors_its_count() {
    let mut document = Document::from_source_text("a\n");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "yy3p");
    assert_eq!(document.text(), "a\na\na\na\n");
}

#[test]
fn dot_repeats_paste() {
    let mut document = Document::from_source_text("a\nb\n");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "yyp");
    assert_eq!(document.text(), "a\na\nb\n");

    press(&mut engine, &mut document, &mut selection, ".");
    assert_eq!(document.text(), "a\na\na\nb\n");
}

#[test]
fn yank_does_not_replace_the_repeatable_change() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "dw");
    press(&mut engine, &mut document, &mut selection, "vly");
    press(&mut engine, &mut document, &mut selection, ".");
    assert_eq!(document.text(), "three");
}

#[test]
fn mapping_targets_are_not_resolved_again() {
    let (mappings, diagnostics) = parse_mappings(
        &[RawMapping::new("a", "l"), RawMapping::new("l", "h")],
        '\\',
    );
    assert!(diagnostics.is_empty());
    let mut engine = Engine::new(mappings);
    let mut document = Document::from_source_text("abc");
    let mut selection = Selection::caret(CharOffset::ZERO);

    let outcome = engine.handle_key(Key::Char('a'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(
        selection.head,
        CharOffset(1),
        "`a` expands to `l`; `l` must not expand to `h` a second time"
    );
}

#[test]
fn replace_characters_treats_one_grapheme_as_one_replacement() {
    let mut document = Document::from_source_text("a😀b");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "rX");
    assert_eq!(document.text(), "aXb");
}

#[test]
fn delete_with_l_motion_does_not_join_lines() {
    let mut document = Document::from_source_text("ab\ncd");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "dl");
    assert_eq!(document.text(), "a\ncd");
}

#[test]
fn delete_characters_stops_at_the_line_break() {
    let mut document = Document::from_source_text("ab\ncd");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "5x");
    assert_eq!(document.text(), "a\ncd");
}

#[test]
fn replace_characters_does_not_cross_a_line_break() {
    let mut document = Document::from_source_text("abc\ndef");
    let mut selection = Selection::caret(CharOffset(2));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "3rX");
    assert_eq!(document.text(), "abc\ndef");

    selection = Selection::caret(CharOffset(1));
    press(&mut engine, &mut document, &mut selection, "2rX");
    assert_eq!(document.text(), "aXX\ndef");
}

#[test]
fn bare_insert_does_not_move_the_caret() {
    let mut document = Document::from_source_text("abc");
    let mut selection = Selection::caret(CharOffset(1));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "i\u{1b}");
    assert_eq!(document.text(), "abc");
    assert_eq!(selection.head, CharOffset(1));
}

#[test]
fn change_line_on_an_empty_line_enters_insert() {
    let mut document = Document::from_source_text("a\n\nb");
    let mut selection = Selection::caret(CharOffset(2));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "cc");
    type_text(&mut document, &mut selection, "Z");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "a\nZ\nb");
}

#[test]
fn change_line_keeps_leading_indentation() {
    let mut document = Document::from_source_text("    abc\nx");
    let mut selection = Selection::caret(CharOffset(4));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "cc");
    type_text(&mut document, &mut selection, "Z");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "    Z\nx");
}

#[test]
fn cancel_prefix_drops_a_pending_operator() {
    let mut document = Document::from_source_text("one two");
    let mut selection = Selection::caret(CharOffset::ZERO);
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "d");
    engine.cancel_prefix();
    press(&mut engine, &mut document, &mut selection, "w");

    assert_eq!(document.text(), "one two");
    assert_eq!(selection.head, CharOffset(4), "`w` starts the next word");
}

#[test]
fn zero_is_a_motion_after_an_operator() {
    let mut document = Document::from_source_text("abc\ndef");
    let mut selection = Selection::caret(CharOffset(5));
    let mut engine = Engine::new(Vec::new());

    press(&mut engine, &mut document, &mut selection, "d0");
    assert_eq!(document.text(), "abc\nef");
}

#[test]
fn literal_mappings_respect_the_key_cap() {
    let (mappings, diagnostics) = parse_mappings(&[RawMapping::new("abcdefghi", "x")], '\\');
    assert!(mappings.is_empty());
    assert_eq!(diagnostics.len(), 1);
}
