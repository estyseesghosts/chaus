//! `LineSource::Text::window()` uses Unicode scalar offsets, not byte offsets.

use chaus_layout::{LineCharOffset, LineSource};

fn window(text: &str, start: usize, end: usize) -> String {
    LineSource::from_str(text)
        .window(LineCharOffset::new(start), LineCharOffset::new(end))
        .into_owned()
}

#[test]
fn non_zero_scalar_range_returns_exact_scalars() {
    let text = "abcdef";
    assert_eq!(window(text, 1, 3), "bc", "scalars 1 and 2 of an ASCII line");
    assert_eq!(window(text, 0, 3), "abc");
    assert_eq!(window(text, 2, 5), "cde");
    assert_eq!(window(text, 3, 6), "def");
}

#[test]
fn multibyte_scalars_are_addressed_by_scalar_index() {
    let text = "aé漢👩\u{200d}💻z";
    assert_eq!(text.chars().count(), 7);
    assert_eq!(window(text, 1, 3), "é漢");
    assert_eq!(window(text, 2, 4), "漢👩");
    assert_eq!(window(text, 3, 6), "👩\u{200d}💻");
    assert_eq!(window(text, 4, 6), "\u{200d}💻");
    assert_eq!(window(text, 6, 7), "z");
    assert_eq!(window(text, 0, 7), text);
}

#[test]
fn degenerate_ranges_return_empty_or_full_text() {
    let text = "aé漢👩\u{200d}💻z";
    assert_eq!(window(text, 0, 0), "");
    assert_eq!(window(text, 3, 3), "");
    assert_eq!(window(text, 7, 7), "", "start == len");
    assert_eq!(window(text, 6, 7), "z", "end == len");
    assert_eq!(window(text, 0, 7), text);
    assert_eq!(window(text, 7, 3), "", "end below start clamps to start");
}
