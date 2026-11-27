//! A 1 MiB unwrapped hard line must not be materialized by layout queries.

// The TLS initializers are already const; clippy 1.98 reports them twice.
#![allow(clippy::missing_const_for_thread_local)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use chaus_layout::{
    build_checkpoints, build_column_checkpoints, char_at_column_source, column_at_source,
    row_count_source, row_of_char_source, LineCharOffset, LineSource, WrapPolicy, WrapRowIndex,
};
use ropey::Rope;

struct Counting;

// Per-thread counters keep parallel tests from contaminating a peak reading.
thread_local! {
    static LIVE: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let _ = LIVE.try_with(|live| {
                let current = live.get() + layout.size();
                live.set(current);
                let _ = PEAK.try_with(|peak| {
                    if current > peak.get() {
                        peak.set(current);
                    }
                });
            });
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = LIVE.try_with(|live| {
            live.set(live.get().saturating_sub(layout.size()));
        });
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn peak_extra(measure: impl FnOnce()) -> usize {
    let before = LIVE.with(|live| live.get());
    PEAK.with(|peak| peak.set(before));
    measure();
    PEAK.with(|peak| peak.get()) - before
}

const PATTERN: &str = "aé漢\t👩\u{200d}💻 x e\u{301}";

fn giant_line() -> (String, usize) {
    let prefix = PATTERN.repeat(500);
    let target = prefix.chars().count();
    let mut line = prefix;
    line.push_str("TARGET");
    line.push_str(&PATTERN.repeat(100));
    line.push_str(&"x".repeat(1_100_000));
    assert!(line.len() > 1_000_000, "the hard line must exceed 1 MiB");
    (line, target)
}

fn chunked_rope(line: &str) -> Rope {
    // Build the Rope from many chunks so the query cannot rely on contiguous
    // storage inside a single Rope leaf.
    let mut rope = Rope::new();
    let mut rest = line;
    while !rest.is_empty() {
        let take = rest
            .char_indices()
            .nth(997)
            .map_or(rest.len(), |(index, _)| index);
        rope.append(Rope::from_str(&rest[..take]));
        rest = &rest[take..];
    }
    rope
}

#[test]
fn one_row_is_reported_without_materializing_the_line() {
    let (line, _) = giant_line();
    let rope = chunked_rope(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let allocated = peak_extra(|| {
        let checkpoints = build_checkpoints(&source, None, 4, WrapPolicy::Word, 256);
        assert_eq!(
            row_count_source(&source, None, 4, WrapPolicy::Word, &checkpoints),
            1,
            "an unwrapped hard line is exactly one visual row"
        );
        std::hint::black_box(checkpoints);
    });
    assert!(
        allocated < 64 * 1024,
        "counting one row must not flatten a {} byte line; peak grew by {allocated} bytes",
        line.len()
    );
}

#[test]
fn deep_offset_lookup_does_not_stop_at_a_window_edge() {
    let (line, target) = giant_line();
    assert!(target > 4096, "target must be past a 4096-char window");
    let rope = chunked_rope(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = build_checkpoints(&source, None, 4, WrapPolicy::Word, 256);
    let columns = build_column_checkpoints(&source, 4, 4096);
    assert!(columns.len() > 1, "a 1 MiB line needs several checkpoints");

    let row = row_of_char_source(
        &source,
        None,
        4,
        WrapPolicy::Word,
        LineCharOffset::new(target),
        &checkpoints,
    );
    assert_eq!(row.row, WrapRowIndex::new(0));

    let column = column_at_source(&source, LineCharOffset::new(target), 4, &columns);
    let found = char_at_column_source(&source, column, 4, &columns);
    assert_eq!(
        found.get(),
        target,
        "column-to-char lookup must continue past the first window"
    );
}

#[test]
fn deep_selection_round_trips_through_columns() {
    let (line, target) = giant_line();
    let rope = chunked_rope(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let columns = build_column_checkpoints(&source, 4, 4096);

    let selection_end = target + "TARGET".chars().count();
    let start = column_at_source(&source, LineCharOffset::new(target), 4, &columns);
    let end = column_at_source(&source, LineCharOffset::new(selection_end), 4, &columns);
    assert_eq!(
        char_at_column_source(&source, start, 4, &columns).get(),
        target,
        "selection start beyond char 4096"
    );
    assert_eq!(
        char_at_column_source(&source, end, 4, &columns).get(),
        selection_end,
        "selection end beyond char 4096"
    );
}
