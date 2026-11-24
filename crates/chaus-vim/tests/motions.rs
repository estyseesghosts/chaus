//! Motion parity: Normal movement, operator-pending movement and counts all
//! resolve through one semantic motion model.

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

fn caret_after(source: &str, caret: usize, keys: &str) -> CharOffset {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    selection.head
}

fn edit(source: &str, caret: usize, keys: &str) -> String {
    let mut document = Document::from_source_text(source);
    let mut selection = Selection::caret(CharOffset(caret));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, keys);
    document.text()
}

#[test]
fn horizontal_and_vertical_movement_respect_counts_and_line_edges() {
    let source = "abc\ndefgh\nij";
    assert_eq!(caret_after(source, 0, "2l"), CharOffset(2));
    assert_eq!(caret_after(source, 0, "9l"), CharOffset(2));
    assert_eq!(caret_after(source, 6, "2h"), CharOffset(4));
    assert_eq!(caret_after(source, 6, "9h"), CharOffset(4));
    assert_eq!(caret_after(source, 0, "2j"), CharOffset(10));
    assert_eq!(caret_after(source, 4, "2k"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "2$"), CharOffset(8));
    assert_eq!(caret_after(source, 8, "0"), CharOffset(4));
    assert_eq!(caret_after("  abc", 0, "^"), CharOffset(2));
}

#[test]
fn first_and_last_line_motions_distinguish_plain_g_from_a_count() {
    let source = "one\ntwo\nthree\nfour";
    assert_eq!(caret_after(source, 5, "G"), CharOffset(14));
    assert_eq!(caret_after(source, 5, "gg"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "3G"), CharOffset(8));
    assert_eq!(caret_after(source, 14, "2gg"), CharOffset(4));
    assert_eq!(caret_after(source, 0, "1G"), CharOffset(0));
}

#[test]
fn lowercase_words_distinguish_punctuation_runs() {
    let source = "foo.bar/baz qux";
    assert_eq!(caret_after(source, 0, "w"), CharOffset(3));
    assert_eq!(caret_after(source, 0, "2w"), CharOffset(4));
    assert_eq!(caret_after(source, 0, "e"), CharOffset(2));
    assert_eq!(caret_after(source, 0, "2e"), CharOffset(3));
    assert_eq!(caret_after(source, 0, "3e"), CharOffset(6));
    assert_eq!(caret_after(source, 12, "b"), CharOffset(8));
    assert_eq!(caret_after(source, 12, "2b"), CharOffset(7));
}

#[test]
fn big_words_are_non_whitespace_runs() {
    let source = "foo.bar/baz qux";
    assert_eq!(caret_after(source, 0, "W"), CharOffset(12));
    assert_eq!(caret_after(source, 0, "2W"), CharOffset(14));
    assert_eq!(caret_after(source, 0, "E"), CharOffset(10));
    assert_eq!(caret_after(source, 10, "B"), CharOffset(0));
    assert_eq!(caret_after(source, 12, "B"), CharOffset(0));
}

#[test]
fn ge_moves_backward_to_the_previous_word_end() {
    let source = "one two three";
    assert_eq!(caret_after(source, 8, "ge"), CharOffset(6));
    assert_eq!(caret_after(source, 7, "ge"), CharOffset(6));
    assert_eq!(caret_after(source, 6, "ge"), CharOffset(2));
}

#[test]
fn find_and_till_motions_fail_without_moving() {
    let source = "a,b,c";
    assert_eq!(caret_after(source, 0, "f,"), CharOffset(1));
    assert_eq!(caret_after(source, 0, "2f,"), CharOffset(3));
    assert_eq!(caret_after(source, 4, "F,"), CharOffset(3));
    assert_eq!(caret_after(source, 0, "t,"), CharOffset(0));
    assert_eq!(caret_after(source, 0, "2t,"), CharOffset(2));
    assert_eq!(caret_after(source, 4, "T,"), CharOffset(4));
    assert_eq!(caret_after(source, 1, "fx"), CharOffset(1));
    assert_eq!(caret_after(source, 1, "2fx"), CharOffset(1));
}

#[test]
fn semicolon_and_comma_repeat_the_last_find_in_both_directions() {
    let source = "a,b,c,d";
    assert_eq!(caret_after(source, 0, "f,;"), CharOffset(3));
    assert_eq!(caret_after(source, 0, "2f,,"), CharOffset(1));
    assert_eq!(caret_after(source, 0, "2f,;"), CharOffset(5));
    assert_eq!(caret_after(source, 6, "2F,;"), CharOffset(1));
    assert_eq!(caret_after(source, 6, "2F,,"), CharOffset(5));
}

#[test]
fn distinct_find_motions_keep_their_own_repeat_direction() {
    let source = "a:b:c:d";
    assert_eq!(caret_after(source, 6, "2F:;"), CharOffset(1));
    assert_eq!(caret_after(source, 6, "2F:,"), CharOffset(5));
    assert_eq!(caret_after(source, 0, "f:;"), CharOffset(3));
    assert_eq!(caret_after(source, 0, "f:,"), CharOffset(1));
}

#[test]
fn operators_use_the_same_word_ranges_as_movement() {
    assert_eq!(edit("one two three", 0, "dw"), "two three");
    assert_eq!(edit("one two three", 0, "d2w"), "three");
    assert_eq!(edit("one two three", 0, "2dw"), "three");
    assert_eq!(
        edit("one two three four five six seven", 0, "2d3w"),
        "seven"
    );
    assert_eq!(edit("abc", 0, "d$"), "");
    assert_eq!(edit("abc\ndef", 0, "dj"), "");
    assert_eq!(edit("one\ntwo\nthree", 0, "yG"), "one\ntwo\nthree");
}

#[test]
fn big_word_operators_work() {
    assert_eq!(edit("foo.bar/baz qux", 0, "dW"), "qux");
    assert_eq!(edit("foo.bar/baz qux", 0, "dE"), " qux");
    assert_eq!(edit("foo.bar/baz qux", 12, "dB"), "qux");
}

#[test]
fn find_operators_are_inclusive_of_the_till_target() {
    assert_eq!(edit("a,b,c", 0, "df,"), "b,c");
    assert_eq!(edit("a,b,c", 0, "dt,"), ",b,c");
    assert_eq!(edit("abc(def)", 0, "df)"), "");
    assert_eq!(edit("a:b:c", 6, "yF:"), "a:b:c");
}

#[test]
fn percent_matches_nested_and_adjacent_delimiters() {
    assert_eq!(caret_after("a(b[c]d)e", 1, "%"), CharOffset(7));
    assert_eq!(caret_after("a(b[c]d)e", 7, "%"), CharOffset(1));
    assert_eq!(edit("a(b[c]d)e", 1, "d%"), "ae");
    assert_eq!(caret_after("a(b[c]d)e", 3, "%"), CharOffset(5));
}

#[test]
fn percent_does_not_jump_from_an_unmatched_delimiter() {
    assert_eq!(caret_after("a(b", 1, "%"), CharOffset(1));
    assert_eq!(edit("a(b", 1, "d%"), "a(b");
    assert_eq!(caret_after("plain", 0, "%"), CharOffset(0));
}

#[test]
fn gg_and_g_work_in_operator_pending_mode() {
    let source = "one\ntwo\nthree\nfour\n";
    assert_eq!(edit(source, 8, "dgg"), "four\n");
    assert_eq!(edit(source, 0, "dG"), "");
    assert_eq!(edit(source, 14, "2dgg"), "one\n");
    assert_eq!(edit(source, 14, "d2gg"), "one\n");
    let yanked = {
        let mut document = Document::from_source_text(source);
        let mut selection = Selection::caret(CharOffset(0));
        let mut engine = Engine::new(Vec::new());
        press(&mut engine, &mut document, &mut selection, "y10Gp");
        document.text()
    };
    assert_eq!(yanked.matches("one\n").count(), 2);
}

#[test]
fn cgg_enters_insert_and_undoes_as_one_operation() {
    let mut document = Document::from_source_text("one\ntwo\nthree\nfour\n");
    let mut selection = Selection::caret(CharOffset(9));
    let mut engine = Engine::new(Vec::new());
    press(&mut engine, &mut document, &mut selection, "cgg");
    assert_eq!(engine.mode(), Mode::Insert);
    type_text(&mut document, &mut selection, "X");
    press(&mut engine, &mut document, &mut selection, "\u{1b}");
    assert_eq!(document.text(), "X\nfour\n");
    assert_eq!(document.undo_depth(), 1);
    document.undo();
    assert_eq!(document.text(), "one\ntwo\nthree\nfour\n");
}

#[test]
fn an_invalid_g_continuation_does_not_poison_the_next_command() {
    let source = "abc";
    assert_eq!(caret_after(source, 0, "dgx"), CharOffset(0));
    assert_eq!(edit(source, 0, "dgxx"), "bc");
    assert_eq!(caret_after(source, 0, "gx"), CharOffset(0));
    assert_eq!(edit(source, 0, "gxx"), "bc");
}

#[test]
fn escape_cancels_an_incomplete_g_sequence() {
    let source = "abc";
    assert_eq!(edit(source, 0, "d\u{1b}x"), "bc");
    assert_eq!(edit(source, 0, "g\u{1b}x"), "bc");
}

#[test]
fn unicode_graphemes_count_as_one_motion() {
    assert_eq!(caret_after("a😀b", 0, "l"), CharOffset(1));
    assert_eq!(edit("a😀b", 0, "dl"), "😀b");
    assert_eq!(caret_after("éé", 0, "e"), CharOffset(1));
}
