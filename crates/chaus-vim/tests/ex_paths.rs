//! Home-relative Ex paths and the `:o`/`:open` alias.
//!
//! These tests live in `chaus-vim` because the parser records the spelling
//! without touching the environment; the editor resolves `~` with
//! `chaus_io::expand_home` when it actually opens or writes a file.

use chaus_vim::ex_command::{parse, path_argument, quote_path, ExParseError, ExPath};
use chaus_vim::ExCommand;

fn home(text: &str) -> ExPath {
    ExPath::unquoted(text)
}

fn literal(text: &str) -> ExPath {
    ExPath::literal(text)
}

#[test]
fn unquoted_home_paths_stay_home_relative_for_the_editor() {
    for input in [
        "e ~/notes.md",
        "edit ~/notes.md",
        "o ~/notes.md",
        "open ~/notes.md",
        "w ~/notes.md",
        "saveas ~/notes.md",
        "r ~/notes.md",
    ] {
        let command = parse(input).unwrap_or_else(|error| panic!("{input:?}: {error}"));
        let expected = if input.starts_with('w') {
            ExCommand::Write {
                force: false,
                path: Some(home("~/notes.md")),
            }
        } else if input.starts_with("saveas") {
            ExCommand::SaveAs {
                force: false,
                path: home("~/notes.md"),
            }
        } else if input.starts_with('r') {
            ExCommand::Read {
                path: home("~/notes.md"),
            }
        } else {
            ExCommand::Edit {
                force: false,
                path: Some(home("~/notes.md")),
            }
        };
        assert_eq!(command, expected, "input {input:?}");
    }
}

#[test]
fn bare_tilde_is_a_home_path() {
    assert_eq!(
        parse("e ~"),
        Ok(ExCommand::Edit {
            force: false,
            path: Some(home("~")),
        })
    );
    match parse("e ~").unwrap() {
        ExCommand::Edit {
            path: Some(path), ..
        } => assert!(path.expands_home()),
        other => panic!("unexpected command {other:?}"),
    }
}

#[test]
fn quoted_tilde_is_a_literal_file_name() {
    assert_eq!(
        parse("e \"~/notes.md\""),
        Ok(ExCommand::Edit {
            force: false,
            path: Some(literal("~/notes.md")),
        })
    );
    assert!(!literal("~/notes.md").expands_home());
    assert_eq!(
        parse("w \"~\\notes.md\""),
        Ok(ExCommand::Write {
            force: false,
            path: Some(literal("~\\notes.md")),
        })
    );
}

#[test]
fn home_paths_keep_spaces_unicode_and_windows_separators() {
    assert_eq!(
        parse(r"e ~/my\ notes.md"),
        Ok(ExCommand::Edit {
            force: false,
            path: Some(home("~/my notes.md")),
        })
    );
    assert_eq!(
        parse("w ~/ノート/é.md"),
        Ok(ExCommand::Write {
            force: false,
            path: Some(home("~/ノート/é.md")),
        })
    );
    assert_eq!(
        parse("e \"~\" other"),
        Err(ExParseError::UnexpectedArgument("edit"))
    );
    let literal = parse("e \"~\"").unwrap();
    assert_eq!(
        literal,
        ExCommand::Edit {
            force: false,
            path: Some(ExPath::literal("~")),
        }
    );
}

#[test]
fn windows_separator_is_accepted_only_where_the_platform_uses_it() {
    let parsed = parse("e ~\\notes.md");
    if cfg!(windows) {
        assert_eq!(
            parsed,
            Ok(ExCommand::Edit {
                force: false,
                path: Some(home("~\\notes.md")),
            })
        );
    } else {
        assert_eq!(parsed, Err(ExParseError::UnsupportedExpansion('~')));
    }
}

#[test]
fn other_users_and_expansions_remain_rejected() {
    for input in ["e ~other/notes", "o ~other/notes", "w ~other/x"] {
        assert_eq!(
            parse(input),
            Err(ExParseError::UnsupportedExpansion('~')),
            "input {input:?}"
        );
    }
}

#[test]
fn open_is_a_chaus_alias_for_edit() {
    let cases = [
        ("o notes.md", false, Some("notes.md")),
        ("open notes.md", false, Some("notes.md")),
        ("o! notes.md", true, Some("notes.md")),
        ("open! notes.md", true, Some("notes.md")),
    ];
    for (input, force, path) in cases {
        assert_eq!(
            parse(input),
            Ok(ExCommand::Edit {
                force,
                path: path.map(home),
            }),
            "input {input:?}"
        );
    }
    assert_eq!(
        parse("o!"),
        Ok(ExCommand::Edit {
            force: true,
            path: None,
        }),
        ":o! with no path reloads like :e!"
    );
    assert_eq!(parse("o"), Err(ExParseError::MissingPath("open")));
    assert_eq!(
        parse("open a b"),
        Err(ExParseError::UnexpectedArgument("open"))
    );
}

#[test]
fn completion_sees_the_path_argument_of_open() {
    let argument = path_argument("o src/ma").expect(":o takes a path");
    assert_eq!(argument.typed, "src/ma");
    assert!(!argument.quoted);
    let argument = path_argument("open ~/no").expect(":open takes a home path");
    assert_eq!(argument.typed, "~/no");
    assert!(!argument.quoted);
    let argument = path_argument("o \"~/no").expect("unterminated quote still completes");
    assert_eq!(argument.typed, "~/no");
    assert!(argument.quoted);
}

#[test]
fn home_paths_round_trip_through_quote_path() {
    for (raw, expected) in [
        ("~/notes.md", "~/notes.md"),
        ("~/my notes.md", "~/my\\ notes.md"),
        ("~", "~"),
    ] {
        let spelled = quote_path(raw).expect("home paths are spellable");
        assert_eq!(spelled, expected);
        let command = parse(&format!("w {spelled}")).expect("spelled path parses");
        match command {
            ExCommand::Write {
                path: Some(path), ..
            } => {
                assert_eq!(path.as_path().to_str(), Some(raw));
                assert!(path.expands_home());
            }
            other => panic!("unexpected command {other:?}"),
        }
    }
    assert_eq!(
        quote_path("~tilde"),
        Some("\"~tilde\"".to_owned()),
        "a non-home tilde stays literal and needs quoting"
    );
}
