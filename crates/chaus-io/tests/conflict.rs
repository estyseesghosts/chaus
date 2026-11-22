//! Conflict checks must never treat an unreadable target as unchanged.

use std::fs;
use std::io::Write as _;

use chaus_core::eol::Eol;
use chaus_io::{check_conflict, load, save, ConflictCheckError, SaveError, SaveRequest};

#[test]
fn check_conflict_reports_an_unreadable_target_instead_of_success() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("target.txt");
    {
        let mut file = fs::File::create(&path).expect("create fixture");
        file.write_all(b"text").expect("write fixture");
    }
    let loaded = load(&path).expect("load");

    // Replace the file with a directory of the same name. Reading the target
    // now fails with an OS error that is neither "not found" nor a content
    // change.
    fs::remove_file(&path).expect("remove fixture");
    fs::create_dir(&path).expect("create directory");

    match check_conflict(&loaded.meta) {
        Err(ConflictCheckError::Io { .. }) => {}
        other => panic!("expected an I/O error, got {other:?}"),
    }
}

#[test]
fn save_refuses_when_the_target_cannot_be_read() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("target.txt");
    fs::write(&path, b"original").expect("write fixture");
    let loaded = load(&path).expect("load");

    fs::remove_file(&path).expect("remove fixture");
    fs::create_dir(&path).expect("create directory");

    let error = save(SaveRequest::new(
        &path,
        &path,
        b"replacement",
        false,
        Eol::Lf,
        Some(&loaded.meta),
    ))
    .expect_err("must refuse to replace an unreadable target");

    assert!(matches!(&error, SaveError::Io { .. }), "got {error:?}");
    assert!(path.is_dir(), "the unreadable target must be left in place");
}

#[test]
fn save_refuses_to_break_additional_hard_links() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("target.txt");
    let other_link = directory.path().join("other-link.txt");
    fs::write(&path, b"original").expect("write fixture");
    fs::hard_link(&path, &other_link).expect("hard link");

    let error = save(SaveRequest::new(
        &path,
        &path,
        b"replacement",
        false,
        Eol::Lf,
        None,
    ))
    .expect_err("a multiply-linked target must not be replaced silently");

    match &error {
        SaveError::MultipleLinks { links, .. } => assert_eq!(*links, 2),
        other => panic!("expected a hard-link refusal, got {other:?}"),
    }
    assert_eq!(fs::read(&path).expect("read target"), b"original");
    assert_eq!(fs::read(&other_link).expect("read link"), b"original");
}
