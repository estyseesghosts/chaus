//! An oversized stdout kills the child instead of waiting for the deadline.

use std::time::{Duration, Instant};

use chaus_io::format::{run_with_timeout, FormatError, Formatter};

#[cfg(windows)]
fn noisy_formatter() -> Formatter {
    Formatter::new(
        "powershell",
        vec![
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            "[Console]::Out.Write('x' * (9 * 1024 * 1024)); Start-Sleep -Seconds 30".to_owned(),
        ],
    )
}

#[cfg(not(windows))]
fn noisy_formatter() -> Formatter {
    Formatter::new(
        "sh",
        vec![
            "-c".to_owned(),
            // exec makes the child itself the long-lived process; otherwise a
            // shell may leave `sleep` as a grandchild holding the pipes open.
            "yes x | head -c 9437184; exec sleep 30".to_owned(),
        ],
    )
}

#[test]
fn output_over_the_cap_is_reported_before_the_timeout() {
    let started = Instant::now();
    let error = run_with_timeout(&noisy_formatter(), "input", Duration::from_secs(10))
        .expect_err("oversized output must be rejected");
    let elapsed = started.elapsed();
    assert!(
        matches!(error, FormatError::OutputTooLarge),
        "expected the output cap, got {error}"
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "the child must be killed promptly, not left until the deadline: {elapsed:?}"
    );
}
