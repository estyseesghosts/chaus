//! Header-first discovery, duplicate/stale pruning and shutdown draining.

use std::time::{Duration, SystemTime};

use chaus_core::eol::Eol;
use chaus_io::recovery::{self, PendingRecoveryOp, RecoveryRecord, DEFAULT_MAX_AGE};

fn record(id: u64, path: Option<&str>, text: &str) -> RecoveryRecord {
    RecoveryRecord::new(
        id,
        path.map(std::path::PathBuf::from),
        text,
        Eol::Lf,
        false,
        1,
    )
}

#[test]
fn scan_returns_headers_and_read_loads_the_selected_body() {
    let directory = tempfile::tempdir().unwrap();
    let mut first = record(1, Some("C:/notes/README.md"), "first body");
    first.saved_at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    recovery::write_to(directory.path(), &first).unwrap();

    let entries = recovery::scan_in_without_age(directory.path());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, 1);
    assert_eq!(entries[0].display_name(), "README.md");
    assert!(
        entries[0].display_detail().contains("C:/notes/README.md"),
        "the prompt must disambiguate identical basenames: {}",
        entries[0].display_detail()
    );
    assert_eq!(entries[0].read(), Some(first));
}

#[test]
fn a_renamed_snapshot_is_rejected_when_its_filename_disagrees_with_the_embedded_id() {
    let directory = tempfile::tempdir().unwrap();
    let original = record(7, None, "ghost");
    recovery::write_to(directory.path(), &original).unwrap();
    let canonical = directory.path().join(format!("recovery-{:016x}.rec", 7));
    let renamed = directory.path().join(format!("recovery-{:016x}.rec", 8));
    std::fs::rename(canonical, renamed).unwrap();

    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "a file whose name claims another id must never be offered or retired"
    );
}

#[test]
fn default_scan_prunes_snapshots_older_than_the_age_limit() {
    let directory = tempfile::tempdir().unwrap();
    let mut old = record(9, None, "old");
    old.saved_at = SystemTime::now()
        .checked_sub(DEFAULT_MAX_AGE + Duration::from_secs(60))
        .unwrap();
    recovery::write_to(directory.path(), &old).unwrap();

    assert!(recovery::scan_in(directory.path()).is_empty());
    assert!(
        !directory
            .path()
            .join("recovery-0000000000000009.rec")
            .exists(),
        "an aged-out snapshot is removed, not merely hidden"
    );
    assert_eq!(
        recovery::scan_in_without_age(directory.path()).len(),
        0,
        "the same snapshot is gone from disk"
    );
}

#[test]
fn identical_snapshots_for_one_path_collapse_to_the_newest() {
    let directory = tempfile::tempdir().unwrap();
    let mut newer = record(21, Some("C:/notes/README.md"), "same text");
    newer.saved_at = SystemTime::now();
    let mut older = record(20, Some("C:/notes/README.md"), "same text");
    older.saved_at = newer.saved_at - Duration::from_secs(10);
    recovery::write_to(directory.path(), &older).unwrap();
    recovery::write_to(directory.path(), &newer).unwrap();

    let entries = recovery::scan_in_without_age(directory.path());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, 21);
    assert!(
        !directory
            .path()
            .join("recovery-0000000000000014.rec")
            .exists(),
        "the older duplicate is retired"
    );
}

#[test]
fn distinct_unsaved_text_for_one_path_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to(
        directory.path(),
        &record(30, Some("C:/notes/README.md"), "first draft"),
    )
    .unwrap();
    recovery::write_to(
        directory.path(),
        &record(31, Some("C:/notes/README.md"), "second draft"),
    )
    .unwrap();

    let entries = recovery::scan_in_without_age(directory.path());
    assert_eq!(
        entries.len(),
        2,
        "deduplication must not discard genuinely different unsaved work"
    );
}

#[test]
fn a_snapshot_already_matched_by_the_saved_file_is_pruned() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("saved.txt");
    let snapshot = record(40, Some(saved.to_str().unwrap()), "already saved\n");
    recovery::write_to(directory.path(), &snapshot).unwrap();
    // Make the saved file strictly newer than the snapshot, as it is after a
    // successful Save in another window.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&saved, "already saved\n").unwrap();

    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "a snapshot that matches the current file has nothing to recover"
    );
    assert!(
        !directory
            .path()
            .join("recovery-0000000000000028.rec")
            .exists(),
        "the stale snapshot is removed from disk"
    );
}

#[test]
fn a_non_canonical_snapshot_name_is_retired_from_its_actual_path() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to(directory.path(), &record(12, None, "renamed")).unwrap();
    let canonical = directory.path().join(format!("recovery-{:016x}.rec", 12));
    let renamed = directory.path().join("recovery-c.rec");
    std::fs::rename(canonical, &renamed).unwrap();

    let entries = recovery::scan_in_without_age(directory.path());
    assert_eq!(entries.len(), 1, "the embedded id still matches the name");
    recovery::retire_entries(&entries, 1).unwrap();
    assert!(
        !renamed.exists(),
        "retirement must delete the file discovery loaded, not a reconstructed name"
    );
}

#[test]
fn draining_waits_for_an_in_flight_recovery_operation() {
    let (release, gate) = std::sync::mpsc::channel();
    let ticket = PendingRecoveryOp::begin();
    let worker = std::thread::spawn(move || {
        let _ticket = ticket;
        let _ = gate.recv();
    });

    assert!(
        !recovery::drain_pending(Duration::from_millis(20)),
        "a held operation must block the drain"
    );
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(
        recovery::drain_pending(Duration::from_secs(5)),
        "the drain finishes once the operation is released"
    );
    assert_eq!(recovery::pending_operations(), 0);
}
