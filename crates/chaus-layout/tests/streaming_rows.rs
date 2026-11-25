//! The row walker must not allocate in proportion to the hard line.

// The TLS initializers are already const; clippy 1.98 reports them twice.
#![allow(clippy::missing_const_for_thread_local)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use chaus_layout::{column_at, row_count_with, rows, WrapPolicy};

struct Counting;

// Per-thread counters. A process-wide counter also sees allocations from the
// other tests in this binary, which run in parallel and make the measured peak
// depend on scheduling rather than on the code under test.
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

/// Peak bytes allocated while `measure` runs, relative to the live total when
/// it started.
fn peak_extra(measure: impl FnOnce()) -> usize {
    let before = LIVE.with(|live| live.get());
    PEAK.with(|peak| peak.set(before));
    measure();
    PEAK.with(|peak| peak.get()) - before
}

#[test]
fn a_megabyte_line_is_walked_in_constant_memory() {
    // One hard line, no newlines: the pathological case where materializing
    // every grapheme used to cost tens of megabytes.
    let line = "the quick brown fox jumps over the lazy dog ".repeat(23_500);
    assert!(line.len() > 1_000_000);

    let count = peak_extra(|| {
        let rows = row_count_with(&line, Some(80), 4, WrapPolicy::Word);
        assert!(rows > 10_000, "the fixture must actually wrap");
        // Keep the value alive so the closure cannot be optimized away.
        std::hint::black_box(rows);
    });
    assert!(
        count < 512 * 1024,
        "walking a {} byte line must stay bounded, peak grew by {count} bytes",
        line.len()
    );

    let mut previous_end = 0usize;
    for row in rows(&line, Some(80), 4, WrapPolicy::Word) {
        assert_eq!(row.char_start, previous_end, "rows must stay contiguous");
        previous_end = row.char_end.into();
    }
    assert_eq!(previous_end, line.chars().count());
    assert_eq!(
        row_count_with(&line, Some(80), 4, WrapPolicy::Word),
        rows(&line, Some(80), 4, WrapPolicy::Word).count()
    );
    assert_eq!(
        column_at(&line, previous_end, 4),
        line.chars().count(),
        "every ASCII character occupies one column"
    );
}
