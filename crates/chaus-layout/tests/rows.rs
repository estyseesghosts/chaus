//! `row_count` must agree with the segment layout it summarizes.

use chaus_layout::{row_count, segments};

#[test]
fn row_count_matches_segments_for_tricky_lines() {
    let lines = [
        "",
        "ascii",
        "a\tb\tc",
        "\t\tindented",
        "日本語のテキスト",
        "e\u{301}\u{302} combining",
        "👨‍👩‍👧‍👦 family",
        "mixed\t日本 e\u{301} 👨‍👩‍👧‍👦",
        "trailing spaces   ",
        "   ",
    ];
    let widths = [None, Some(1), Some(2), Some(3), Some(4), Some(7), Some(80)];
    for line in lines {
        for max in widths {
            for tab_stop in [1, 2, 4, 8] {
                assert_eq!(
                    row_count(line, max, tab_stop),
                    segments(line, max, tab_stop).len(),
                    "line {line:?}, max {max:?}, tab_stop {tab_stop}"
                );
            }
        }
    }
}
