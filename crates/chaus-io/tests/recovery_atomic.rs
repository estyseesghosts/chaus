use std::time::{Duration, UNIX_EPOCH};

use chaus_core::eol::Eol;
use chaus_io::recovery::{self, RecoveryRecord};

fn record(id: u64, text: &str, revision: u64) -> RecoveryRecord {
    let mut record = RecoveryRecord::new(id, None, text, Eol::Lf, false, revision);
    record.saved_at = UNIX_EPOCH + Duration::from_secs(100 + revision);
    record
}

#[test]
fn an_older_epoch_cannot_replace_a_newer_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to_with_epoch(directory.path(), &record(7, "newer", 11), 10).unwrap();
    recovery::write_to_with_epoch(directory.path(), &record(7, "older", 12), 4).unwrap();

    let records = recovery::list_in(directory.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].text, "newer");
}

#[test]
fn retiring_suppresses_in_flight_writes_but_allows_later_edits() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to_with_epoch(directory.path(), &record(3, "dirty", 2), 2).unwrap();
    recovery::retire_in(directory.path(), 3, 3).unwrap();
    assert!(recovery::list_in(directory.path()).is_empty());

    // A debounced write from before the retirement must not resurrect the
    // snapshot.
    recovery::write_to_with_epoch(directory.path(), &record(3, "stale", 2), 2).unwrap();
    assert!(recovery::list_in(directory.path()).is_empty());

    // Editing again advances the epoch, so a fresh snapshot is accepted.
    recovery::write_to_with_epoch(directory.path(), &record(3, "again", 4), 4).unwrap();
    let records = recovery::list_in(directory.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].text, "again");
}

#[test]
fn concurrent_writes_leave_one_complete_record_and_no_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        for index in 0..16 {
            let directory = directory.path().to_path_buf();
            scope.spawn(move || {
                let text = format!("revision {}", index);
                recovery::write_to_with_epoch(&directory, &record(5, &text, index), index).unwrap();
            });
        }
    });

    let records = recovery::list_in(directory.path());
    assert_eq!(records.len(), 1);
    assert!(records[0].text.starts_with("revision "));

    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".chaus-recovery-")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files must be cleaned up: {leftovers:?}"
    );
}

#[test]
fn a_removed_snapshot_is_gone_even_if_an_older_write_finishes_late() {
    let directory = tempfile::tempdir().unwrap();
    recovery::write_to_with_epoch(directory.path(), &record(8, "saved", 9), 9).unwrap();
    recovery::remove_in(directory.path(), 8).unwrap();
    recovery::write_to_with_epoch(directory.path(), &record(8, "late", 8), 8).unwrap();
    assert!(recovery::list_in(directory.path()).is_empty());
}
