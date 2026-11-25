//! Oracle parity and multi-megabyte microbenchmarks for conventional word
//! movement. The oracle functions are the index-based scans from before the
//! iterator rewrite.

use std::hint::black_box;
use std::time::{Duration, Instant};

use chaus_core::position::{CharOffset, CharRange};
use ropey::Rope;

use super::{scan_word_bounds, scan_word_left, scan_word_right};

fn oracle_is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn oracle_word_left(rope: &Rope, offset: CharOffset) -> CharOffset {
    let mut index = offset.0.min(rope.len_chars());
    while index > 0 && !oracle_is_word_char(rope.char(index - 1)) {
        index -= 1;
    }
    while index > 0 && oracle_is_word_char(rope.char(index - 1)) {
        index -= 1;
    }
    CharOffset(index)
}

fn oracle_word_right(rope: &Rope, offset: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = offset.0.min(length);
    while index < length && !oracle_is_word_char(rope.char(index)) {
        index += 1;
    }
    while index < length && oracle_is_word_char(rope.char(index)) {
        index += 1;
    }
    CharOffset(index)
}

fn oracle_word_bounds(rope: &Rope, offset: CharOffset) -> CharRange {
    let length = rope.len_chars();
    if length == 0 {
        return CharRange::empty(offset);
    }
    let mut start = offset.0.min(length - 1);
    let mut end = start;
    if oracle_is_word_char(rope.char(start)) {
        while start > 0 && oracle_is_word_char(rope.char(start - 1)) {
            start -= 1;
        }
        while end < length && oracle_is_word_char(rope.char(end)) {
            end += 1;
        }
    } else {
        while start > 0
            && !oracle_is_word_char(rope.char(start - 1))
            && rope.char(start - 1) != '\n'
        {
            start -= 1;
        }
        while end < length && !oracle_is_word_char(rope.char(end)) && rope.char(end) != '\n' {
            end += 1;
        }
        if end == start && end < length {
            end += 1;
        }
    }
    CharRange::new(CharOffset(start), CharOffset(end))
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

fn assert_every_offset_matches(source: &str) {
    let rope = Rope::from_str(source);
    let length = rope.len_chars();
    let mut origins: Vec<usize> = (0..=length).collect();
    origins.extend([length + 1, length + 100, usize::MAX]);
    for origin in origins {
        let from = CharOffset(origin);
        let context = format!("source {source:?}, offset {origin}");
        assert_eq!(
            scan_word_left(&rope, from),
            oracle_word_left(&rope, from),
            "Ctrl+Left: {context}"
        );
        assert_eq!(
            scan_word_right(&rope, from),
            oracle_word_right(&rope, from),
            "Ctrl+Right: {context}"
        );
        assert_eq!(
            scan_word_bounds(&rope, from),
            oracle_word_bounds(&rope, from),
            "double click: {context}"
        );
    }
}

#[test]
fn conventional_word_scans_match_the_index_based_oracle() {
    for source in CORPUS {
        assert_every_offset_matches(source);
    }
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
    let left = black_box(scan_word_left(&rope, CharOffset(rope.len_chars())));
    let right = black_box(scan_word_right(&rope, CharOffset::ZERO));
    let bounds = black_box(scan_word_bounds(&rope, CharOffset::ZERO));
    let elapsed = started.elapsed();
    assert_eq!(left, CharOffset(RUN + 1));
    assert_eq!(right, CharOffset(RUN));
    assert_eq!(bounds, CharRange::new(CharOffset::ZERO, CharOffset(RUN)));
    assert!(
        elapsed < Duration::from_secs(20),
        "three scans of a {RUN}-char identifier run took {elapsed:?}"
    );
}

#[test]
fn multi_megabyte_punctuation_run_scans_in_bounded_time() {
    const RUN: usize = 2_000_000;
    let rope = large_rope('.', RUN);
    let started = Instant::now();
    let left = black_box(scan_word_left(&rope, CharOffset(rope.len_chars())));
    let right = black_box(scan_word_right(&rope, CharOffset::ZERO));
    let bounds = black_box(scan_word_bounds(&rope, CharOffset::ZERO));
    let elapsed = started.elapsed();
    // Ctrl+Right skips the punctuation run and then the following word;
    // double-click stops at the first word character instead.
    assert_eq!(left, CharOffset(RUN + 1));
    assert_eq!(right, CharOffset(RUN + 5));
    assert_eq!(
        bounds,
        CharRange::new(CharOffset::ZERO, CharOffset(RUN + 1))
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "three scans of a {RUN}-char punctuation run took {elapsed:?}"
    );
}

#[test]
fn multi_megabyte_whitespace_run_scans_in_bounded_time() {
    const RUN: usize = 2_000_000;
    let rope = large_rope(' ', RUN);
    let started = Instant::now();
    let left = black_box(scan_word_left(&rope, CharOffset(rope.len_chars())));
    let right = black_box(scan_word_right(&rope, CharOffset::ZERO));
    let bounds = black_box(scan_word_bounds(&rope, CharOffset::ZERO));
    let elapsed = started.elapsed();
    // The fixture is `RUN` spaces plus `" tail"`, so the whitespace run is
    // RUN + 1 chars long.
    assert_eq!(left, CharOffset(RUN + 1));
    assert_eq!(right, CharOffset(RUN + 5));
    assert_eq!(
        bounds,
        CharRange::new(CharOffset::ZERO, CharOffset(RUN + 1))
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "three scans of a {RUN}-char whitespace run took {elapsed:?}"
    );
}
