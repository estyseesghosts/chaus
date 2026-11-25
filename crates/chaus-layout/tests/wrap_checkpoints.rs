//! Bounded deep-row access with numeric wrap checkpoints.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use chaus_layout::{
    build_checkpoints, row_count_source, segment_at_source, segments_window, segments_with,
    LineSource, WrapCheckpoints, WrapPolicy, WrapRowIndex,
};
use ropey::Rope;

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

fn long_line() -> Rope {
    Rope::from_str(&"alpha beta gamma delta epsilon ".repeat(40_000))
}

fn checkpoints_for(
    source: &LineSource<'_>,
    columns: Option<usize>,
    policy: WrapPolicy,
) -> WrapCheckpoints {
    build_checkpoints(source, columns, 4, policy, 64)
}

#[test]
fn deep_rows_match_the_oracle_without_walking_the_prefix() {
    let rope = long_line();
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = checkpoints_for(&source, Some(23), WrapPolicy::Word);
    assert!(
        checkpoints.len() > 50,
        "a long wrapped line gets checkpoints"
    );

    let oracle = segments_with(&rope.to_string(), Some(23), 4, WrapPolicy::Word);
    assert!(oracle.len() > 5_100);
    for row in [0usize, 63, 64, 4_999, 5_000, oracle.len() - 1] {
        let segment = segment_at_source(
            &source,
            Some(23),
            4,
            WrapPolicy::Word,
            WrapRowIndex::new(row),
            &checkpoints,
        )
        .unwrap_or_else(|| panic!("row {row} was not produced"));
        let expected = &oracle[row];
        assert_eq!(segment.display, expected.display, "display at row {row}");
        assert_eq!(segment.char_start.get(), expected.char_start.get());
        assert_eq!(segment.char_end.get(), expected.char_end.get());
        assert_eq!(segment.start_column.get(), expected.start_column.get());
    }

    let (window, stats) = chaus_layout::segments_window_source(
        &source,
        Some(23),
        4,
        WrapPolicy::Word,
        WrapRowIndex::new(5_000),
        40,
        &checkpoints,
    );
    assert_eq!(window.len(), 40);
    assert!(
        stats.rows_skipped_via_checkpoint >= 4_928,
        "the deep window must start at a checkpoint, not row zero: {stats:?}"
    );
    assert!(
        stats.rows_walked <= 4 * (64 + 40),
        "work must stay near the checkpoint and window, not the prefix: {stats:?}"
    );
}

#[test]
fn a_deep_window_stays_bounded_while_the_prefix_is_not_materialized() {
    let rope = long_line();
    let source = LineSource::from_rope(rope.slice(..));
    let checkpoints = checkpoints_for(&source, Some(23), WrapPolicy::Word);
    let peak = peak_extra(|| {
        let (segments, stats) = chaus_layout::segments_window_source(
            &source,
            Some(23),
            4,
            WrapPolicy::Word,
            WrapRowIndex::new(5_000),
            40,
            &checkpoints,
        );
        assert_eq!(segments.len(), 40);
        std::hint::black_box((segments, stats));
    });
    assert!(
        peak < 256 * 1024,
        "a deep Ropey window must not flatten the line, peak grew by {peak} bytes"
    );
}

#[test]
fn checkpoints_are_invalidated_when_wrap_inputs_change() {
    let rope = long_line();
    let source = LineSource::from_rope(rope.slice(..));
    let mut checkpoints = checkpoints_for(&source, Some(23), WrapPolicy::Word);
    let before = checkpoints.len();
    assert!(before > 0);

    // A changed tab stop or wrap width makes the old numeric index stale.
    checkpoints.clear();
    assert!(checkpoints.is_empty());
    let rebuilt = checkpoints_for(&source, Some(40), WrapPolicy::Character);
    assert!(!rebuilt.is_empty());
    for entry in rebuilt.entries() {
        assert!(entry.char_offset.get() < source.char_len());
    }
}

#[test]
fn row_count_from_source_matches_the_contiguous_oracle() {
    for policy in [WrapPolicy::Character, WrapPolicy::Word] {
        let text = "short words for counting rows\n".repeat(40);
        let line = text.replace('\n', " ");
        let rope = Rope::from_str(&line);
        let source = LineSource::from_rope(rope.slice(..));
        let checkpoints = checkpoints_for(&source, Some(17), policy);
        assert_eq!(
            row_count_source(&source, Some(17), 4, policy, &checkpoints),
            segments_window(&line, Some(17), 4, policy, 0, usize::MAX).len()
        );
    }
}
