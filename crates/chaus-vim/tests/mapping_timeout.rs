//! Timed-out mapping prefixes must fall back to native Vim commands.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_core::settings::RawMapping;
use chaus_vim::{parse_mappings, Engine, Key, Mode, Outcome};

fn engine(pairs: &[(&str, &str)]) -> Engine {
    let raw: Vec<RawMapping> = pairs
        .iter()
        .map(|(left, right)| RawMapping::new(*left, *right))
        .collect();
    let (mappings, diagnostics) = parse_mappings(&raw, '\\');
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Engine::new(mappings)
}

#[test]
fn a_prefix_that_is_also_a_native_command_still_runs_it() {
    let mut engine = engine(&[("jj", "<Esc>")]);
    let mut document = Document::from_source_text("one\ntwo");
    let mut selection = Selection::caret(CharOffset::ZERO);

    assert_eq!(
        engine.handle_key(Key::Char('j'), &mut document, &mut selection),
        Outcome::Handled
    );
    assert_eq!(selection.head, CharOffset::ZERO, "the first j is a prefix");
    assert!(engine.mapping_prefix_pending());

    assert_eq!(
        engine.resolve_mapping_timeout(&mut document, &mut selection),
        Outcome::Handled
    );
    assert_eq!(
        selection.head,
        CharOffset(4),
        "the timed-out j must still move down one line"
    );
    assert!(!engine.mapping_prefix_pending());
}

#[test]
fn native_gg_still_works_after_a_timed_out_g_prefix() {
    let mut engine = engine(&[("ggX", "x")]);
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(8));

    let _ = engine.handle_key(Key::Char('g'), &mut document, &mut selection);
    assert!(engine.mapping_prefix_pending());
    let _ = engine.resolve_mapping_timeout(&mut document, &mut selection);

    let outcome = engine.handle_key(Key::Char('g'), &mut document, &mut selection);
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(
        selection.head,
        CharOffset::ZERO,
        "the replayed g must leave the native G pending"
    );
}

#[test]
fn a_timed_out_prefix_that_is_a_native_insert_command_enters_insert() {
    let mut engine = engine(&[("abc", "iX")]);
    let mut document = Document::from_source_text("one");
    let mut selection = Selection::caret(CharOffset::ZERO);

    let _ = engine.handle_key(Key::Char('a'), &mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Normal);
    let _ = engine.resolve_mapping_timeout(&mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Insert, "a must append as in Vim");
    let _ = engine.handle_key(Key::Escape, &mut document, &mut selection);
    assert_eq!(engine.mode(), Mode::Normal);
}

#[test]
fn a_leader_prefix_with_no_native_meaning_is_reported_not_dropped() {
    let mut engine = engine(&[("<leader>w", ":w<CR>")]);
    let mut document = Document::from_source_text("one");
    let mut selection = Selection::caret(CharOffset::ZERO);

    let _ = engine.handle_key(Key::Char('\\'), &mut document, &mut selection);
    assert!(engine.mapping_prefix_pending());
    match engine.resolve_mapping_timeout(&mut document, &mut selection) {
        Outcome::Message(message) => {
            assert!(message.contains("unknown command"), "{message}");
        }
        other => panic!("expected an explicit rejection, got {other:?}"),
    }
}
