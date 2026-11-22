//! CLI parsing without a graphical environment.

use std::ffi::OsString;
use std::path::PathBuf;

use chaus_cli::{parse, CliError, Command};

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn no_arguments_opens_one_empty_window() {
    assert_eq!(
        parse(args(&[])).expect("parse"),
        Command::Run {
            paths: vec![],
            new_window: false
        }
    );
}

#[test]
fn paths_keep_spaces_and_relative_form() {
    let command = parse(args(&["notes with spaces.txt", "../other.md"])).expect("parse");
    assert_eq!(
        command,
        Command::Run {
            paths: vec![
                PathBuf::from("notes with spaces.txt"),
                PathBuf::from("../other.md")
            ],
            new_window: false
        }
    );
}

#[test]
fn help_and_version_short_circuit() {
    assert_eq!(parse(args(&["--help"])).expect("parse"), Command::Help);
    assert_eq!(parse(args(&["-h"])).expect("parse"), Command::Help);
    assert_eq!(
        parse(args(&["file.txt", "--version"])).expect("parse"),
        Command::Version
    );
    assert_eq!(parse(args(&["-V"])).expect("parse"), Command::Version);
}

#[test]
fn double_dash_ends_options() {
    let command = parse(args(&["--", "--help", "-n"])).expect("parse");
    assert_eq!(
        command,
        Command::Run {
            paths: vec![PathBuf::from("--help"), PathBuf::from("-n")],
            new_window: false
        }
    );
}

#[test]
fn unknown_option_is_an_error() {
    let error = parse(args(&["--bogus"])).expect_err("must reject");
    assert_eq!(error, CliError::UnknownOption(OsString::from("--bogus")));
}

#[test]
fn new_flag_is_recorded_with_paths() {
    let command = parse(args(&["--new", "a.txt"])).expect("parse");
    assert_eq!(
        command,
        Command::Run {
            paths: vec![PathBuf::from("a.txt")],
            new_window: true
        }
    );
}
