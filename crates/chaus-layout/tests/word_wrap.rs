//! Unicode word-preferred wrapping and the shared row walker.

use chaus_layout::{
    column_at, row_count, row_count_with, rows, segments, segments_with, WrapPolicy,
};

fn word(text: &str, max: usize) -> Vec<(String, usize, usize)> {
    segments_with(text, Some(max), 4, WrapPolicy::Word)
        .into_iter()
        .map(|segment| (segment.display, segment.char_start, segment.char_end))
        .collect()
}

fn character(text: &str, max: usize) -> Vec<(String, usize, usize)> {
    segments_with(text, Some(max), 4, WrapPolicy::Character)
        .into_iter()
        .map(|segment| (segment.display, segment.char_start, segment.char_end))
        .collect()
}

#[test]
fn english_words_are_preferred_over_character_limits() {
    assert_eq!(
        word("hello world", 8),
        vec![("hello ".to_string(), 0, 6), ("world".to_string(), 6, 11),]
    );
    assert_eq!(
        character("hello world", 8),
        vec![("hello wo".to_string(), 0, 8), ("rld".to_string(), 8, 11)]
    );
}

#[test]
fn a_long_token_falls_back_to_grapheme_boundaries() {
    assert_eq!(
        word("a verylongword b", 8),
        vec![
            ("a ".to_string(), 0, 2),
            ("verylong".to_string(), 2, 10),
            ("word b".to_string(), 10, 16),
        ]
    );
    // UAX #14 allows a break after `//` and after a path slash, but not inside
    // `example`/`com` when the following break would overflow.
    assert_eq!(
        word("https://example.com/a/very/long/path", 10),
        vec![
            ("https://".to_string(), 0, 8),
            ("example.co".to_string(), 8, 18),
            ("m/a/very/".to_string(), 18, 27),
            ("long/path".to_string(), 27, 36),
        ]
    );
}

#[test]
fn cjk_and_nonbreaking_spaces_follow_unicode_breaks() {
    // Each ideograph is two columns and UAX #14 permits a break between them.
    assert_eq!(
        word("你好世界", 2),
        vec![
            ("你".to_string(), 0, 1),
            ("好".to_string(), 1, 2),
            ("世".to_string(), 2, 3),
            ("界".to_string(), 3, 4),
        ]
    );
    // A nonbreaking space is not a break opportunity, so the overflow falls
    // back to a grapheme boundary rather than splitting at the NBSP.
    assert_eq!(
        word("xx\u{00A0}yy", 4),
        vec![("xx\u{00A0}y".to_string(), 0, 4), ("y".to_string(), 4, 5)]
    );
    assert_eq!(
        word("a\u{00A0}b", 3),
        vec![("a\u{00A0}b".to_string(), 0, 3)]
    );
}

#[test]
fn graphemes_survive_combining_marks_and_zwj_emoji() {
    assert_eq!(
        word("e\u{0301}e\u{0301}", 1),
        vec![
            ("e\u{0301}".to_string(), 0, 2),
            ("e\u{0301}".to_string(), 2, 4),
        ]
    );
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let segments = segments_with(family, Some(1), 4, WrapPolicy::Word);
    assert_eq!(segments.len(), 1, "a ZWJ emoji is one indivisible grapheme");
    assert_eq!(segments[0].char_start, 0);
    assert_eq!(segments[0].char_end, family.chars().count());
}

#[test]
fn tabs_keep_their_stop_anchored_to_the_logical_line() {
    assert_eq!(
        word("a\tb", 4),
        vec![("a   ".to_string(), 0, 2), ("b".to_string(), 2, 3)]
    );
    assert_eq!(
        word("ab\tc", 4),
        vec![("ab  ".to_string(), 0, 3), ("c".to_string(), 3, 4)]
    );
}

#[test]
fn trailing_whitespace_does_not_create_a_blank_row() {
    let segments = segments_with("abc      ", Some(3), 4, WrapPolicy::Word);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].char_end, 9);
    assert!(segments
        .iter()
        .all(|segment| segment.display.contains("abc")));
}

#[test]
fn no_wrap_returns_the_whole_line_and_empty_lines_stay_one_row() {
    assert_eq!(
        segments_with("hello world", None, 4, WrapPolicy::Word),
        segments("hello world", None, 4)
    );
    assert_eq!(segments_with("", Some(4), 4, WrapPolicy::Word).len(), 1);
    assert_eq!(row_count_with("", Some(4), 4, WrapPolicy::Word), 1);
    assert_eq!(row_count("", Some(4), 4), 1);
}

#[test]
fn the_walker_and_segments_share_one_break_decision() {
    let corpus = [
        "",
        " ",
        "        ",
        "hello world",
        "a verylongword b",
        "你好世界",
        "a\u{00A0}b",
        "https://example.com/a/very/long/path",
        "one\ttwo\tthree",
        "e\u{0301}e\u{0301}e\u{0301}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}!",
        "  leading and trailing  ",
    ];
    for text in corpus {
        let char_count = text.chars().count();
        for max in 1..=12 {
            for policy in [WrapPolicy::Word, WrapPolicy::Character] {
                let rows: Vec<_> = rows(text, Some(max), 4, policy).collect();
                let segments = segments_with(text, Some(max), 4, policy);
                assert_eq!(rows.len(), segments.len(), "{text:?} at {max}");
                assert_eq!(
                    row_count_with(text, Some(max), 4, policy),
                    rows.len(),
                    "{text:?} at {max}"
                );
                assert_eq!(rows[0].char_start, 0);
                assert_eq!(rows.last().unwrap().char_end, char_count);
                for pair in rows.windows(2) {
                    assert_eq!(pair[0].char_end, pair[1].char_start);
                    assert_eq!(pair[0].end_column(), pair[1].start_column);
                }
                let total: usize = rows.iter().map(|row| row.columns).sum();
                assert_eq!(total, column_at(text, char_count, 4), "{text:?} at {max}");
            }
        }
    }
}

#[test]
fn large_lines_and_many_hard_lines_stay_consistent() {
    let line = "word ".repeat(20_000);
    let available = "some ordinary text ".repeat(5_000);
    assert_eq!(line.chars().count(), 100_000);
    let rows = row_count_with(&line, Some(80), 4, WrapPolicy::Word);
    let segments = segments_with(&line, Some(80), 4, WrapPolicy::Word);
    assert_eq!(rows, segments.len());
    assert!(rows > 1_000);
    let characters: Vec<char> = line.chars().collect();
    for segment in &segments[..segments.len() - 1] {
        assert!(characters[segment.char_end - 1].is_whitespace());
    }
    assert!(segments_with(&available, Some(80), 4, WrapPolicy::Word).len() > 1_000);
    assert_eq!(row_count_with("abc\n", None, 4, WrapPolicy::Word), 1);
}

#[test]
fn word_character_and_none_produce_distinct_rows() {
    let text = "alpha beta gamma";
    let word = segments_with(text, Some(9), 4, WrapPolicy::Word);
    let character = segments_with(text, Some(9), 4, WrapPolicy::Character);
    let none = segments_with(text, None, 4, WrapPolicy::Word);
    assert_ne!(word, character);
    assert_eq!(none.len(), 1);
    assert!(word.len() >= 2 && character.len() >= 2);
    let reassembled: String = word.iter().map(|segment| segment.display.clone()).collect();
    assert_eq!(reassembled.replace(' ', ""), text.replace(' ', ""));
}
