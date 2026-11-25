//! The row walker must not allocate in proportion to the hard line.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use chaus_layout::{column_at, row_count_with, rows, WrapPolicy};

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

/// Peak bytes allocated while `measure` runs, relative to the live total when
/// it started.
fn peak_extra(measure: impl FnOnce()) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    measure();
    PEAK.load(Ordering::Relaxed) - before
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
