//! External formatter detection and bounded process execution.

use std::path::Path;
use std::time::Duration;

use chaus_io::format::{run, run_with_timeout, FormatError, Formatter};

fn formatter(program: &str, arguments: &[&str]) -> Formatter {
    Formatter::new(
        program,
        arguments.iter().map(|arg| (*arg).to_owned()).collect(),
    )
}

#[cfg(windows)]
fn pass_through() -> Formatter {
    formatter(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "[Console]::Out.Write([Console]::In.ReadToEnd())",
        ],
    )
}

#[cfg(not(windows))]
fn pass_through() -> Formatter {
    formatter("cat", &[])
}

#[cfg(windows)]
fn slow() -> Formatter {
    formatter(
        "powershell",
        &["-NoProfile", "-Command", "Start-Sleep -Seconds 30"],
    )
}

#[cfg(not(windows))]
fn slow() -> Formatter {
    formatter("sleep", &["30"])
}

#[cfg(windows)]
fn failing() -> Formatter {
    formatter("cmd", &["/C", "exit", "3"])
}

#[cfg(not(windows))]
fn failing() -> Formatter {
    formatter("sh", &["-c", "exit 3"])
}

#[test]
fn formatters_are_detected_from_the_file_extension() {
    assert_eq!(
        Formatter::for_path(Path::new("src/main.rs")).map(|f| f.program),
        Some("rustfmt".to_owned())
    );
    assert_eq!(
        Formatter::for_path(Path::new("main.cpp")).map(|f| f.program),
        Some("clang-format".to_owned())
    );
    assert_eq!(
        Formatter::for_path(Path::new("tool.py")).map(|f| f.program),
        Some("black".to_owned())
    );
    assert_eq!(
        Formatter::for_path(Path::new("app.tsx")).map(|f| f.program),
        Some("prettier".to_owned())
    );
    assert_eq!(Formatter::for_path(Path::new("notes.txt")), None);
    assert_eq!(Formatter::for_path(Path::new("README")), None);
}

#[test]
fn a_missing_formatter_is_reported_not_silently_ignored() {
    let error = run(&formatter("chaus-no-such-formatter", &[]), "text").unwrap_err();
    assert!(
        matches!(error, FormatError::Unavailable(_)),
        "a missing executable must surface as unavailable: {error}"
    );
}

#[test]
fn formatter_stdout_becomes_the_replacement_without_shell_interpolation() {
    let output = run(&pass_through(), "one\ntwo\n").unwrap();
    assert_eq!(output, "one\ntwo\n");
}

#[test]
fn a_slow_formatter_is_killed_at_the_deadline() {
    let error = run_with_timeout(&slow(), "text", Duration::from_millis(150)).unwrap_err();
    assert!(
        matches!(error, FormatError::Timeout),
        "a formatter past its deadline must be stopped: {error}"
    );
}

#[test]
fn a_nonzero_exit_keeps_the_document_unchanged_and_reports_status() {
    let error = run(&failing(), "text").unwrap_err();
    match error {
        FormatError::Failed { status, .. } => assert_eq!(status, Some(3)),
        other => panic!("expected a failed formatter, got {other}"),
    }
}
