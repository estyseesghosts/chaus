//! A deep horizontal scroll must shape only a bounded source window.

// The TLS initializers are already const; clippy 1.98 reports them twice.
#![allow(clippy::missing_const_for_thread_local)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use chaus_layout::{
    build_column_checkpoints, column_at_source, unwrapped_window, ColumnCheckpoints,
    LineCharOffset, LineSource, VisualColumn,
};
use ropey::Rope;

struct Counting;

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
        let _ = LIVE.try_with(|live| live.set(live.get().saturating_sub(layout.size())));
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

fn line_and_target() -> (String, usize) {
    let pattern = "aé漢\t👩\u{200d}💻 x e\u{301}";
    let prefix = pattern.repeat(500);
    let target = prefix.chars().count();
    let mut line = prefix;
    line.push_str("TARGET");
    line.push_str(&pattern.repeat(100));
    line.push_str(&"x".repeat(1_100_000));
    (line, target)
}

fn rope_for(line: &str) -> Rope {
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

fn paint_window<'a>(
    source: &LineSource<'a>,
    first_column: usize,
    checkpoints: &ColumnCheckpoints,
) -> chaus_layout::UnwrappedWindow<'a> {
    unwrapped_window(
        source,
        VisualColumn::new(first_column),
        VisualColumn::new(80),
        VisualColumn::new(chaus_layout::OVERSCAN_COLUMNS),
        4,
        checkpoints,
    )
}

#[test]
fn painting_the_beginning_and_a_deep_scroll_are_both_bounded() {
    let (line, target) = line_and_target();
    let rope = rope_for(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = build_column_checkpoints(&source, 4, 4096);
    let target_column = column_at_source(&source, LineCharOffset::new(target), 4, &checkpoints);

    let first = peak_extra(|| {
        std::hint::black_box(paint_window(&source, 0, &checkpoints));
    });
    assert!(first < 256 * 1024, "first screen peaked at {first} bytes");

    let deep = peak_extra(|| {
        let window = paint_window(
            &source,
            target_column.get().saturating_sub(20),
            &checkpoints,
        );
        assert!(
            window.text.contains("TARGET"),
            "the deep window must contain the target grapheme run"
        );
        assert!(
            window.char_start.get() < target && window.char_end.get() > target,
            "the visible window must cover the deep target"
        );
        assert!(
            window.source_chars < 2 * 4096,
            "a deep window must stay bounded, got {} source chars",
            window.source_chars
        );
        std::hint::black_box(window);
    });
    assert!(deep < 256 * 1024, "deep screen peaked at {deep} bytes");
}

#[test]
fn tab_expansion_uses_absolute_columns_at_a_deep_window_start() {
    let (line, target) = line_and_target();
    let rope = rope_for(&line);
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = build_column_checkpoints(&source, 4, 4096);
    let target_column = column_at_source(&source, LineCharOffset::new(target), 4, &checkpoints);
    let window = paint_window(
        &source,
        target_column.get().saturating_sub(20),
        &checkpoints,
    );

    assert_eq!(
        window.start_column,
        column_at_source(&source, window.char_start, 4, &checkpoints),
        "the window start column must be the absolute logical-line column"
    );
    assert!(window.display.chars().count() >= window.text.chars().count());
}
