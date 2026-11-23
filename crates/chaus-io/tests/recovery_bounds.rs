//! A snapshot that cannot be read back must never be written.

use chaus_core::eol::Eol;
use chaus_io::recovery::{self, RecoveryError, RecoveryRecord, MAX_RECORD_BYTES};

fn record(id: u64, text: String) -> RecoveryRecord {
    RecoveryRecord::new(id, None, text, Eol::Lf, false, 1)
}

#[test]
fn an_oversized_snapshot_is_rejected_without_replacing_a_valid_one() {
    let directory = tempfile::tempdir().unwrap();
    let original = record(1, "still recoverable".to_owned());
    recovery::write_to(directory.path(), &original).unwrap();

    let oversized = record(1, "x".repeat(MAX_RECORD_BYTES as usize + 1));
    let error = recovery::write_to(directory.path(), &oversized).unwrap_err();
    match error {
        RecoveryError::TooLarge { bytes, limit } => {
            assert_eq!(limit, MAX_RECORD_BYTES);
            assert!(bytes > limit);
        }
        other => panic!("oversized snapshot must be rejected by size: {other}"),
    }

    let records = recovery::list_in(directory.path());
    assert_eq!(records.len(), 1, "the previous usable snapshot must remain");
    assert_eq!(records[0].text, "still recoverable");
}

#[test]
fn a_snapshot_at_the_readable_limit_is_accepted() {
    let directory = tempfile::tempdir().unwrap();
    // The header is less than 128 bytes; this record must stay under the cap.
    let text = "x".repeat(MAX_RECORD_BYTES as usize - 128);
    recovery::write_to(directory.path(), &record(2, text.clone())).unwrap();
    let records = recovery::list_in(directory.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].text, text);
}
