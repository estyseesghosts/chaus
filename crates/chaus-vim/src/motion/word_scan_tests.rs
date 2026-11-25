//! Oracle parity and multi-megabyte microbenchmarks for Vim word scans.
//!
//! The oracle helpers below are the index-based scans that existed before the
//! iterator rewrite, kept here so a behavior change is a test failure.

use std::hint::black_box;
use std::time::{Duration, Instant};

use chaus_core::position::CharOffset;
use ropey::Rope;

use super::{
    next_big_word_start, next_word_start, previous_big_word_start, previous_word_start, word_end,
    word_end_back, word_end_big,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OracleClass {
    Word,
    Punctuation,
}

fn oracle_run_class(character: char) -> Option<OracleClass> {
    if character.is_whitespace() {
        None
    } else if character.is_alphanumeric() || character == '_' {
        Some(OracleClass::Word)
    } else {
        Some(OracleClass::Punctuation)
    }
}

fn oracle_char_at(rope: &Rope, offset: CharOffset) -> Option<char> {
    if offset.0 < rope.len_chars() {
        Some(rope.char(offset.0))
    } else {
        None
    }
}

fn oracle_next_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    if let Some(class) = oracle_char_at(rope, CharOffset(index)).and_then(oracle_run_class) {
        while index < length
            && oracle_char_at(rope, CharOffset(index)).and_then(oracle_run_class) == Some(class)
        {
            index += 1;
        }
    }
    while index < length && oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    CharOffset(index)
}

fn oracle_next_big_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    if oracle_char_at(rope, CharOffset(index)).is_some_and(|character| !character.is_whitespace()) {
        while index < length
            && oracle_char_at(rope, CharOffset(index))
                .is_some_and(|character| !character.is_whitespace())
        {
            index += 1;
        }
    }
    while index < length && oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    CharOffset(index)
}

fn oracle_word_end(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    while index < length && oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    if index + 1 < length
        && !oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
        && oracle_char_at(rope, CharOffset(index + 1)).is_some_and(|next| !next.is_whitespace())
        && oracle_run_class(oracle_char_at(rope, CharOffset(index)).unwrap_or(' '))
            != oracle_run_class(oracle_char_at(rope, CharOffset(index + 1)).unwrap_or(' '))
    {
        index += 1;
    }
    while index + 1 < length
        && oracle_char_at(rope, CharOffset(index + 1)).is_some_and(|next| {
            oracle_char_at(rope, CharOffset(index)).is_some_and(|current| {
                oracle_run_class(current) == oracle_run_class(next) && !next.is_whitespace()
            })
        })
    {
        index += 1;
    }
    CharOffset(index.min(length.saturating_sub(1)))
}

fn oracle_word_end_big(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    while index < length && oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    while index + 1 < length
        && oracle_char_at(rope, CharOffset(index + 1)).is_some_and(|next| !next.is_whitespace())
        && !oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    CharOffset(index.min(length.saturating_sub(1)))
}

fn oracle_previous_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let mut index = from.0.min(rope.len_chars());
    while index > 0 && oracle_char_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    if index == 0 {
        return CharOffset(0);
    }
    let class = oracle_char_at(rope, CharOffset(index - 1)).and_then(oracle_run_class);
    while index > 0
        && oracle_char_at(rope, CharOffset(index - 1)).and_then(oracle_run_class) == class
        && !oracle_char_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    CharOffset(index)
}

fn oracle_previous_big_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let mut index = from.0.min(rope.len_chars());
    while index > 0 && oracle_char_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    while index > 0
        && oracle_char_at(rope, CharOffset(index - 1))
            .is_some_and(|character| !character.is_whitespace())
    {
        index -= 1;
    }
    CharOffset(index)
}

fn oracle_word_end_back(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    if length == 0 {
        return CharOffset::ZERO;
    }
    let mut index = from.0.min(length - 1);
    if index == 0 {
        return CharOffset::ZERO;
    }
    if oracle_char_at(rope, CharOffset(index)).is_some_and(|character| !character.is_whitespace()) {
        let class = oracle_char_at(rope, CharOffset(index)).and_then(oracle_run_class);
        while index > 0
            && oracle_char_at(rope, CharOffset(index - 1)).and_then(oracle_run_class) == class
            && !oracle_char_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
        {
            index -= 1;
        }
    }
    if index == 0 {
        return CharOffset::ZERO;
    }
    index -= 1;
    while index > 0 && oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index -= 1;
    }
    if oracle_char_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        return CharOffset(index);
    }
    let class = oracle_char_at(rope, CharOffset(index)).and_then(oracle_run_class);
    while index > 0
        && oracle_char_at(rope, CharOffset(index - 1)).and_then(oracle_run_class) == class
        && !oracle_char_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    let mut end = index;
    while end + 1 < length
        && oracle_char_at(rope, CharOffset(end + 1)).and_then(oracle_run_class) == class
        && !oracle_char_at(rope, CharOffset(end + 1)).is_some_and(char::is_whitespace)
    {
        end += 1;
    }
    CharOffset(end)
}

const CORPUS: &[&str] = &[
    "",
    "a",
    " ",
    "\n",
    "\r\n",
    "a\r\nb",
    "one two three",
    "foo.bar/baz qux",
    "...???!!!",
    "___foo___",
    "éclair Ωμέγα 漢字",
    "e\u{301} b",
    "😀🤖 x",
    " leading",
    "trailing ",
    "a\nb\nc",
    "\t\t",
    "x_y.z",
    "foo\n\nbar",
    "\r",
    "a\rb",
    "word.;word.;word.",
];

fn assert_every_origin_matches(source: &str) {
    let rope = Rope::from_str(source);
    let length = rope.len_chars();
    let mut origins: Vec<usize> = (0..=length).collect();
    origins.extend([length + 1, length + 100, usize::MAX]);
    for origin in origins {
        let from = CharOffset(origin);
        let context = format!("source {source:?}, origin {origin}");
        assert_eq!(
            next_word_start(&rope, from),
            oracle_next_word_start(&rope, from),
            "w: {context}"
        );
        assert_eq!(
            next_big_word_start(&rope, from),
            oracle_next_big_word_start(&rope, from),
            "W: {context}"
        );
        assert_eq!(
            word_end(&rope, from),
            oracle_word_end(&rope, from),
            "e: {context}"
        );
        assert_eq!(
            word_end_big(&rope, from),
            oracle_word_end_big(&rope, from),
            "E: {context}"
        );
        assert_eq!(
            previous_word_start(&rope, from),
            oracle_previous_word_start(&rope, from),
            "b: {context}"
        );
        assert_eq!(
            previous_big_word_start(&rope, from),
            oracle_previous_big_word_start(&rope, from),
            "B: {context}"
        );
        assert_eq!(
            word_end_back(&rope, from),
            oracle_word_end_back(&rope, from),
            "ge: {context}"
        );
    }
}

#[test]
fn all_word_scans_match_the_index_based_oracle() {
    for source in CORPUS {
        assert_every_origin_matches(source);
    }
}

#[test]
fn repeated_scans_do_not_drift() {
    let rope = Rope::from_str("one.two three four  ");
    let mut target = CharOffset::ZERO;
    for _ in 0..10_000 {
        target = next_word_start(&rope, target);
    }
    assert_eq!(target, CharOffset(20), "w clamps at the document end");

    let mut target = CharOffset(rope.len_chars());
    for _ in 0..10_000 {
        target = previous_word_start(&rope, target);
    }
    assert_eq!(target, CharOffset::ZERO, "b clamps at the document start");
}

fn large_rope(run: char, run_length: usize) -> Rope {
    let text = format!("{} tail", run.to_string().repeat(run_length));
    Rope::from_str(&text)
}

#[test]
fn multi_megabyte_identifier_run_scans_in_bounded_time() {
    const RUN: usize = 2_000_000;
    let rope = large_rope('a', RUN);
    let started = Instant::now();
    let start = black_box(next_word_start(&rope, CharOffset::ZERO));
    let end = black_box(word_end(&rope, CharOffset::ZERO));
    let back = black_box(previous_word_start(&rope, CharOffset(rope.len_chars())));
    let end_back = black_box(word_end_back(&rope, CharOffset(RUN - 1)));
    let elapsed = started.elapsed();
    assert_eq!(start, CharOffset(RUN + 1));
    assert_eq!(end, CharOffset(RUN - 1));
    assert_eq!(back, CharOffset(RUN + 1));
    assert_eq!(end_back, CharOffset::ZERO);
    assert!(
        elapsed < Duration::from_secs(20),
        "four scans of a {RUN}-char identifier run took {elapsed:?}"
    );
}

#[test]
fn multi_megabyte_punctuation_run_scans_in_bounded_time() {
    const RUN: usize = 2_000_000;
    let rope = large_rope('.', RUN);
    let started = Instant::now();
    let start = black_box(next_word_start(&rope, CharOffset::ZERO));
    let big_start = black_box(next_big_word_start(&rope, CharOffset::ZERO));
    let end = black_box(word_end(&rope, CharOffset::ZERO));
    let big_end = black_box(word_end_big(&rope, CharOffset::ZERO));
    let back = black_box(previous_big_word_start(&rope, CharOffset(rope.len_chars())));
    let elapsed = started.elapsed();
    assert_eq!(start, CharOffset(RUN + 1));
    assert_eq!(big_start, CharOffset(RUN + 1));
    assert_eq!(end, CharOffset(RUN - 1));
    assert_eq!(big_end, CharOffset(RUN - 1));
    assert_eq!(back, CharOffset(RUN + 1));
    assert!(
        elapsed < Duration::from_secs(20),
        "five scans of a {RUN}-char punctuation run took {elapsed:?}"
    );
}

#[test]
fn multi_megabyte_whitespace_run_scans_in_bounded_time() {
    const RUN: usize = 2_000_000;
    let rope = large_rope(' ', RUN);
    let started = Instant::now();
    let start = black_box(next_word_start(&rope, CharOffset::ZERO));
    let big_start = black_box(next_big_word_start(&rope, CharOffset::ZERO));
    let end = black_box(word_end(&rope, CharOffset::ZERO));
    let back = black_box(previous_word_start(&rope, CharOffset(rope.len_chars())));
    let end_back = black_box(word_end_back(&rope, CharOffset(rope.len_chars() - 1)));
    let elapsed = started.elapsed();
    // The fixture is `RUN` spaces plus `" tail"`, so the whitespace run is
    // RUN + 1 chars long and `tail` starts at RUN + 1.
    assert_eq!(start, CharOffset(RUN + 1));
    assert_eq!(big_start, CharOffset(RUN + 1));
    assert_eq!(end, CharOffset(RUN + 4));
    assert_eq!(back, CharOffset(RUN + 1));
    assert_eq!(end_back, CharOffset::ZERO);
    assert!(
        elapsed < Duration::from_secs(20),
        "five scans of a {RUN}-char whitespace run took {elapsed:?}"
    );
}
