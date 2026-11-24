//! Bounded local diagnostics with redaction and a panic record.

use std::fs;
use std::path::PathBuf;

use chaus_app::diagnostics;

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read(path: &PathBuf) -> String {
    fs::read_to_string(path).expect("diagnostic log is readable")
}

#[test]
fn diagnostics_are_structured_bounded_and_redacted() {
    let _test = TEST_LOCK.lock().expect("diagnostics test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let previous_dir = std::env::var_os("CHAUS_DIAGNOSTICS_DIR");
    let previous_limit = std::env::var_os("CHAUS_DIAGNOSTICS_MAX_BYTES");
    std::env::set_var("CHAUS_DIAGNOSTICS_DIR", directory.path());
    std::env::set_var("CHAUS_DIAGNOSTICS_MAX_BYTES", "2048");

    diagnostics::init();
    let path = directory.path().join("chaus.log");
    let startup = read(&path);
    assert!(
        startup.contains(&format!("build={}", diagnostics::BUILD_SHA)),
        "the build identity must be in the first record: {startup}"
    );
    assert!(startup.contains("phase=startup"));
    assert!(startup.contains("seq=0"));

    diagnostics::event("save", 7, "queued");
    let after_event = read(&path);
    assert!(after_event.contains("correlation=7"));
    assert!(after_event.contains("phase=save"));
    assert!(after_event.contains("seq=1"));

    let worker = std::thread::spawn(|| panic!("failed C:/Users/terezi/notes.txt"));
    let _ = worker.join();
    let after_panic = read(&path);
    assert!(after_panic.contains("phase=panic"));
    assert!(
        !after_panic.contains("notes.txt"),
        "a panic message must not carry a file path"
    );
    assert!(after_panic.contains("<path>"));

    for index in 0..24 {
        diagnostics::event("resize", index, "C:/Users/terezi/private.txt");
    }
    diagnostics::shutdown();
    let latest = read(&path);
    assert!(latest.contains("phase=shutdown"));
    assert!(latest.contains("<path>"));
    assert!(!latest.contains("private.txt"));
    assert!(
        directory.path().join("chaus.1.log").exists(),
        "an oversized log must rotate to one backup"
    );
    let files: Vec<_> = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect();
    assert!(files.len() <= 2, "diagnostics must stay bounded: {files:?}");
    assert!(
        fs::metadata(&path).unwrap().len() < 4096,
        "the active log must stay near the configured limit"
    );

    diagnostics::set_sink_for_test(None);
    match previous_dir {
        Some(value) => std::env::set_var("CHAUS_DIAGNOSTICS_DIR", value),
        None => std::env::remove_var("CHAUS_DIAGNOSTICS_DIR"),
    }
    match previous_limit {
        Some(value) => std::env::set_var("CHAUS_DIAGNOSTICS_MAX_BYTES", value),
        None => std::env::remove_var("CHAUS_DIAGNOSTICS_MAX_BYTES"),
    }
}
