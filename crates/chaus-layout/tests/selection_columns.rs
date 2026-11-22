//! Segment-local selection columns for wrapped rows.

use chaus_layout::{local_selection_columns, segments};

#[test]
fn selection_columns_are_relative_to_the_wrapped_segment() {
    let segments = segments("abcdefghij", Some(5), 4);
    assert_eq!(segments.len(), 2);

    assert_eq!(
        local_selection_columns(&segments[0], 3, 8),
        Some((3, 5)),
        "the first row stops at its own end"
    );
    assert_eq!(
        local_selection_columns(&segments[1], 3, 8),
        Some((0, 3)),
        "the second row starts at column zero, not column five"
    );
}

#[test]
fn a_selection_that_misses_a_row_returns_no_rectangle() {
    let segments = segments("abcdefghij", Some(5), 4);
    assert_eq!(local_selection_columns(&segments[0], 6, 8), None);
    assert_eq!(local_selection_columns(&segments[1], 6, 7), Some((1, 2)));
    assert_eq!(local_selection_columns(&segments[1], 1, 2), None);
}

#[test]
fn expanded_tabs_keep_their_visual_width_inside_the_segment() {
    let segments = segments("\tab", Some(10), 4);
    // The tab expands to four columns, so absolute columns 1..3 are inside it.
    assert_eq!(local_selection_columns(&segments[0], 1, 3), Some((1, 3)));
    assert_eq!(local_selection_columns(&segments[0], 4, 6), Some((4, 6)));
}
