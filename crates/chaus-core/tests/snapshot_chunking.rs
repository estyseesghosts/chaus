//! Snapshot cost and structural sharing for newline metadata.
//!
//! A background reflow captures a `DocumentSnapshot` while editing continues.
//! These tests pin the promise that capturing stays proportional to chunk
//! count, that edits cannot mutate an older snapshot, and that mixed-EOL data
//! survives across chunk boundaries and transaction replay.

use std::time::Instant;

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, Selection};

fn insert(document: &mut Document, at: usize, text: &str) {
    let caret = CharOffset(at);
    document
        .apply(EditTransaction::new(
            EditCause::Typing,
            TextEdit::insertion(caret, text),
            Selection::caret(caret),
            Selection::caret(CharOffset(at + text.chars().count())),
        ))
        .expect("insert should apply");
}

#[test]
fn mixed_eol_round_trips_across_chunk_boundaries() {
    let mut kinds = Vec::new();
    let mut source = String::new();
    for index in 0..2_000 {
        match index % 3 {
            0 => {
                source.push_str("a\r\n");
                kinds.push(Eol::CrLf);
            }
            1 => {
                source.push_str("b\n");
                kinds.push(Eol::Lf);
            }
            _ => {
                source.push_str("c\r");
                kinds.push(Eol::Cr);
            }
        }
    }

    let (normalized, map) = EolMap::from_source(&source);
    assert!(map.chunk_count() > 1, "fixture spans several chunks");
    assert_eq!(map.kinds(), kinds);
    assert_eq!(map.apply_to(&normalized, Eol::Lf), source);
}

#[test]
fn edits_after_snapshot_cannot_change_it() {
    let mut document = Document::from_source_text("a\r\nb\nc\rd");
    let snapshot = document.snapshot();
    let before = snapshot.text_with_eol();

    insert(&mut document, 0, "x\ny\n");
    document.undo();

    assert_eq!(snapshot.text_with_eol(), before);
    assert_eq!(before, "a\r\nb\nc\rd");
    assert_eq!(document.text_with_eol(), "a\r\nb\nc\rd");
}

#[test]
fn snapshot_cost_tracks_chunks_not_lines() {
    for lines in [100_000usize, 1_000_000] {
        let source = "x\n".repeat(lines);
        let document = Document::from_source_text(&source);

        let started = Instant::now();
        let snapshot = document.snapshot();
        let elapsed = started.elapsed();

        assert_eq!(snapshot.line_count(), lines + 1);
        assert!(
            snapshot.eol_chunk_count() < lines / 16,
            "snapshot held {} chunks for {lines} lines",
            snapshot.eol_chunk_count()
        );
        assert!(
            elapsed.as_millis() < 100,
            "snapshot of {lines} lines took {elapsed:?}"
        );
    }
}

#[test]
fn large_single_line_snapshot_does_not_scan_text() {
    let source = "a".repeat(1 << 20);
    let document = Document::from_source_text(&source);

    let started = Instant::now();
    let snapshot = document.snapshot();
    let elapsed = started.elapsed();

    assert_eq!(snapshot.line_count(), 1);
    assert_eq!(snapshot.eol_chunk_count(), 1);
    assert!(snapshot.eol_heap_bytes() < 1 << 16);
    assert!(elapsed.as_millis() < 50, "snapshot took {elapsed:?}");
}
