//! Viewport intents and the exit commands that do not edit text.
//!
//! `chaus-vim` reports semantic intents; these tests deliberately never build
//! an editor or a viewport size.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::ex_command::{parse, ExParseError};
use chaus_vim::{Engine, ExCommand, Key, Outcome, ViewportCommand, ViewportIntent};

fn press(
    engine: &mut Engine,
    document: &mut Document,
    selection: &mut Selection,
    key: Key,
) -> Outcome {
    engine.handle_key(key, document, selection)
}

fn press_keys(keys: &str) -> Outcome {
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    let mut outcome = Outcome::Handled;
    for character in keys.chars() {
        let key = match character {
            '\u{4}' => Key::Control('d'),
            '\u{15}' => Key::Control('u'),
            other => Key::Char(other),
        };
        outcome = press(&mut engine, &mut document, &mut selection, key);
    }
    outcome
}

fn viewport(outcome: Outcome) -> ViewportIntent {
    match outcome {
        Outcome::Viewport(intent) => intent,
        other => panic!("expected a viewport intent, got {other:?}"),
    }
}

#[test]
fn zz_zt_zb_request_semantic_placements() {
    assert_eq!(
        viewport(press_keys("zz")),
        ViewportIntent {
            command: ViewportCommand::Center,
            count: 1,
        }
    );
    assert_eq!(
        viewport(press_keys("zt")),
        ViewportIntent {
            command: ViewportCommand::Top,
            count: 1,
        }
    );
    assert_eq!(
        viewport(press_keys("zb")),
        ViewportIntent {
            command: ViewportCommand::Bottom,
            count: 1,
        }
    );
}

#[test]
fn half_page_commands_carry_their_count() {
    assert_eq!(
        viewport(press_keys("\u{4}")),
        ViewportIntent {
            command: ViewportCommand::HalfPageDown,
            count: 1,
        }
    );
    assert_eq!(
        viewport(press_keys("3\u{4}")),
        ViewportIntent {
            command: ViewportCommand::HalfPageDown,
            count: 3,
        }
    );
    assert_eq!(
        viewport(press_keys("\u{15}")),
        ViewportIntent {
            command: ViewportCommand::HalfPageUp,
            count: 1,
        }
    );
    assert_eq!(
        viewport(press_keys("2\u{15}")),
        ViewportIntent {
            command: ViewportCommand::HalfPageUp,
            count: 2,
        }
    );
}

#[test]
fn unknown_z_commands_report_and_leave_no_pending_state() {
    let mut document = Document::from_source_text("one\ntwo");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    let outcome = press(&mut engine, &mut document, &mut selection, Key::Char('z'));
    assert_eq!(outcome, Outcome::Handled);
    let outcome = press(&mut engine, &mut document, &mut selection, Key::Char('x'));
    assert!(matches!(outcome, Outcome::Message(_)));
    // The next key is a fresh Normal command, proving `z` was cleared.
    let outcome = press(&mut engine, &mut document, &mut selection, Key::Char('j'));
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(selection.head, CharOffset(4));
}

#[test]
fn z_prefix_does_not_swallow_an_operator() {
    let mut document = Document::from_source_text("one two\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    let _ = press(&mut engine, &mut document, &mut selection, Key::Char('z'));
    let _ = press(&mut engine, &mut document, &mut selection, Key::Char('d'));
    let _ = press(&mut engine, &mut document, &mut selection, Key::Char('w'));
    assert_eq!(document.text(), "one two\nthree");
}

#[test]
fn zz_ignores_a_count_but_consumes_it() {
    let outcome = press_keys("2zz");
    assert_eq!(
        viewport(outcome),
        ViewportIntent {
            command: ViewportCommand::Center,
            count: 1,
        }
    );
}

#[test]
fn zz_and_zq_are_typed_exits_not_text_changes() {
    assert_eq!(press_keys("ZZ"), Outcome::Ex(ExCommand::Exit));
    assert_eq!(
        press_keys("ZQ"),
        Outcome::Ex(ExCommand::Quit { force: true })
    );
    let outcome = press_keys("Za");
    assert!(matches!(outcome, Outcome::Message(_)));
}

#[test]
fn write_quit_all_has_typed_spellings() {
    for input in ["wqa", "wqall", ":wqa", "  wqall  "] {
        assert_eq!(parse(input), Ok(ExCommand::WriteQuitAll), "input {input:?}");
    }
    assert_eq!(parse("wq"), Ok(ExCommand::WriteQuit));
    assert_eq!(parse("qa"), Ok(ExCommand::QuitAll { force: false }));
    assert_eq!(parse("qa!"), Ok(ExCommand::QuitAll { force: true }));
}

#[test]
fn write_quit_all_rejects_force_and_arguments() {
    assert_eq!(parse("wqa!"), Err(ExParseError::UnexpectedForce("wqall")));
    assert_eq!(
        parse("wqa! now"),
        Err(ExParseError::UnexpectedForce("wqall"))
    );
    assert_eq!(
        parse("wqa now"),
        Err(ExParseError::UnexpectedArgument("wqall"))
    );
    assert_eq!(parse("wqall!"), Err(ExParseError::UnexpectedForce("wqall")));
}
