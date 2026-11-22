//! Position newtype behavior.

use chaus_core::position::{CharOffset, CharRange, Selection};

#[test]
fn range_normalizes_direction_and_reports_length() {
    let range = CharRange::new(CharOffset(9), CharOffset(2));
    assert_eq!(range.start, CharOffset(2));
    assert_eq!(range.end, CharOffset(9));
    assert_eq!(range.len(), 7);
    assert!(!range.is_empty());
    assert!(range.contains(CharOffset(2)));
    assert!(!range.contains(CharOffset(9)));
}

#[test]
fn selection_preserves_direction() {
    let selection = Selection::new(CharOffset(4), CharOffset(1));
    assert!(selection.reversed());
    assert_eq!(
        selection.range(),
        CharRange::new(CharOffset(1), CharOffset(4))
    );
}

#[test]
fn checked_arithmetic_does_not_wrap() {
    assert_eq!(CharOffset(1).checked_add(usize::MAX), None);
    assert_eq!(CharOffset(0).checked_sub(1), None);
}
