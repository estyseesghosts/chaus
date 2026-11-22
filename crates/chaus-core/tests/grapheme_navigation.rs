//! Grapheme cursor movement through bounded windows.
//!
//! The reference is a plain `unicode-segmentation` walk over the normalized
//! Rope text. Every offset in the document, including invalid/interior ones,
//! must answer exactly like that reference.

use std::time::Instant;

use chaus_core::document::Document;
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::CharOffset;
use unicode_segmentation::UnicodeSegmentation;

fn document(source: &str) -> Document {
    let (normalized, eol) = EolMap::from_source(source);
    Document::from_normalized_text(normalized, eol, Eol::Lf)
}

fn boundaries(text: &str) -> Vec<usize> {
    let mut result = vec![0usize];
    let mut offset = 0usize;
    for grapheme in text.graphemes(true) {
        offset += grapheme.chars().count();
        result.push(offset);
    }
    result
}

fn reference_prev(boundaries: &[usize], offset: usize) -> usize {
    boundaries
        .iter()
        .rev()
        .copied()
        .find(|boundary| *boundary < offset)
        .unwrap_or(0)
}

fn reference_next(boundaries: &[usize], offset: usize, len: usize) -> usize {
    boundaries
        .iter()
        .copied()
        .find(|boundary| *boundary > offset)
        .unwrap_or(len)
}

fn assert_matches_reference(source: &str) {
    let doc = document(source);
    let text = doc.rope().to_string();
    let expected = boundaries(&text);
    let len = text.chars().count();
    for offset in 0..=len {
        let prev = doc.prev_grapheme_boundary(CharOffset(offset)).0;
        let next = doc.next_grapheme_boundary(CharOffset(offset)).0;
        assert_eq!(
            prev,
            reference_prev(&expected, offset),
            "prev at {offset} in {text:?}"
        );
        assert_eq!(
            next,
            reference_next(&expected, offset, len),
            "next at {offset} in {text:?}"
        );
    }
}

#[test]
fn navigation_matches_the_reference_for_unicode_and_hard_breaks() {
    let cases = [
        "",
        "a",
        "abc\ndef",
        "é",
        "e\u{301}x\u{301}",
        "👩‍💻",
        "👨‍👩‍👧‍👦 flag 🇺🇸",
        "漢字かな",
        "a\tb\r\nc\n\nlast",
        "🇺🇸🇨🇦",
        "\u{301}\u{301}base",
    ];
    for case in cases {
        assert_matches_reference(case);
    }
}

#[test]
fn interior_offsets_clamp_to_legal_boundaries() {
    let doc = document("a👩‍💻e\u{301}z");
    let text = doc.rope().to_string();
    let expected = boundaries(&text);
    let len = text.chars().count();
    for offset in 0..=len {
        let prev = doc.prev_grapheme_boundary(CharOffset(offset)).0;
        let next = doc.next_grapheme_boundary(CharOffset(offset)).0;
        assert_eq!(prev, reference_prev(&expected, offset));
        assert_eq!(next, reference_next(&expected, offset, len));
    }
    assert_eq!(
        doc.prev_grapheme_boundary(CharOffset(len + 100)).0,
        doc.prev_grapheme_boundary(CharOffset(len)).0,
        "out-of-range offsets clamp to the document end"
    );
    assert_eq!(doc.next_grapheme_boundary(CharOffset(len + 100)).0, len);
    assert_eq!(doc.prev_grapheme_boundary(CharOffset(0)).0, 0);
}

#[test]
fn a_cluster_longer_than_a_window_still_resolves() {
    let cluster = format!("a{}b", "\u{301}".repeat(5_000));
    let doc = document(&cluster);
    let text = doc.rope().to_string();
    let expected = boundaries(&text);
    let len = text.chars().count();
    assert_eq!(expected.len(), 3, "one base cluster plus b");
    assert_eq!(
        doc.prev_grapheme_boundary(CharOffset(len)).0,
        expected[1],
        "previous boundary before b is the end of the long cluster"
    );
    assert_eq!(doc.next_grapheme_boundary(CharOffset(0)).0, expected[1]);
    assert_eq!(doc.next_grapheme_boundary(CharOffset(expected[1])).0, len);
}

#[test]
fn repeated_movement_on_a_megabyte_line_stays_fast() {
    let line = "a".repeat(1_000_000);
    let doc = document(&line);
    let len = doc.len_chars();
    let started = Instant::now();
    let mut offset = len - 1;
    for _ in 0..1_000 {
        offset = doc.prev_grapheme_boundary(CharOffset(offset)).0;
    }
    assert_eq!(offset, len - 1_001);
    for _ in 0..1_000 {
        offset = doc.next_grapheme_boundary(CharOffset(offset)).0;
    }
    assert_eq!(offset, len - 1);
    let elapsed = started.elapsed();
    eprintln!("2000 grapheme moves on a 1 MiB line took {elapsed:?}");
    assert!(
        elapsed.as_secs() < 10,
        "grapheme movement must not scan the whole line per step: {elapsed:?}"
    );
}
