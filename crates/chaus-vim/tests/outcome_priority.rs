//! Outcome priority when one key expands into several commands.
//!
//! A change wins over a later rejection message, and a rejection message wins
//! over a later Ex command. These are the editor-visible results of merging
//! per-step outcomes.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, Selection};
use chaus_core::settings::RawMapping;
use chaus_vim::{parse_mappings, Engine, ExCommand, Key, Outcome};

fn fixture(text: &str, rhs: &str) -> (Engine, Document, Selection) {
    let (mappings, diagnostics) = parse_mappings(&[RawMapping::new("q", rhs)], '\\');
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    (
        Engine::new(mappings),
        Document::from_source_text(text),
        Selection::caret(CharOffset::ZERO),
    )
}

#[test]
fn change_outranks_a_later_message() {
    let (mut engine, mut document, mut selection) = fixture("abc", "xQ");
    let outcome = engine.handle_key(Key::Char('q'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Changed);
    assert_eq!(document.text(), "bc");
}

#[test]
fn change_outranks_an_earlier_message() {
    let (mut engine, mut document, mut selection) = fixture("abc", "Qx");
    let outcome = engine.handle_key(Key::Char('q'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Changed);
    assert_eq!(document.text(), "bc");
}

#[test]
fn message_outranks_a_later_ex_command() {
    let (mut engine, mut document, mut selection) = fixture("abc", "Q:w<CR>");
    let outcome = engine.handle_key(Key::Char('q'), &mut document, &mut selection);
    assert!(matches!(outcome, Outcome::Message(_)), "{outcome:?}");
    assert_ne!(
        outcome,
        Outcome::Ex(ExCommand::Write {
            force: false,
            path: None
        })
    );
}

#[test]
fn dot_repeat_of_insert_reports_changed() {
    let mut engine = Engine::new(Vec::new());
    let mut document = Document::from_source_text("hello world");
    let mut selection = Selection::caret(CharOffset::ZERO);

    engine.handle_key(Key::Char('i'), &mut document, &mut selection);
    let before = selection;
    let caret = CharOffset(1);
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(CharOffset::ZERO, "X"),
            before,
            Selection::caret(caret),
        ))
        .expect("insert");
    selection = Selection::caret(caret);
    engine.handle_key(Key::Escape, &mut document, &mut selection);

    let outcome = engine.handle_key(Key::Char('.'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Changed);
    assert_eq!(document.text(), "XXhello world");
}
