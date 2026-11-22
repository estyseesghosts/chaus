//! Explicit, opt-in external formatters.
//!
//! Chaus never installs formatters, never runs them through a shell and never
//! formats on save. A formatter receives the document on stdin and returns the
//! replacement text on stdout with bounded time and output. The caller applies
//! the result through the document transaction API, so a format is one undo
//! step and is discarded if the document changed while it ran.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Longest a formatter may run before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest stdout or stderr accepted from a formatter.
pub const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

/// One externally installed formatter invocation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatter {
    pub program: String,
    pub arguments: Vec<String>,
}

impl Formatter {
    /// Build an explicit formatter invocation.
    #[must_use]
    pub fn new(program: impl Into<String>, arguments: Vec<String>) -> Self {
        Self {
            program: program.into(),
            arguments,
        }
    }

    /// The formatter expected for a path, or `None` when the language has no
    /// known tool. Detection is by extension only; availability is checked when
    /// the command is spawned and reported explicitly.
    #[must_use]
    pub fn for_path(path: &Path) -> Option<Self> {
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase());
        let path_text = path.to_string_lossy().into_owned();
        let formatter = match extension.as_deref()? {
            "rs" => Self {
                program: "rustfmt".to_owned(),
                arguments: vec![
                    "--emit".to_owned(),
                    "stdout".to_owned(),
                    "--edition".to_owned(),
                    "2021".to_owned(),
                ],
            },
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" => Self {
                program: "clang-format".to_owned(),
                arguments: vec!["-assume-filename".to_owned(), path_text],
            },
            "py" | "pyi" => Self {
                program: "black".to_owned(),
                arguments: vec!["-".to_owned(), "--quiet".to_owned()],
            },
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "json" | "css" | "scss" | "less"
            | "html" | "md" | "yaml" | "yml" => Self {
                program: "prettier".to_owned(),
                arguments: vec!["--stdin-filepath".to_owned(), path_text],
            },
            _ => {
                return None;
            }
        };
        Some(formatter)
    }
}

/// Why a formatter did not produce usable text.
#[non_exhaustive]
#[derive(Debug)]
pub enum FormatError {
    /// The executable could not be started (usually not installed).
    Unavailable(std::io::Error),
    Timeout,
    OutputTooLarge,
    InvalidUtf8,
    Failed {
        status: Option<i32>,
        stderr: String,
    },
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(error) => {
                write!(
                    formatter,
                    "formatter is not installed or could not start: {error}"
                )
            }
            Self::Timeout => write!(formatter, "formatter timed out and was stopped"),
            Self::OutputTooLarge => write!(
                formatter,
                "formatter returned more than {} MiB",
                OUTPUT_LIMIT / (1024 * 1024)
            ),
            Self::InvalidUtf8 => write!(formatter, "formatter returned invalid UTF-8"),
            Self::Failed { status, stderr } => {
                let status = status.map_or_else(|| "unknown".to_owned(), |code| code.to_string());
                if stderr.trim().is_empty() {
                    write!(formatter, "formatter exited with status {status}")
                } else {
                    write!(formatter, "formatter exited with status {status}: {stderr}")
                }
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Run `formatter` over `source`, returning its stdout as the replacement text.
pub fn run(formatter: &Formatter, source: &str) -> Result<String, FormatError> {
    run_with_timeout(formatter, source, TIMEOUT)
}

/// Same as [`run`] with an explicit deadline; used by tests and future
/// per-language budgets.
pub fn run_with_timeout(
    formatter: &Formatter,
    source: &str,
    timeout: Duration,
) -> Result<String, FormatError> {
    let mut child = Command::new(&formatter.program)
        .args(&formatter.arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(FormatError::Unavailable)?;

    let mut stdin = child.stdin.take().expect("stdin was requested as piped");
    let source = source.to_owned();
    let writer = std::thread::spawn(move || {
        // A formatter may exit before reading everything; a broken pipe is not
        // an application error.
        let _ = stdin.write_all(source.as_bytes());
    });
    let stdout = child.stdout.take().expect("stdout was requested as piped");
    let stderr = child.stderr.take().expect("stderr was requested as piped");
    let stdout = std::thread::spawn(move || read_bounded(stdout));
    let stderr = std::thread::spawn(move || read_bounded(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = writer.join();
                    let _ = stdout.join();
                    let _ = stderr.join();
                    return Err(FormatError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = stdout.join();
                let _ = stderr.join();
                return Err(FormatError::Unavailable(error));
            }
        }
    };
    let _ = writer.join();
    let stdout = join_bytes(stdout)?;
    let stderr = String::from_utf8_lossy(&join_bytes(stderr)?).into_owned();

    if !status.success() {
        return Err(FormatError::Failed {
            status: status.code(),
            stderr,
        });
    }
    let text = String::from_utf8(stdout).map_err(|_| FormatError::InvalidUtf8)?;
    Ok(text)
}

fn join_bytes(
    handle: std::thread::JoinHandle<Result<Vec<u8>, FormatError>>,
) -> Result<Vec<u8>, FormatError> {
    handle
        .join()
        .map_err(|_| FormatError::Unavailable(std::io::Error::other("formatter reader panicked")))?
}

fn read_bounded(mut pipe: impl Read) -> Result<Vec<u8>, FormatError> {
    let mut collected = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = pipe.read(&mut buffer).map_err(FormatError::Unavailable)?;
        if read == 0 {
            return Ok(collected);
        }
        if collected.len() + read > OUTPUT_LIMIT {
            return Err(FormatError::OutputTooLarge);
        }
        collected.extend_from_slice(&buffer[..read]);
    }
}
