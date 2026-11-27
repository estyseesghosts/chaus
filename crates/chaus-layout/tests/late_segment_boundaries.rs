//! Display boundaries must use a segment's own logical column, not a window-relative one.

use chaus_layout::{
    display_boundaries, display_boundaries_source, segments_with, LineSource, VisualColumn,
    WrapPolicy,
};
use ropey::Rope;

fn chunked(text: &str) -> Rope {
    let mut rope = Rope::new();
    let mut rest = text;
    while !rest.is_empty() {
        let take = rest
            .char_indices()
            .nth(37)
            .map_or(rest.len(), |(index, _)| index);
        rope.append(Rope::from_str(&rest[..take]));
        rest = &rest[take..];
    }
    rope
}

#[test]
fn a_late_segment_maps_to_its_own_display_bytes() {
    // The prefix contains tabs and multi-byte graphemes so the segment's
    // start column is far from zero and cannot be mistaken for a window start.
    let prefix = "aé漢\t👩\u{200d}💻\t".repeat(30);
    let line = format!("{prefix}{}", "x\té\t漢\t👩\u{200d}💻\tzzzz wwww".repeat(6));
    let segments = segments_with(&line, Some(12), 3, WrapPolicy::Word);
    assert!(segments.len() > 2, "the line should wrap into several rows");
    assert!(
        segments
            .iter()
            .any(|segment| segment.char_start.get() > 128),
        "a segment must start beyond the source window context"
    );

    let target = segments
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, segment)| {
            if segment.char_start.get() <= 128 || segment.columns.get() == 0 {
                return false;
            }
            line.chars()
                .skip(segment.char_start.get())
                .take(segment.char_end.get() - segment.char_start.get())
                .any(|character| character == '\t')
        })
        .map(|(index, _)| index)
        .expect("a later, non-empty segment containing a tab");
    let segment = &segments[target];
    let rope = chunked(&line);
    let source = LineSource::from_rope(rope.slice(..));

    let actual = display_boundaries_source(&source, segment, 3);
    let expected = display_boundaries(&line, segment, 3);

    assert_eq!(actual.len(), expected.len(), "one boundary per grapheme");
    for (index, (left, right)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            left.char_offset.get(),
            right.char_offset.get(),
            "source offset of boundary {index}"
        );
        assert_eq!(
            left.display_byte.get(),
            right.display_byte.get(),
            "display byte of boundary {index}"
        );
    }
    assert_eq!(
        actual.first().map(|b| b.char_offset),
        Some(segment.char_start)
    );
    assert_eq!(actual.last().map(|b| b.char_offset), Some(segment.char_end));
    assert_eq!(
        actual.last().map(|b| b.display_byte.get()),
        Some(segment.display.len()),
        "the mapping must cover exactly this segment"
    );
}

#[test]
fn late_column_argument_is_consistent_with_the_segment_prefix() {
    let line = format!(
        "{}{}",
        "aé漢\t👩\u{200d}💻\t".repeat(30),
        "zzzz\twwww\tyyyy\t漢字\t👩\u{200d}💻\t".repeat(4)
    );
    let segments = segments_with(&line, Some(10), 4, WrapPolicy::Word);
    let segment = segments.last().expect("several rows");
    assert!(segment.char_start.get() > 128, "late segment required");
    let rope = chunked(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let boundaries = display_boundaries_source(&source, segment, 4);
    let oracle = display_boundaries(&line, segment, 4);
    assert_eq!(
        boundaries
            .iter()
            .map(|b| (b.char_offset.get(), b.display_byte.get()))
            .collect::<Vec<_>>(),
        oracle
            .iter()
            .map(|b| (b.char_offset.get(), b.display_byte.get()))
            .collect::<Vec<_>>(),
        "a tab inside a late window still expands against the logical line"
    );
    let _ = VisualColumn::new(segment.start_column.get());
}
