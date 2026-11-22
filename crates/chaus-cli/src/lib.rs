//! Command-line parsing.
//!
//! Parsed before GPUI, fonts or the GPU are initialized so `--help` and
//! `--version` work in any terminal. Paths stay `OsString`/`PathBuf` all the
//! way through.

use std::ffi::OsString;
use std::path::PathBuf;

/// What the process should do after parsing.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open one window per path, plus an untitled window when requested.
    Run {
        paths: Vec<PathBuf>,
        new_window: bool,
    },
    Help,
    Version,
}

/// A malformed invocation. Exit status is 2 for usage errors.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliError {
    UnknownOption(OsString),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::UnknownOption(option) => {
                write!(formatter, "unknown option: {}", option.to_string_lossy())
            }
        }
    }
}

impl std::error::Error for CliError {}

/// Parse program arguments, excluding `argv[0]`.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let mut paths = Vec::new();
    let mut new_window = false;
    let mut options_ended = false;
    for argument in args {
        if !options_ended {
            match argument.to_str() {
                Some("--") => {
                    options_ended = true;
                    continue;
                }
                Some("--help" | "-h") => return Ok(Command::Help),
                Some("--version" | "-V") => return Ok(Command::Version),
                Some("--new") => {
                    new_window = true;
                    continue;
                }
                Some(text) if text.starts_with('-') && text != "-" => {
                    return Err(CliError::UnknownOption(argument));
                }
                _ => {}
            }
        }
        paths.push(PathBuf::from(argument));
    }
    Ok(Command::Run { paths, new_window })
}

/// Usage text printed for `--help`.
#[must_use]
pub fn help_text() -> String {
    format!(
        "\
chaus {version} — a small native plaintext editor

USAGE:
    chaus [OPTIONS] [PATH ...]

OPTIONS:
    --new        Also open an empty untitled window
    -h, --help   Print this help and exit without launching the GUI
    -V, --version  Print the version and exit
    --           Treat every later argument as a path

Each PATH opens in its own window. With no PATH, one untitled window opens.
Files are edited as UTF-8; UTF-8 BOM and LF/CRLF/CR line endings are preserved.
",
        version = env!("CARGO_PKG_VERSION")
    )
}
