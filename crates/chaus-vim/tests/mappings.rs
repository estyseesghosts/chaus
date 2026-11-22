//! Rc mapping parsing and non-recursive expansion.

use chaus_core::settings::RawMapping;
use chaus_vim::{parse_mappings, Engine, Key, Mode, Outcome};

fn raw(lhs: &str, rhs: &str) -> RawMapping {
    RawMapping::new(lhs, rhs)
}

#[test]
fn expands_leader_and_control_tokens() {
    let (mappings, diagnostics) = parse_mappings(&[raw("<leader>w", ":w<CR>")], ' ');
    assert!(diagnostics.is_empty());
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].keys(), &[Key::Char(' '), Key::Char('w')]);
    assert_eq!(
        mappings[0].rhs(),
        &[Key::Char(':'), Key::Char('w'), Key::Enter]
    );
}

#[test]
fn rejects_duplicate_and_unsupported_mappings() {
    let (mappings, diagnostics) = parse_mappings(
        &[
            raw("a", "b"),
            raw("a", "c"),
            raw("z", "<Bogus>"),
            raw("j", ""),
        ],
        '\\',
    );
    assert_eq!(mappings.len(), 1);
    assert_eq!(diagnostics.len(), 3);
}

#[test]
fn mapped_sequence_reaches_the_ex_command() {
    let (mappings, diagnostics) = parse_mappings(&[raw("<leader>w", ":w<CR>")], ' ');
    assert!(diagnostics.is_empty());
    let mut engine = Engine::new(mappings);
    let mut document = chaus_core::document::Document::new();
    let mut selection = chaus_core::position::Selection::caret(Default::default());

    assert_eq!(
        engine.handle_key(Key::Char(' '), &mut document, &mut selection),
        Outcome::Handled,
        "the leader alone is only a mapping prefix"
    );

    let outcome = engine.handle_key(Key::Char('w'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Command("w".to_owned()),
        "the mapped keys must arrive as one Ex command"
    );
    assert_eq!(engine.mode(), Mode::Normal);

    engine.handle_key(Key::Char('a'), &mut document, &mut selection);
    assert_eq!(
        engine.mode(),
        Mode::Insert,
        "a key after the mapping is interpreted normally"
    );
}

#[test]
fn mapping_waits_for_a_complete_prefix() {
    let (mappings, _) = parse_mappings(&[raw("jj", "<Esc>")], '\\');
    let mut engine = Engine::new(mappings);
    let mut document = chaus_core::document::Document::new();
    let mut selection = chaus_core::position::Selection::caret(Default::default());
    let _ = engine.handle_key(Key::Char('j'), &mut document, &mut selection);
    assert_eq!(selection.head, Default::default(), "first j is a prefix");
    let _ = engine.handle_key(Key::Char('j'), &mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Normal);
}
