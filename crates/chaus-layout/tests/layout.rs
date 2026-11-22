//! Soft-wrap column geometry.

use chaus_layout::{char_at_column, column_at, row_count, segments, Segment};
use unicode_width::UnicodeWidthStr;

#[test]
fn ascii_wraps_at_exact_column() {
    let rows = segments("abcdefgh", Some(3), 4);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].display, "abc");
    assert_eq!(rows[1].display, "def");
    assert_eq!(rows[2].display, "gh");
    assert_eq!(rows[1].start_column, 3);
    assert_eq!(rows[2].char_start, 6);
}

#[test]
fn empty_line_still_is_single_row() {
    let rows = segments("", Some(80), 4);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_empty());
    assert_eq!(rows[0].columns, 0);
    let _: Segment = rows[0].clone();
}

#[test]
fn tabs_advance_to_the_next_tab_stop() {
    let rows = segments("\tx", Some(80), 4);
    assert_eq!(rows[0].display, "    x");
    assert_eq!(column_at("\tx", 1, 4), 4);
    assert_eq!(column_at("\tx", 2, 4), 5);
}

#[test]
fn tab_stops_follow_the_logical_line_across_wraps() {
    // The tab starts at logical column 3, so it expands to one column and ends
    // exactly on the wrap boundary; it stays on the first row.
    let rows = segments("abc\tx", Some(4), 4);
    assert_eq!(rows[0].display, "abc ");
    assert_eq!(rows[0].columns, 4);
    assert_eq!(rows[1].display, "x");
    assert_eq!(rows[1].start_column, 4);

    // A tab that does not fit moves whole to the next row. Its width still
    // comes from its logical column, so wrapping never changes tab stops.
    let rows = segments("abc\td", Some(3), 4);
    assert_eq!(rows[0].display, "abc");
    assert_eq!(
        rows[1].display, " d",
        "the wrapped tab keeps its logical width of one column"
    );
    assert_eq!(rows[1].start_column, 3);
    assert_eq!(column_at("abc\td", 3, 4), 3, "caret before the tab");
    assert_eq!(column_at("abc\td", 4, 4), 4, "caret after the tab");
}

#[test]
fn wide_graphemes_take_two_columns_and_stay_whole() {
    let rows = segments("日本語", Some(4), 4);
    assert_eq!(rows[0].display, "日本");
    assert_eq!(rows[0].columns, 4);
    assert_eq!(rows[1].display, "語");
    assert_eq!(rows[1].start_column, 4);
    let family = "👨‍👩‍👧‍👦";
    assert_eq!(column_at(family, 1, 4), family.width());
    assert_eq!(
        segments(family, Some(1), 4).len(),
        1,
        "never split a cluster"
    );
}

#[test]
fn combining_marks_share_the_base_column() {
    assert_eq!(column_at("e\u{301}x", 2, 4), 1);
    assert_eq!(char_at_column("e\u{301}x", 1, 4), 2);
}

#[test]
fn row_count_matches_segments_after_tabs() {
    let text = "a\tb\tc";
    assert_eq!(
        row_count(text, Some(4), 4),
        segments(text, Some(4), 4).len()
    );
    assert_eq!(row_count(text, None, 4), 1);
}
