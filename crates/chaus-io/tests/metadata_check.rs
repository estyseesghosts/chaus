//! Cheap metadata gating for the periodic disk watcher.
//!
//! `metadata_matches` is only an idle-tick shortcut. These tests pin both
//! halves of that contract: it must notice the cheap signals, and content
//! hashing must still catch the cases metadata cannot (a same-size edit whose
//! timestamp was preserved).

use std::io::Write as _;

use chaus_io::{ConflictCheckError, SaveConflict};

fn loaded(path: &std::path::Path) -> chaus_io::FileMeta {
    chaus_io::load(path).unwrap().meta
}

#[test]
fn an_untouched_file_matches_its_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("idle.txt");
    std::fs::write(&path, "one\ntwo\n").unwrap();
    let meta = loaded(&path);

    assert!(chaus_io::metadata_matches(&meta));
    assert!(chaus_io::check_conflict(&meta).is_ok());
}

#[test]
fn a_size_change_is_caught_without_reading_the_contents() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("grown.txt");
    std::fs::write(&path, "short\n").unwrap();
    let meta = loaded(&path);

    std::fs::write(&path, "a much longer body\n").unwrap();

    assert!(!chaus_io::metadata_matches(&meta));
    assert!(matches!(
        chaus_io::check_conflict(&meta),
        Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk))
    ));
}

#[test]
fn a_same_size_edit_is_still_caught_when_the_timestamp_is_restored() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sneaky.txt");
    std::fs::write(&path, "aaaa\n").unwrap();
    let meta = loaded(&path);
    let original_modified = meta.modified.expect("a fresh file has an mtime");

    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.write_all(b"bbbb\n").unwrap();
    file.flush().unwrap();
    file.set_modified(original_modified).unwrap();
    drop(file);

    // Size and mtime now look unchanged, so the watcher would skip the hash on
    // an ordinary tick...
    assert!(
        chaus_io::metadata_matches(&meta),
        "the cheap check is allowed to miss this edit"
    );

    // ...but the forced hash on a later tick must still notice it.
    assert!(
        matches!(
            chaus_io::check_conflict(&meta),
            Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk))
        ),
        "content hashing must not rely on size or mtime"
    );
}

#[test]
fn a_missing_file_never_matches() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gone.txt");
    std::fs::write(&path, "one\n").unwrap();
    let meta = loaded(&path);

    std::fs::remove_file(&path).unwrap();

    assert!(!chaus_io::metadata_matches(&meta));
    assert!(matches!(
        chaus_io::check_conflict(&meta),
        Err(ConflictCheckError::Conflict(SaveConflict::DeletedOnDisk))
    ));
}

#[test]
fn a_path_replaced_by_a_directory_reports_io_not_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("replaced.txt");
    std::fs::write(&path, "one\n").unwrap();
    let meta = loaded(&path);

    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();

    assert!(!chaus_io::metadata_matches(&meta));
    assert!(
        matches!(
            chaus_io::check_conflict(&meta),
            Err(ConflictCheckError::Io { .. })
        ),
        "an unreadable target must not be reported as unchanged"
    );
}

#[test]
fn a_replaced_file_with_different_content_is_a_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("swapped.txt");
    std::fs::write(&path, "one\n").unwrap();
    let meta = loaded(&path);

    let replacement = directory.path().join("replacement.txt");
    std::fs::write(&replacement, "two\n").unwrap();
    std::fs::rename(&replacement, &path).unwrap();

    assert!(matches!(
        chaus_io::check_conflict(&meta),
        Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk))
    ));
}
