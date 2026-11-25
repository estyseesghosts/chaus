//! Windowed segment access for giant wrapped lines.

// The TLS initializers are already const; clippy 1.98 reports them twice.
#![allow(clippy::missing_const_for_thread_local)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use chaus_layout::{segments_window, segments_with, WrapPolicy};

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

fn peak_extra(measure: impl FnOnce()) -> usize {
    let before = LIVE.with(|live| live.get());
    PEAK.with(|peak| peak.set(before));
    measure();
    PEAK.with(|peak| peak.get()) - before
}

#[test]
fn a_window_matches_the_corresponding_slice_of_the_full_layout() {
    let text = "alpha beta gamma delta epsilon ".repeat(200);
    let full = segments_with(&text, Some(23), 4, WrapPolicy::Word);
    assert!(full.len() > 50);
    for (skip, limit) in [(0, 5), (7, 11), (40, 100), (full.len() - 1, 10), (0, 0)] {
        let window = segments_window(&text, Some(23), 4, WrapPolicy::Word, skip, limit);
        let expected: Vec<_> = full.iter().skip(skip).take(limit).cloned().collect();
        assert_eq!(window, expected, "skip {skip}, limit {limit}");
    }
}

#[test]
fn painting_a_screenful_of_a_megabyte_line_stays_bounded() {
    let line = "the quick brown fox jumps over the lazy dog ".repeat(23_500);
    assert!(line.len() > 1_000_000);

    let window = peak_extra(|| {
        let segments = segments_window(&line, Some(80), 4, WrapPolicy::Word, 0, 40);
        assert_eq!(segments.len(), 40);
        std::hint::black_box(segments);
    });
    assert!(
        window < 256 * 1024,
        "a screenful must not build the whole line, peak grew by {window} bytes"
    );

    // A deep window builds only its own display strings, not the skipped ones.
    let deep = peak_extra(|| {
        let segments = segments_window(&line, Some(80), 4, WrapPolicy::Word, 5_000, 40);
        assert_eq!(segments.len(), 40);
        std::hint::black_box(segments);
    });
    assert!(
        deep < 256 * 1024,
        "a deep window must stay bounded too, peak grew by {deep} bytes"
    );
}
