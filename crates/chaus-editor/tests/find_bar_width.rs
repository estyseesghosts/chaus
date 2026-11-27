//! Floating find-bar width is editor UI geometry, not layout policy.

use chaus_editor::find_bar_width;

#[test]
fn the_bar_fits_inside_the_minimum_window() {
    assert_eq!(find_bar_width(420.0), 400.0);
    assert_eq!(find_bar_width(360.0), 340.0);
}

#[test]
fn the_bar_stops_growing_at_its_readable_maximum() {
    assert_eq!(find_bar_width(800.0), 470.0);
    assert_eq!(find_bar_width(10_000.0), 470.0);
}

#[test]
fn degenerate_widths_never_produce_a_negative_size() {
    assert_eq!(find_bar_width(12.0), 0.0);
    assert_eq!(find_bar_width(0.0), 0.0);
}
