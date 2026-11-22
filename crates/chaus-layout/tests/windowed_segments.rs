//! Windowed segment access for giant wrapped lines.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use chaus_layout::{segments_window, segments_with, WrapPolicy};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn peak_extra(measure: impl FnOnce()) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    measure();
    PEAK.load(Ordering::Relaxed) - before
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
