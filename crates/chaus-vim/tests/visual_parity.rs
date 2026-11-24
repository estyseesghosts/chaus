//! Visual mode uses the same motion resolver and text objects as Normal and
//! operator-pending mode, including reversed selections.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{Engine, Key, Mode, Outcome, ViewportCommand, ViewportIntent};

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

fn selection_after(source: &str, caret: usize, keys: &str) -> (String, Selection) {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    (document.text(), selection)
}

#[test]
fn visual_inner_and_outer_word_objects_match_operator_pending() {
    assert_eq!(
        selection_after("one two", 0, "viwd").0,
        " two",
        "viw selects one word"
    );
    assert_eq!(
        selection_after("one two", 0, "vawd").0,
        "two",
        "vaw includes the following space"
    );
}

#[test]
fn visual_quote_and_bracket_objects_keep_or_remove_delimiters() {
    assert_eq!(
        selection_after("say \"hi\" now", 5, "vi\"d").0,
        "say \"\" now"
    );
    assert_eq!(selection_after("say \"hi\" now", 5, "va\"d").0, "say  now");
    assert_eq!(
        selection_after("a(b[c]d)e", 4, "vi[d").0,
        "a(b[]d)e",
        "inner bracket object uses the selected pair"
    );
    assert_eq!(selection_after("a(b[c]d)e", 3, "va(d").0, "ae");
}

#[test]
fn visual_movement_preserves_anchor_head_direction() {
    let (text, selection) = selection_after("abcdef", 2, "vlld");
    assert_eq!(text, "abf");
    assert_eq!(selection.head, CharOffset(2));
    assert!(!selection.reversed());

    let (text, selection) = selection_after("abcdef", 2, "vhhd");
    assert_eq!(text, "def");
    assert_eq!(selection.head, CharOffset(0));
    assert!(
        !selection.reversed(),
        "finishing Visual collapses to a caret"
    );
}

#[test]
fn visual_o_swaps_the_two_ends_after_several_motions() {
    let mut document = Document::from_source_text("one two three");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "vww");
    let before = selection;
    press(&mut engine, &mut document, &mut selection, "o");
    assert_eq!(selection.anchor, before.head);
    assert_eq!(selection.head, before.anchor);
    press(&mut engine, &mut document, &mut selection, "o");
    assert_eq!(selection, before);
}

#[test]
fn visual_line_stays_linewise_for_movement_and_text_objects() {
    assert_eq!(
        selection_after("one\ntwo\nthree", 0, "Vjd").0,
        "three",
        "Vj deletes two whole lines"
    );
    let (text, _) = selection_after("one two\nthree", 0, "Viwd");
    assert_eq!(
        text, "three",
        "a text object inside Visual-line must not turn it characterwise"
    );
}

#[test]
fn new_word_find_and_percent_motions_work_in_visual_mode() {
    assert_eq!(
        selection_after("foo.bar/baz qux", 0, "vWd").0,
        "ux",
        "Visual includes the motion target character"
    );
    assert_eq!(
        selection_after("a,b,c", 0, "vf,d").0,
        "b,c",
        "vf includes the find target"
    );
    assert_eq!(
        selection_after("a(b[c]d)e", 0, "v%d").0,
        "e",
        "v% selects through the matching delimiter"
    );
    let (_, selection) = selection_after("plain", 0, "v%");
    assert_eq!(
        selection.head,
        CharOffset(0),
        "an unmatched % does not move"
    );
    assert_eq!(
        selection_after("plain text", 2, "vi\"\u{1b}").0,
        "plain text",
        "an unmatched text object leaves the text alone"
    );
}

#[test]
fn unmatched_visual_text_object_leaves_the_selection_intact() {
    let mut document = Document::from_source_text("plain text");
    let mut selection = Selection::caret(CharOffset(2));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "v");
    let before = selection;
    press(&mut engine, &mut document, &mut selection, "i\"");
    assert_eq!(engine.mode(), Mode::VisualChar);
    assert_eq!(selection, before);
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(engine.mode(), Mode::Normal);
}

#[test]
fn visual_indent_returns_a_line_intent_and_leaves_visual_mode() {
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "Vj");
    let outcome = engine.handle_key(Key::Char('>'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Indent {
            outdent: false,
            first_line: chaus_core::position::LineIndex(0),
            last_line: chaus_core::position::LineIndex(1),
        }
    );
    assert_eq!(engine.mode(), Mode::Normal);
    assert_eq!(
        document.text(),
        "one\ntwo\nthree",
        "the engine does not indent"
    );
}

#[test]
fn visual_change_enters_insert_and_undo_restores_the_selection_text() {
    let mut document = Document::from_source_text("one two");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "vec");
    assert_eq!(engine.mode(), Mode::Insert);
    assert_eq!(document.text(), " two");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.undo_depth(), 1);
    document.undo();
    assert_eq!(document.text(), "one two");
}

#[test]
fn visual_half_page_commands_keep_the_mode_and_emit_viewport_intents() {
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "v");

    let outcome = engine.handle_key(Key::Control('d'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Viewport(ViewportIntent {
            command: ViewportCommand::HalfPageDown,
            count: 1,
        })
    );
    assert_eq!(engine.mode(), Mode::VisualChar);

    press(&mut engine, &mut document, &mut selection, "2");
    let outcome = engine.handle_key(Key::Control('u'), &mut document, &mut selection);
    assert_eq!(
        outcome,
        Outcome::Viewport(ViewportIntent {
            command: ViewportCommand::HalfPageUp,
            count: 2,
        })
    );
    assert_eq!(engine.mode(), Mode::VisualChar);
    assert_eq!(document.text(), "one\ntwo\nthree");
}

#[test]
fn visual_yank_populates_the_linewise_register() {
    let mut document = Document::from_source_text("one\ntwo\nthree");
    let mut selection = Selection::caret(CharOffset(0));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "Vjy");
    assert_eq!(engine.mode(), Mode::Normal);
    assert_eq!(document.text(), "one\ntwo\nthree");
    press(&mut engine, &mut document, &mut selection, "p");
    assert_eq!(document.text(), "one\none\ntwo\ntwo\nthree");
}
