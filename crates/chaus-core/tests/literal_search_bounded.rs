//! Bounded-work search on repeated prefixes and randomized oracle checks.

use std::time::{Duration, Instant};

use chaus_core::document::Document;
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, CharRange};

fn document(source: &str) -> Document {
    let (normalized, eol) = EolMap::from_source(source);
    Document::from_normalized_text(normalized, eol, Eol::Lf)
}

fn matches_at(text: &[char], needle: &[char], start: usize, case_sensitive: bool) -> bool {
    if start + needle.len() > text.len() {
        return false;
    }
    text[start..start + needle.len()]
        .iter()
        .zip(needle)
        .all(|(candidate, expected)| {
            if case_sensitive {
                candidate == expected
            } else {
                candidate.to_lowercase().eq(expected.to_lowercase())
            }
        })
}

fn naive_ranges(text: &[char], needle: &[char], case_sensitive: bool) -> Vec<CharRange> {
    if needle.is_empty() || needle.len() > text.len() {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut candidate = 0usize;
    while candidate + needle.len() <= text.len() {
        if matches_at(text, needle, candidate, case_sensitive) {
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
fn a_long_near_miss_does_not_grow_with_repeated_prefixes() {
    let doc = document(&"a".repeat(1_000_000));
    let needle = format!("{}b", "a".repeat(64));
    for case_sensitive in [true, false] {
        let started = Instant::now();
        assert_eq!(
            doc.find_literal(&needle, CharOffset(0), true, case_sensitive),
            None
        );
        assert!(doc.literal_ranges(&needle, case_sensitive).is_empty());
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "near-miss search grew too fast: {elapsed:?} (case-sensitive {case_sensitive})"
        );
    }
}

#[test]
fn randomized_search_matches_the_naive_oracle() {
    let alphabet = ['a', 'A', 'b', 'é', 'É', '😀', ' ', '\n'];
    let needles = ["a", "aa", "ab", "É", "aé", "aba", "😀a", "missing"];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        state
    };

    for _ in 0..40 {
        let length = (next() % 48) as usize;
        let source: String = (0..length)
            .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
            .collect();
        let doc = document(&source);
        let text: Vec<char> = source.chars().collect();
        for needle in needles {
            let expected = needle.chars().collect::<Vec<char>>();
            for case_sensitive in [true, false] {
                assert_eq!(
                    doc.literal_ranges(needle, case_sensitive),
                    naive_ranges(&text, &expected, case_sensitive),
                    "ranges differ for {needle:?} in {source:?}"
                );
                for from in 0..=text.len() {
                    for forward in [true, false] {
                        let expected_at = if expected.is_empty() || expected.len() > text.len() {
                            None
                        } else {
                            let matches =
                                |start: usize| matches_at(&text, &expected, start, case_sensitive);
                            let from = from.min(text.len());
                            let last_start = text.len() - expected.len();
                            if forward {
                                (from..=last_start)
                                    .find(|start| matches(*start))
                                    .or_else(|| {
                                        (0..from.min(last_start + 1)).find(|start| matches(*start))
                                    })
                            } else {
                                (from >= expected.len())
                                    .then(|| {
                                        (0..=from - expected.len())
                                            .rev()
                                            .find(|start| matches(*start))
                                    })
                                    .flatten()
                                    .or_else(|| {
                                        (0..=last_start).rev().find(|start| matches(*start))
                                    })
                            }
                        };
                        assert_eq!(
                            doc.find_literal(needle, CharOffset(from), forward, case_sensitive),
                            expected_at.map(CharOffset),
                            "find differs for {needle:?} from {from} forward={forward} case={case_sensitive} in {source:?}"
                        );
                    }
                }
            }
        }
    }
}
