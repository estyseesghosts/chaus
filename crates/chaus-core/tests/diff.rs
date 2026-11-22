//! Prefix/suffix change spans used by the formatter path.

use chaus_core::diff::{common_change, TextChange};
use chaus_core::position::{CharOffset, CharRange};

fn span(old: CharRange, insert: &str) -> TextChange {
    TextChange {
        old,
        insert: insert.to_owned(),
    }
}

#[test]
fn equal_texts_have_no_change() {
    assert_eq!(common_change("", ""), None);
    assert_eq!(common_change("abc", "abc"), None);
    assert_eq!(common_change("one\r\ntwo", "one\r\ntwo"), None);
}

#[test]
fn an_empty_document_becomes_the_inserted_text() {
    assert_eq!(
        common_change("", "hello"),
        Some(span(CharRange::empty(CharOffset(0)), "hello"))
    );
}

#[test]
fn inserting_one_word_keeps_the_surrounding_text_out_of_the_change() {
    assert_eq!(
        common_change("hello world", "hello brave world"),
        Some(span(CharRange::empty(CharOffset(6)), "brave "))
    );
}

#[test]
fn changing_one_character_reports_a_one_character_replacement() {
    assert_eq!(
        common_change("aXb", "aYb"),
        Some(span(CharRange::new(CharOffset(1), CharOffset(2)), "Y"))
    );
}

#[test]
fn a_pure_deletion_has_an_empty_insert() {
    assert_eq!(
        common_change("abc", "ab"),
        Some(span(CharRange::new(CharOffset(2), CharOffset(3)), ""))
    );
    assert_eq!(
        common_change("prefix suffix", "prefix "),
        Some(span(CharRange::new(CharOffset(7), CharOffset(13)), ""))
    );
}

#[test]
fn repeated_characters_do_not_collapse_to_a_zero_width_change() {
    // `a` -> `aa` has a one-character common prefix and would otherwise look
    // like the suffix covers the whole text.
    assert_eq!(
        common_change("a", "aa"),
        Some(span(CharRange::empty(CharOffset(1)), "a"))
    );
    assert_eq!(
        common_change("aa", "a"),
        Some(span(CharRange::new(CharOffset(1), CharOffset(2)), ""))
    );
}

#[test]
fn an_entirely_different_document_replaces_everything() {
    assert_eq!(
        common_change("abc", "xyz"),
        Some(span(CharRange::new(CharOffset(0), CharOffset(3)), "xyz"))
    );
}

#[test]
fn characters_are_counted_not_bytes() {
    // Both texts share "π" and the trailing emoji; only the middle differs.
    let change = common_change("πx😀", "πyy😀").unwrap();
    assert_eq!(change.old, CharRange::new(CharOffset(1), CharOffset(2)));
    assert_eq!(change.insert, "yy");
    assert_eq!(change.new_len(3), 4);
}

#[test]
fn combining_marks_are_kept_whole() {
    // Deleting the combining acute leaves the base character; the span never
    // starts inside the two-scalar cluster in a way that loses a scalar.
    let change = common_change("e\u{301}!", "e!").unwrap();
    assert_eq!(change.old, CharRange::new(CharOffset(1), CharOffset(2)));
    assert_eq!(change.insert, "");

    let change = common_change("ae\u{301}", "ae").unwrap();
    assert_eq!(change.old, CharRange::new(CharOffset(2), CharOffset(3)));
    assert_eq!(change.insert, "");
}

#[test]
fn crlf_text_keeps_the_line_ending_out_of_the_change() {
    let change = common_change("one\r\ntwo\r\n", "one\r\nTWO\r\n").unwrap();
    assert_eq!(change.old, CharRange::new(CharOffset(5), CharOffset(8)));
    assert_eq!(change.insert, "TWO");
}

#[test]
fn a_one_line_change_in_a_large_document_stays_small() {
    let mut lines = String::new();
    for index in 0..200_000 {
        lines.push_str(&format!("line {index} of a large document\n"));
    }
    let mut formatted = lines.clone();
    let marker = "line 150000 of a large document\n";
    let at = formatted.find(marker).unwrap();
    formatted.replace_range(at..at + marker.len(), "line 150000, reformatted\n");

    let change = common_change(&lines, &formatted).expect("the line changed");
    assert!(
        change.old.len() < 64,
        "a one-line edit must not replace the document, got {} characters",
        change.old.len()
    );
    assert!(
        change.insert.chars().count() < 64,
        "the replacement must be the edited line, got {} characters",
        change.insert.chars().count()
    );
    let expected_start = CharOffset(lines[..at].chars().count() + "line 150000".chars().count());
    assert_eq!(change.old.start, expected_start);
}
