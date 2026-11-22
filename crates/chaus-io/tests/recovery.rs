use std::time::{Duration, UNIX_EPOCH};

use chaus_core::eol::Eol;
use chaus_io::recovery::{self, RecoveryRecord};

fn record(id: u64, text: &str) -> RecoveryRecord {
    let mut record = RecoveryRecord::new(
        id,
        Some(std::path::PathBuf::from("C:/notes/example.txt")),
        text,
        Eol::CrLf,
        true,
        7,
    );
    record.saved_at = UNIX_EPOCH + Duration::from_secs(100 + id);
    record
}

#[test]
fn snapshots_round_trip_and_replace_the_same_id() {
    let directory = tempfile::tempdir().unwrap();
    let first = record(1, "one\r\ntwo\r\n");
    recovery::write_to(directory.path(), &first).unwrap();
    let second = record(1, "one\r\ntwo\r\nthree\r\n");
    recovery::write_to(directory.path(), &second).unwrap();

    let records = recovery::list_in(directory.path());
    assert_eq!(records, vec![second.clone()]);
    assert_eq!(records[0].display_name(), "example.txt");
    assert_eq!(records[0].preferred_eol, Eol::CrLf);
    assert!(records[0].had_utf8_bom);
    assert_eq!(records[0].revision, 7);
}

#[test]
fn listing_is_newest_first_and_skips_corrupt_files() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to(directory.path(), &record(1, "old")).unwrap();
    recovery::write_to(directory.path(), &record(2, "new")).unwrap();
    std::fs::write(
        directory.path().join("recovery-deadbeef.rec"),
        b"not a record",
    )
    .unwrap();
    std::fs::write(directory.path().join("unrelated.txt"), b"ignore me").unwrap();

    let records = recovery::list_in(directory.path());
    assert_eq!(
        records.iter().map(|record| record.id).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn removing_a_snapshot_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to(directory.path(), &record(4, "text")).unwrap();
    recovery::remove_in(directory.path(), 4).unwrap();
    recovery::remove_in(directory.path(), 4).unwrap();
    assert!(recovery::list_in(directory.path()).is_empty());
}

#[test]
fn missing_directories_list_as_empty() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("not-created");
    assert!(recovery::list_in(&missing).is_empty());
}
