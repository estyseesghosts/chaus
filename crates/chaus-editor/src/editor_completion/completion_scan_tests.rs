//! A truncated directory scan must not be reported as "no such file".

use super::{empty_listing_message, list_matches_with_limit};

#[test]
fn a_truncated_scan_reports_truncation_instead_of_a_missing_match() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("wanted.txt"), "text").unwrap();
    std::fs::write(directory.path().join("other.txt"), "text").unwrap();

    // Stop before examining any entry, as a bounded scan does when the match
    // lives past the examined prefix.
    let listing = list_matches_with_limit(directory.path(), "wanted", 0, 256);
    assert!(listing.names.is_empty());
    assert!(listing.truncated);
    assert!(
        empty_listing_message(listing.truncated).contains("truncated"),
        "{}",
        empty_listing_message(listing.truncated)
    );
}

#[test]
fn an_exhausted_scan_still_reports_no_match() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("other.txt"), "text").unwrap();

    let listing = list_matches_with_limit(directory.path(), "wanted", 4096, 256);
    assert!(listing.names.is_empty());
    assert!(!listing.truncated);
    assert_eq!(empty_listing_message(false), "No matching files");
}

#[test]
fn a_candidate_cap_also_marks_the_listing_truncated() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..4 {
        std::fs::write(directory.path().join(format!("file-{index}.txt")), "text").unwrap();
    }
    let listing = list_matches_with_limit(directory.path(), "file-", 4096, 2);
    assert_eq!(listing.names.len(), 2);
    assert!(listing.truncated);
}
