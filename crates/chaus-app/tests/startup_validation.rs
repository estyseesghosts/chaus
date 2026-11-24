//! Startup preflight is cheap and reports invalid paths truthfully.

use chaus_app::app::validate_startup_paths;

#[test]
fn missing_paths_and_directories_are_rejected_before_the_gui_starts() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "one\n").unwrap();
    std::fs::write(&second, "two\n").unwrap();
    let missing = directory.path().join("missing.txt");

    let (valid, failures) = validate_startup_paths(vec![
        first.clone(),
        missing.clone(),
        directory.path().to_path_buf(),
        second.clone(),
    ]);

    assert_eq!(valid, vec![first, second]);
    assert_eq!(failures, 2);
}

#[test]
fn an_empty_path_list_has_no_failures() {
    assert_eq!(validate_startup_paths(Vec::new()), (Vec::new(), 0));
}
