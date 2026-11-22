//! Atomic replacement used by settings persistence.

#[test]
fn creates_a_new_file_and_replaces_existing_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested").join("settings.rc");
    chaus_io::write_atomic(&path, b"first\n").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first\n");

    chaus_io::write_atomic(&path, b"second\n").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second\n");
}

#[test]
fn a_failed_replacement_leaves_the_previous_target_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("settings.rc");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("marker"), b"still here").unwrap();

    assert!(chaus_io::write_atomic(&target, b"new").is_err());
    assert!(target.is_dir());
    assert_eq!(
        std::fs::read(target.join("marker")).unwrap(),
        b"still here",
        "the old target must survive a failed replacement"
    );
}

#[test]
fn no_temporary_files_are_left_behind() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.rc");
    chaus_io::write_atomic(&path, b"content").unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".chaus-write-")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
