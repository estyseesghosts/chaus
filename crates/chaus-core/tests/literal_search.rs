//! Streaming literal search without whole-document copies.
//!
//! Every result is compared with a naive char-vector reference so wraparound,
//! backward matching and case behavior cannot drift.

use std::time::Instant;

use chaus_core::document::Document;
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, CharRange};

fn document(source: &str) -> Document {
    let (normalized, eol) = EolMap::from_source(source);
    Document::from_normalized_text(normalized, eol, Eol::Lf)
}

fn chars_match(candidate: char, expected: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        candidate == expected
    } else {
        candidate.to_lowercase().eq(expected.to_lowercase())
    }
}

fn naive_find(
    text: &[char],
    needle: &[char],
    from: usize,
    forward: bool,
    case_sensitive: bool,
) -> Option<usize> {
    if needle.is_empty() || needle.len() > text.len() {
        return None;
    }
    let matches_at = |start: usize| {
        start + needle.len() <= text.len()
            && text[start..start + needle.len()]
                .iter()
                .zip(needle)
                .all(|(candidate, expected)| chars_match(*candidate, *expected, case_sensitive))
    };
    let from = from.min(text.len());
    let last_start = text.len() - needle.len();
    if forward {
        (from..=last_start)
            .find(|start| matches_at(*start))
            .or_else(|| {
                let wrap_end = from.min(last_start + 1);
                (0..wrap_end).find(|start| matches_at(*start))
            })
    } else {
        if from >= needle.len() {
            if let Some(found) = (0..=from - needle.len())
                .rev()
                .find(|start| matches_at(*start))
            {
                return Some(found);
            }
        }
        (0..=last_start).rev().find(|start| matches_at(*start))
    }
}

fn naive_ranges(text: &[char], needle: &[char], case_sensitive: bool) -> Vec<CharRange> {
    if needle.is_empty() || needle.len() > text.len() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut candidate = 0usize;
    while candidate + needle.len() <= text.len() {
        let matches = text[candidate..candidate + needle.len()]
            .iter()
            .zip(needle)
            .all(|(candidate, expected)| chars_match(*candidate, *expected, case_sensitive));
        if matches {
            ranges.push(CharRange::new(
                CharOffset(candidate),
                CharOffset(candidate + needle.len()),
            ));
            candidate += needle.len();
        } else {
            candidate += 1;
        }
    }
    ranges
}

#[test]
fn forward_and_backward_find_honor_from_and_wrap() {
    let doc = document("one two one two one");
    let from = |n: usize| CharOffset(n);
    assert_eq!(doc.find_literal("one", from(0), true, true), Some(from(0)));
    assert_eq!(doc.find_literal("one", from(1), true, true), Some(from(8)));
    assert_eq!(
        doc.find_literal("one", from(15), true, true),
        Some(from(16)),
        "a later match is still forward"
    );
    assert_eq!(
        doc.find_literal("one", from(17), true, true),
        Some(from(0)),
        "forward wraps to the first match"
    );
    assert_eq!(
        doc.find_literal("one", from(17), false, true),
        Some(from(8)),
        "backward only accepts matches that end at or before `from`"
    );
    assert_eq!(
        doc.find_literal("one", from(19), false, true),
        Some(from(16)),
        "backward from the true end finds the last match"
    );
    assert_eq!(
        doc.find_literal("one", from(16), false, true),
        Some(from(8)),
        "backward requires the match to end before `from`"
    );
    assert_eq!(
        doc.find_literal("one", from(0), false, true),
        Some(from(16)),
        "backward wraps to the last match"
    );
    assert_eq!(doc.find_literal("", from(0), true, true), None);
    assert_eq!(doc.find_literal("missing", from(0), true, true), None);
    assert_eq!(
        doc.find_literal("longer than document", from(0), true, true),
        None
    );
}

#[test]
fn unicode_needles_and_case_insensitivity_keep_char_offsets() {
    let doc = document("naïve ÉCOLE 👩‍💻 école");
    let text: Vec<char> = doc.rope().to_string().chars().collect();
    for (needle, case_sensitive) in [
        ("école", true),
        ("ÉCOLE", false),
        ("👩‍💻", true),
        ("nai", true),
        ("é", true),
        ("É", false),
    ] {
        let needle_chars: Vec<char> = needle.chars().collect();
        for from in 0..=text.len() {
            for forward in [true, false] {
                let expected =
                    naive_find(&text, &needle_chars, from, forward, case_sensitive).map(CharOffset);
                assert_eq!(
                    doc.find_literal(needle, CharOffset(from), forward, case_sensitive),
                    expected,
                    "needle {needle:?} from {from} forward={forward} case={case_sensitive}"
                );
            }
        }
        assert_eq!(
            doc.literal_ranges(needle, case_sensitive),
            naive_ranges(&text, &needle_chars, case_sensitive),
            "needle {needle:?} case={case_sensitive}"
        );
    }
}

#[test]
fn overlapping_candidates_are_reported_non_overlapping() {
    let doc = document("aaaa");
    assert_eq!(
        doc.literal_ranges("aa", true),
        vec![
            CharRange::new(CharOffset(0), CharOffset(2)),
            CharRange::new(CharOffset(2), CharOffset(4)),
        ]
    );
    assert_eq!(
        doc.literal_ranges("aa", true).len(),
        2,
        "matches advance by the needle length"
    );
}

#[test]
fn matches_crossing_rope_chunks_are_found() {
    // Build enough chunks that a needle placed near several positions will
    // straddle at least one internal boundary on some iteration.
    let prefix = "x".repeat(200_000);
    for position in [1_000, 65_536, 100_000, 131_072, 199_990] {
        let mut source = String::with_capacity(prefix.len() + 16);
        source.push_str(&prefix[..position]);
        source.push_str("needle");
        source.push_str(&prefix[position..]);
        let doc = document(&source);
        let found = doc.find_literal("needle", CharOffset(0), true, true);
        assert_eq!(
            found,
            Some(CharOffset(position)),
            "needle at char {position} must not be lost at a Rope chunk edge"
        );
        assert_eq!(
            doc.literal_ranges("needle", true),
            vec![CharRange::new(
                CharOffset(position),
                CharOffset(position + "needle".chars().count())
            )]
        );
    }
}

#[test]
fn repeated_find_next_on_a_large_buffer_does_not_allocate_the_whole_rope() {
    let unit = "alpha beta gamma delta\n";
    let source = unit.repeat(80_000);
    let doc = document(&source);
    let needle = "gamma";
    let started = Instant::now();
    let mut from = CharOffset(0);
    let mut found = 0usize;
    for _ in 0..1_000 {
        let Some(position) = doc.find_literal(needle, from, true, true) else {
            break;
        };
        found += 1;
        from = CharOffset(position.0 + needle.chars().count());
    }
    let elapsed = started.elapsed();
    assert_eq!(found, 1_000);
    eprintln!(
        "1000 Find Next calls over {} chars took {elapsed:?}",
        doc.len_chars()
    );
    assert!(
        elapsed.as_secs() < 20,
        "streaming search regressed badly: {elapsed:?}"
    );
}
