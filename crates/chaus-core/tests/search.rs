use chaus_core::document::Document;
use chaus_core::position::{CharOffset, CharRange};

fn offset(index: usize) -> CharOffset {
    CharOffset(index)
}

#[test]
fn literal_search_advances_and_wraps_in_both_directions() {
    let document = Document::from_source_text("one two one");
    let first = document.find_literal("one", offset(0), true, true);
    assert_eq!(first, Some(offset(0)));
    let next = document.find_literal("one", offset(3), true, true);
    assert_eq!(next, Some(offset(8)));
    let wrapped = document.find_literal("one", offset(11), true, true);
    assert_eq!(wrapped, Some(offset(0)));

    let previous = document.find_literal("one", offset(8), false, true);
    assert_eq!(previous, Some(offset(0)));
    let wrapped_back = document.find_literal("one", offset(0), false, true);
    assert_eq!(wrapped_back, Some(offset(8)));
}

#[test]
fn literal_search_can_ignore_ascii_and_unicode_case() {
    let document = Document::from_source_text("Café CAFÉ cafe");
    assert_eq!(
        document.find_literal("café", offset(0), true, false),
        Some(offset(0))
    );
    assert_eq!(
        document.find_literal("café", offset(4), true, false),
        Some(offset(5))
    );
    assert_eq!(
        document.find_literal("CAFÉ", offset(0), true, true),
        Some(offset(5))
    );
    assert_eq!(
        document.find_literal("CAFÉ", offset(0), true, false),
        Some(offset(0))
    );
}

#[test]
fn literal_ranges_include_every_non_overlapping_match() {
    let document = Document::from_source_text("aAaA aa");
    let ranges = document.literal_ranges("aa", false);
    assert_eq!(
        ranges,
        vec![
            CharRange::new(offset(0), offset(2)),
            CharRange::new(offset(2), offset(4)),
            CharRange::new(offset(5), offset(7)),
        ]
    );
}
