//! A zero-exit formatter is only trusted when all input reached it.

use chaus_io::format::{run, FormatError, Formatter};

fn formatter(program: &str, arguments: &[&str]) -> Formatter {
    Formatter::new(
        program,
        arguments.iter().map(|arg| (*arg).to_owned()).collect(),
    )
}

#[cfg(windows)]
fn short_reader() -> Formatter {
    formatter(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "$b = New-Object char[] 4; [Console]::In.Read($b, 0, 4) | Out-Null; [Console]::Out.Write((New-Object string $b)); exit 0",
        ],
    )
}

#[cfg(not(windows))]
fn short_reader() -> Formatter {
    formatter("sh", &["-c", "head -c 4; exit 0"])
}

#[cfg(windows)]
fn short_reader_failure() -> Formatter {
    formatter(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "$b = New-Object char[] 4; [Console]::In.Read($b, 0, 4) | Out-Null; exit 3",
        ],
    )
}

#[cfg(not(windows))]
fn short_reader_failure() -> Formatter {
    formatter("sh", &["-c", "head -c 4; exit 3"])
}

#[test]
fn zero_exit_after_a_short_read_is_rejected() {
    let source = "ab".repeat(512 * 1024);
    let error = run(&short_reader(), &source).unwrap_err();
    match error {
        FormatError::InputIncomplete(_) => {}
        other => panic!("a short-read formatter must not supply replacement text: {other}"),
    }
}

#[test]
fn a_failed_child_keeps_its_own_diagnostic() {
    let source = "ab".repeat(512 * 1024);
    let error = run(&short_reader_failure(), &source).unwrap_err();
    match error {
        FormatError::Failed { status, .. } => assert_eq!(status, Some(3)),
        other => panic!("child failure takes precedence over a broken pipe: {other}"),
    }
}
