//! Oracle parity between contiguous text and Ropey-backed windowed input.

use chaus_layout::{
    build_checkpoints, char_at_column, char_at_column_source, column_at, column_at_source,
    display_boundaries, display_boundaries_source, row_count_source, row_of_char,
    row_of_char_source, segments_window, segments_window_source, LineCharOffset, LineSource,
    VisualColumn, WrapPolicy, WrapRowIndex,
};
use ropey::Rope;

fn chunked(text: &str) -> Rope {
    let mut rope = Rope::new();
    let mut rest = text;
    while !rest.is_empty() {
        let take = rest
            .char_indices()
            .nth(7)
            .map_or(rest.len(), |(index, _)| index);
        rope.append(Rope::from_str(&rest[..take]));
        rest = &rest[take..];
    }
    rope
}

fn corpus() -> Vec<&'static str> {
    vec![
        "hello world, a plain line",
        "one\ttab\tstop\tand text",
        "漢字のテキスト e\u{301} 👩\u{200d}💻 mixed",
        "a verylongtokenwithoutanybreakopportunityxxxxxxxxx end",
        "trailing whitespace      ",
        "\r\n mixed \r endings \n",
        "",
        "x",
    ]
}

fn compare_segments(text: &str, columns: Option<usize>, tab_stop: usize, policy: WrapPolicy) {
    let contiguous = LineSource::from_str(text);
    let rope = chunked(text);
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = build_checkpoints(&source, columns, tab_stop, policy, 4);
    let (actual, _) = segments_window_source(
        &source,
        columns,
        tab_stop,
        policy,
        WrapRowIndex::new(0),
        usize::MAX,
        &checkpoints,
    );
    let expected = segments_window(text, columns, tab_stop, policy, 0, usize::MAX);
    assert_eq!(actual.len(), expected.len(), "row count for {text:?}");
    for (index, (left, right)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            left.display, right.display,
            "display of row {index} in {text:?}"
        );
        assert_eq!(left.char_start.get(), right.char_start.get());
        assert_eq!(left.char_end.get(), right.char_end.get());
        assert_eq!(left.start_column.get(), right.start_column.get());
        assert_eq!(left.columns.get(), right.columns.get());
    }
    let row_count = chaus_layout::row_count_with(text, columns, tab_stop, policy);
    assert_eq!(
        row_count_source(&source, columns, tab_stop, policy, &checkpoints),
        row_count,
        "row count of {text:?} at {columns:?}/{tab_stop}/{policy:?}"
    );
    assert_eq!(
        row_count_with_checkpoint_parity(&contiguous, &source, columns, tab_stop, policy),
        row_count
    );
}

fn row_count_with_checkpoint_parity(
    contiguous: &LineSource<'_>,
    source: &LineSource<'_>,
    columns: Option<usize>,
    tab_stop: usize,
    policy: WrapPolicy,
) -> usize {
    let checkpoints = build_checkpoints(contiguous, columns, tab_stop, policy, 4);
    row_count_source(source, columns, tab_stop, policy, &checkpoints)
}

#[test]
fn chunked_rope_matches_contiguous_layout_for_every_policy() {
    for text in corpus() {
        for columns in [None, Some(1), Some(4), Some(80)] {
            for tab_stop in [1, 4, 8] {
                for policy in [WrapPolicy::Character, WrapPolicy::Word] {
                    compare_segments(text, columns, tab_stop, policy);
                }
            }
        }
    }
}

#[test]
fn chunk_boundaries_do_not_split_graphemes_or_line_breaks() {
    let text = "👩\u{200d}💻\te\u{301}\t漢字\r\nlast";
    let rope = chunked(text);
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = build_checkpoints(&source, Some(6), 4, WrapPolicy::Word, 2);
    let (segments, _) = segments_window_source(
        &source,
        Some(6),
        4,
        WrapPolicy::Word,
        WrapRowIndex::new(0),
        usize::MAX,
        &checkpoints,
    );
    let end = segments.last().expect("the line has rows").char_end.get();
    assert_eq!(end, text.chars().count(), "every source char is covered");

    let boundaries = display_boundaries_source(
        &source,
        segments.first().expect("first segment"),
        4,
        VisualColumn::new(0),
    );
    let direct = display_boundaries(text, &segments[0], 4);
    assert_eq!(boundaries.len(), direct.len());
    for (left, right) in boundaries.iter().zip(direct.iter()) {
        assert_eq!(left.char_offset.get(), right.char_offset.get());
        assert_eq!(left.display_byte.get(), right.display_byte.get());
    }
}

#[test]
fn caret_lookup_and_column_mapping_match_the_contiguous_oracle() {
    for text in corpus() {
        let rope = chunked(text);
        let source = LineSource::from_rope(rope.slice(..));
        let checkpoints = build_checkpoints(&source, Some(4), 4, WrapPolicy::Character, 3);
        let count = text.chars().count();
        for offset in 0..=count {
            let expected_row = row_of_char(text, Some(4), 4, WrapPolicy::Character, offset);
            let found = row_of_char_source(
                &source,
                Some(4),
                4,
                WrapPolicy::Character,
                LineCharOffset::new(offset),
                &checkpoints,
            );
            assert_eq!(
                found.row.get(),
                expected_row.get(),
                "row of char {offset} in {text:?}"
            );
            let expected_column = column_at(text, offset, 4);
            let actual = column_at_source(&source, LineCharOffset::new(offset), 4, &checkpoints);
            assert_eq!(
                actual.get(),
                expected_column.get(),
                "column of char {offset}"
            );
        }
        for column in 0..=8 {
            let expected = char_at_column(text, column, 4);
            let actual = char_at_column_source(&source, VisualColumn::new(column), 4, &checkpoints);
            assert_eq!(
                actual.get(),
                expected.get(),
                "char at column {column} in {text:?}"
            );
        }
    }
}
