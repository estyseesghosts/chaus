//! Capture precedence: inner spans override only their own bytes.
//!
//! The resolver works in UTF-8 byte space, like Tree-sitter, and must return
//! sorted, non-overlapping spans that retain the uncovered parts of an outer
//! string or comment capture.

use std::ops::Range;
use std::time::Instant;

use chaus_core::position::DocumentRevision;
use chaus_highlight::highlight::resolve_overlapping_captures;
use chaus_highlight::{parse_snapshot, LanguageId, TokenKind};
use ropey::Rope;

fn kinds(spans: &[(Range<usize>, TokenKind)]) -> Vec<(Range<usize>, TokenKind)> {
    spans.to_vec()
}

fn assert_normalized(spans: &[(Range<usize>, TokenKind)]) {
    for span in spans {
        assert!(span.0.start < span.0.end, "empty span: {span:?}");
    }
    for pair in spans.windows(2) {
        assert!(
            pair[0].0.end <= pair[1].0.start,
            "spans must be sorted and non-overlapping: {pair:?}"
        );
        assert!(
            !(pair[0].0.end == pair[1].0.start && pair[0].1 == pair[1].1),
            "adjacent equal kinds must be coalesced: {pair:?}"
        );
    }
}

#[test]
fn an_inner_escape_keeps_the_string_colour_on_both_sides() {
    let resolved =
        resolve_overlapping_captures(vec![(0..10, TokenKind::String), (3..5, TokenKind::Keyword)]);
    assert_eq!(
        kinds(&resolved),
        vec![
            (0..3, TokenKind::String),
            (3..5, TokenKind::Keyword),
            (5..10, TokenKind::String),
        ]
    );
    assert_normalized(&resolved);
}

#[test]
fn multiple_inner_captures_leave_the_outer_pieces_between_them() {
    let resolved = resolve_overlapping_captures(vec![
        (0..20, TokenKind::Comment),
        (2..4, TokenKind::Keyword),
        (10..11, TokenKind::Number),
        (17..20, TokenKind::Keyword),
    ]);
    assert_eq!(
        kinds(&resolved),
        vec![
            (0..2, TokenKind::Comment),
            (2..4, TokenKind::Keyword),
            (4..10, TokenKind::Comment),
            (10..11, TokenKind::Number),
            (11..17, TokenKind::Comment),
            (17..20, TokenKind::Keyword),
        ]
    );
    assert_normalized(&resolved);
}

#[test]
fn identical_ranges_use_the_first_capture_in_query_order() {
    let resolved =
        resolve_overlapping_captures(vec![(4..8, TokenKind::Keyword), (4..8, TokenKind::String)]);
    assert_eq!(kinds(&resolved), vec![(4..8, TokenKind::Keyword)]);
}

#[test]
fn equal_length_partial_overlaps_prefer_the_earlier_start() {
    let resolved =
        resolve_overlapping_captures(vec![(5..9, TokenKind::String), (3..7, TokenKind::Number)]);
    assert_eq!(
        kinds(&resolved),
        vec![(3..7, TokenKind::Number), (7..9, TokenKind::String)]
    );
}

#[test]
fn a_shorter_capture_wins_only_its_own_bytes() {
    let resolved = resolve_overlapping_captures(vec![
        (0..100, TokenKind::String),
        (40..60, TokenKind::Number),
        (45..55, TokenKind::Keyword),
    ]);
    assert_eq!(
        kinds(&resolved),
        vec![
            (0..40, TokenKind::String),
            (40..45, TokenKind::Number),
            (45..55, TokenKind::Keyword),
            (55..60, TokenKind::Number),
            (60..100, TokenKind::String),
        ]
    );
}

#[test]
fn unicode_byte_ranges_are_preserved_exactly() {
    // "é" is two UTF-8 bytes; Tree-sitter boundaries are bytes, not chars.
    let resolved =
        resolve_overlapping_captures(vec![(0..6, TokenKind::String), (2..4, TokenKind::Keyword)]);
    assert_eq!(
        kinds(&resolved),
        vec![
            (0..2, TokenKind::String),
            (2..4, TokenKind::Keyword),
            (4..6, TokenKind::String),
        ]
    );
}

#[test]
fn a_real_rust_string_keeps_colour_around_an_escape() {
    let source = "let s = \"a\\nb\";\n";
    let rope = Rope::from_str(source);
    let parsed = parse_snapshot(LanguageId::Rust, &rope, None, None, DocumentRevision::ZERO)
        .expect("Rust grammar");
    let spans = parsed.highlight.spans;
    let text = span_text(source, &spans);
    assert!(
        text.contains("\"a\\nb\""),
        "the whole string literal must stay coloured, got {text:?}"
    );
    for span in spans.iter() {
        assert!(span.range.start < span.range.end);
        assert!(span.range.end <= source.chars().count());
    }
}

fn span_text(source: &str, spans: &[chaus_highlight::Span]) -> String {
    let chars: Vec<char> = source.chars().collect();
    spans
        .iter()
        .filter(|span| span.kind == TokenKind::String)
        .map(|span| chars[span.range.clone()].iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn one_hundred_thousand_nested_captures_resolve_in_subquadratic_time() {
    // Alternating kinds keep neighbouring one-byte pieces from coalescing, so
    // the output size proves the resolver actually walks every precedence
    // level instead of collapsing the input early.
    let captures: Vec<(Range<usize>, TokenKind)> = (0..100_000)
        .map(|index| {
            let start = index;
            let end = 200_000 - index;
            let kind = if index % 2 == 0 {
                TokenKind::String
            } else {
                TokenKind::Comment
            };
            (start..end, kind)
        })
        .collect();
    let started = Instant::now();
    let resolved = resolve_overlapping_captures(captures);
    let elapsed = started.elapsed();

    assert_normalized(&resolved);
    let covered: usize = resolved
        .iter()
        .map(|(range, _)| range.end - range.start)
        .sum();
    assert_eq!(covered, 200_000, "the union of the nested captures is full");
    assert_eq!(
        resolved.len(),
        199_999,
        "the innermost capture contributes one piece; every outer level contributes two"
    );
    eprintln!(
        "resolved {} pieces from 100000 captures in {elapsed:?}",
        resolved.len()
    );
    assert!(
        elapsed.as_secs() < 20,
        "capture resolution took {elapsed:?}; the old quadratic loop cannot finish this input"
    );
}
