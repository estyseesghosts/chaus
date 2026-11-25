//! Source-char to displayed-byte mapping for tabs and grapheme clusters.

use chaus_layout::{display_boundaries, segments_with, WrapPolicy};

fn display_for(line: &str, tab_stop: usize) -> String {
    segments_with(line, None, tab_stop, WrapPolicy::Character)
        .into_iter()
        .map(|segment| segment.display)
        .collect()
}

fn assert_mapping(line: &str, tab_stop: usize) {
    let segments = segments_with(line, None, tab_stop, WrapPolicy::Character);
    assert_eq!(segments.len(), 1, "no-wrap line must be one segment");
    let segment = &segments[0];
    let boundaries = display_boundaries(line, segment, tab_stop);
    assert_eq!(
        boundaries
            .first()
            .map(|boundary| (boundary.char_offset.get(), boundary.display_byte.get())),
        Some((0, 0))
    );
    assert_eq!(
        boundaries
            .last()
            .map(|boundary| (boundary.char_offset.get(), boundary.display_byte.get())),
        Some((line.chars().count(), segment.display.len())),
        "the mapping must end at the source and display ends"
    );
    for pair in boundaries.windows(2) {
        assert!(
            pair[0].display_byte <= pair[1].display_byte,
            "display bytes must advance monotonically: {pair:?}"
        );
        assert!(
            segment.display.is_char_boundary(pair[0].display_byte.get()),
            "boundary {} is not a UTF-8 boundary in {:?}",
            pair[0].display_byte.get(),
            segment.display
        );
        assert!(
            pair[0].char_offset <= pair[1].char_offset,
            "source chars must advance monotonically: {pair:?}"
        );
    }
}

#[test]
fn tab_expansion_is_measured_from_the_logical_line_start() {
    let line = "👩‍💻\tlet x = 1;";
    assert_eq!(display_for(line, 4), "👩‍💻  let x = 1;");
    assert_mapping(line, 4);

    let boundaries = display_boundaries(
        line,
        &segments_with(line, None, 4, WrapPolicy::Character)[0],
        4,
    );
    assert_eq!(
        (
            boundaries[1].char_offset.get(),
            boundaries[1].display_byte.get()
        ),
        (3, "👩‍💻".len()),
        "the ZWJ emoji is one source grapheme and one display byte run"
    );
    assert_eq!(
        (
            boundaries[2].char_offset.get(),
            boundaries[2].display_byte.get()
        ),
        (4, "👩‍💻  ".len()),
        "the tab expands to two spaces after a width-2 emoji"
    );
}

#[test]
fn combining_cjk_and_variation_sequences_keep_grapheme_boundaries() {
    for (line, tab_stop) in [
        ("e\u{301}\tlet x", 4usize),
        ("漢字\tlet z", 4),
        ("❤\u{fe0f}\tvalue", 4),
        ("\t\tx", 4),
        ("a\tb\tc", 8),
    ] {
        assert_mapping(line, tab_stop);
    }

    let line = "e\u{301}\tlet x";
    let boundaries = display_boundaries(
        line,
        &segments_with(line, None, 4, WrapPolicy::Character)[0],
        4,
    );
    assert_eq!(
        (
            boundaries[1].char_offset.get(),
            boundaries[1].display_byte.get()
        ),
        (2, "e\u{301}".len()),
        "the combining mark stays attached to its base"
    );
    assert_eq!(
        (
            boundaries[2].char_offset.get(),
            boundaries[2].display_byte.get()
        ),
        (3, "e\u{301}   ".len()),
        "a width-1 grapheme leaves three spaces before the tab stop"
    );
}

#[test]
fn wrapped_segments_map_only_their_own_source_range() {
    let line = "alpha 👩‍💻\tbeta gamma delta epsilon zeta eta theta iota kappa";
    let segments = segments_with(line, Some(10), 4, WrapPolicy::Word);
    assert!(segments.len() > 1);
    let mut previous_end = 0usize;
    for segment in &segments {
        let boundaries = display_boundaries(line, segment, 4);
        assert_eq!(boundaries[0].char_offset, segment.char_start);
        assert_eq!(boundaries[0].display_byte, 0);
        assert_eq!(
            boundaries.last().map(|boundary| boundary.char_offset),
            Some(segment.char_end)
        );
        assert_eq!(
            boundaries
                .last()
                .map(|boundary| boundary.display_byte.get()),
            Some(segment.display.len())
        );
        assert_eq!(
            segment.char_start, previous_end,
            "segments must partition the hard line"
        );
        previous_end = segment.char_end.into();
    }
    assert_eq!(previous_end, line.chars().count());
}
