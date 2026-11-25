//! Long trailing whitespace stays bounded and keeps every source offset.

use chaus_layout::{column_at, row_count_with, segments_with, WrapPolicy};

fn line_with(spaces: usize) -> String {
    format!("abc{}", " ".repeat(spaces))
}

#[test]
fn short_trailing_runs_stay_on_their_row() {
    for spaces in [1, 2] {
        let text = line_with(spaces);
        let segments = segments_with(&text, Some(3), 4, WrapPolicy::Word);
        assert_eq!(segments.len(), 1, "{spaces} trailing spaces fit one row");
        assert_eq!(segments[0].char_end, text.chars().count());
    }
}

#[test]
fn long_trailing_runs_wrap_instead_of_widening_one_row() {
    for spaces in [20, 10_000] {
        let text = line_with(spaces);
        let segments = segments_with(&text, Some(7), 4, WrapPolicy::Word);
        assert!(segments.len() > 1, "{spaces} spaces must wrap");
        assert!(
            segments[0].columns <= 7,
            "the first row must respect the wrap width, got {}",
            segments[0].columns
        );

        let mut end = 0usize;
        for (index, segment) in segments.iter().enumerate() {
            assert_eq!(segment.char_start, end, "rows stay contiguous");
            end = segment.char_end.into();
            assert!(
                !segment.display.is_empty(),
                "row {index} of {spaces} spaces must not be empty"
            );
            if index + 1 < segments.len() {
                assert!(
                    segment.columns <= 7,
                    "row {index} is {} columns wide",
                    segment.columns
                );
            }
        }
        assert_eq!(end, text.chars().count());
        assert!(
            segments.last().unwrap().columns <= 14,
            "only the final row may absorb one extra wrap width"
        );
        assert_eq!(
            row_count_with(&text, Some(7), 4, WrapPolicy::Word),
            segments.len(),
            "the counting walker agrees with the segment walker"
        );
        let total: usize = segments.iter().map(|segment| segment.columns).sum();
        assert_eq!(total, column_at(&text, text.chars().count(), 4));
    }
}

#[test]
fn mixed_trailing_whitespace_is_bounded_too() {
    let text = format!("abc{}", "\t \t  ".repeat(1_000));
    let segments = segments_with(&text, Some(5), 4, WrapPolicy::Word);
    assert!(segments.len() > 1);
    for segment in &segments[..segments.len() - 1] {
        assert!(
            segment.columns <= 5,
            "a mixed whitespace row is {} columns",
            segment.columns
        );
    }
    let mut end = 0usize;
    for segment in &segments {
        assert_eq!(segment.char_start, end);
        end = segment.char_end.into();
    }
    assert_eq!(end, text.chars().count());
}

#[test]
fn all_whitespace_lines_terminate_and_cover_the_text() {
    for width in [1, 2, 3, 80] {
        for spaces in [1, 2, 20, 1_000] {
            let text = " ".repeat(spaces);
            let segments = segments_with(&text, Some(width), 4, WrapPolicy::Word);
            assert!(!segments.is_empty());
            let mut end = 0usize;
            for segment in &segments {
                assert_eq!(segment.char_start, end);
                end = segment.char_end.into();
            }
            assert_eq!(end, spaces, "width {width}, {spaces} spaces");
            assert_eq!(
                row_count_with(&text, Some(width), 4, WrapPolicy::Word),
                segments.len()
            );
        }
    }
}
