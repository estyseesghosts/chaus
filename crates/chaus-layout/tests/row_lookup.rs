//! Row lookup shares the painting walker and stops at the target row.

use chaus_layout::{row_of_char_with_stats, segments_with, WrapPolicy};

fn oracle(
    text: &str,
    columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
    at: usize,
) -> usize {
    let segments = segments_with(text, columns, tab_stop, policy);
    segments
        .iter()
        .position(|segment| at < segment.char_end)
        .unwrap_or(segments.len().saturating_sub(1))
}

#[test]
fn row_lookup_matches_the_full_segment_list() {
    let samples = [
        "",
        "a",
        "one two three four five six seven",
        "a\tbb\tccc\tdddd",
        "naïve ÉCOLE 👩‍💻 漢字",
        &"x".repeat(400),
    ];
    for text in &samples {
        let char_count = text.chars().count();
        for policy in [WrapPolicy::Word, WrapPolicy::Character] {
            for columns in [None, Some(1), Some(4), Some(12)] {
                for at in 0..=char_count {
                    let expected = oracle(text, columns, 4, policy, at);
                    let found = row_of_char_with_stats(text, columns, 4, policy, at);
                    assert_eq!(
                        found.row, expected,
                        "row differs for {text:?} at {at} columns={columns:?} policy={policy:?}"
                    );
                    assert!(
                        found.rows_visited <= found.row + 1,
                        "the lookup must stop once the row is found: {found:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_huge_unwrapped_line_visits_one_row() {
    let text = "a".repeat(1024 * 1024);
    let found = row_of_char_with_stats(&text, None, 4, WrapPolicy::Word, 500);
    assert_eq!(found.row, 0);
    assert_eq!(found.rows_visited, 1);
}
