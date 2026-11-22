//! CLI launch failures must be visible and nonzero.

use std::process::Command;

fn chaus() -> Command {
    Command::new(env!("CARGO_BIN_EXE_chaus"))
}

#[test]
fn a_missing_path_fails_without_opening_a_window() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("does not exist.txt");
    let output = chaus().arg(&missing).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&missing.display().to_string()),
        "stderr must name the attempted path: {stderr:?}"
    );
    assert!(
        stderr.contains("file not found"),
        "stderr must explain the failure: {stderr:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "a failed launch must not print a window-ish success message"
    );
}

#[test]
fn a_directory_argument_is_rejected_as_a_file() {
    let directory = tempfile::tempdir().unwrap();
    let output = chaus().arg(directory.path()).output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a regular file"),
        "stderr must explain the directory rejection: {stderr:?}"
    );
}

#[test]
fn help_and_version_stay_gpu_independent_and_successful() {
    let help = chaus().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("USAGE:"));

    let version = chaus().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("chaus "));
}
