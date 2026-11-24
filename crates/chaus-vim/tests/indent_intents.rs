//! Normal `>>`/`<<` and Visual `>`/`<` produce editor intents; indentation
//! policy stays in the editor.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, LineIndex, Selection};
use chaus_vim::{Engine, Key, Mode, Outcome};

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
fn double_greater_indents_the_current_line() {
    let mut document = Document::from_source_text("one\ntwo");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Handled,
        "the first > only starts a prefix"
    );
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Indent {
            outdent: false,
            first_line: LineIndex(0),
            last_line: LineIndex(0),
        }
    );
    assert_eq!(document.text(), "one\ntwo");
}

#[test]
fn a_count_indents_that_many_lines() {
    let mut document = Document::from_source_text("one\ntwo\nthree\nfour");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "3");
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Indent {
            outdent: false,
            first_line: LineIndex(0),
            last_line: LineIndex(2),
        }
    );
}

#[test]
fn double_less_outdents_the_current_line() {
    let mut document = Document::from_source_text("    one\n\ttwo");
    let mut selection = Selection::caret(CharOffset(6));
    let mut engine = Engine::new(Vec::new());
    let outcome = engine.handle_key(Key::Char('<'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    let outcome = engine.handle_key(Key::Char('<'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Indent {
            outdent: true,
            first_line: LineIndex(0),
            last_line: LineIndex(0),
        }
    );
    assert_eq!(document.text(), "    one\n\ttwo");
}

#[test]
fn an_unknown_prefix_after_indentation_is_reported_and_cleared() {
    let mut document = Document::from_source_text("abc");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, ">");
    let outcome = engine.handle_key(Key::Char('x'), &mut document, &mut selection);
    assert!(matches!(outcome, Outcome::Message(_)));
    let outcome = engine.handle_key(Key::Char('x'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Changed);
    assert_eq!(document.text(), "bc");
}

#[test]
fn dot_repeats_a_normal_indentation_change() {
    let mut document = Document::from_source_text("one\ntwo");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert!(matches!(outcome, Outcome::Indent { .. }));
    let _ = engine.handle_key(Key::Char('j'), &mut document, &mut selection);
    let outcome = engine.handle_key(Key::Char('.'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Indent {
            outdent: false,
            first_line: LineIndex(1),
            last_line: LineIndex(1),
        },
        "the repeated command should target the second line"
    );
    assert_eq!(engine.mode(), Mode::Normal);
}
