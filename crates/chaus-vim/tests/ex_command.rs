//! Typed Ex grammar: accepted forms, rejected forms and the path rules.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use chaus_core::settings::WrapMode;
use chaus_vim::ex_command::{
    parse, path_argument, quote_path, ExParseError, LineTarget, SetArgument, SetOption,
};
use chaus_vim::ExCommand;

fn path(text: &str) -> PathBuf {
    PathBuf::from(text)
}

fn line(number: usize) -> ExCommand {
    ExCommand::GotoLine(LineTarget::Number(NonZeroUsize::new(number).unwrap()))
}

#[test]
fn accepted_forms_produce_typed_intents() {
    let cases: Vec<(&str, ExCommand)> = vec![
        (
            "w",
            ExCommand::Write {
                force: false,
                path: None,
            },
        ),
        (
            "write",
            ExCommand::Write {
                force: false,
                path: None,
            },
        ),
        (
            "w!",
            ExCommand::Write {
                force: true,
                path: None,
            },
        ),
        (
            "w other.txt",
            ExCommand::Write {
                force: false,
                path: Some(path("other.txt")),
            },
        ),
        (
            "w! \"a b.txt\"",
            ExCommand::Write {
                force: true,
                path: Some(path("a b.txt")),
            },
        ),
        (
            "w a\\ b.txt",
            ExCommand::Write {
                force: false,
                path: Some(path("a b.txt")),
            },
        ),
        (
            "saveas \"a b.txt\"",
            ExCommand::SaveAs {
                force: false,
                path: path("a b.txt"),
            },
        ),
        (
            "sav! x",
            ExCommand::SaveAs {
                force: true,
                path: path("x"),
            },
        ),
        ("q", ExCommand::Quit { force: false }),
        ("quit!", ExCommand::Quit { force: true }),
        ("wq", ExCommand::WriteQuit),
        ("x", ExCommand::Exit),
        ("xit", ExCommand::Exit),
        (
            "e! notes.md",
            ExCommand::Edit {
                force: true,
                path: Some(path("notes.md")),
            },
        ),
        (
            "e notes.md",
            ExCommand::Edit {
                force: false,
                path: Some(path("notes.md")),
            },
        ),
        (
            "e!",
            ExCommand::Edit {
                force: true,
                path: None,
            },
        ),
        ("enew", ExCommand::New { force: false }),
        ("ene!", ExCommand::New { force: true }),
        ("f", ExCommand::File),
        ("file", ExCommand::File),
        ("pwd", ExCommand::WorkingDirectory),
        ("noh", ExCommand::NoHighlight),
        ("nohlsearch", ExCommand::NoHighlight),
        ("123", line(123)),
        ("0", line(1)),
        ("$", ExCommand::GotoLine(LineTarget::Last)),
        (
            "99999999999999999999999999",
            ExCommand::GotoLine(LineTarget::Last),
        ),
        (
            "set wrap=word",
            ExCommand::Set(vec![SetArgument::Wrap(WrapMode::Word)]),
        ),
        (
            "set nowrap et ts=8 wrap?",
            ExCommand::Set(vec![
                SetArgument::Wrap(WrapMode::None),
                SetArgument::ExpandTab(true),
                SetArgument::TabStop(8),
                SetArgument::Query(SetOption::Wrap),
            ]),
        ),
        ("format", ExCommand::Format),
        ("trim", ExCommand::Trim),
        ("retab", ExCommand::Retab),
        ("wa", ExCommand::WriteAll),
        ("qa", ExCommand::QuitAll { force: false }),
        ("qall!", ExCommand::QuitAll { force: true }),
        (
            "r in.txt",
            ExCommand::Read {
                path: path("in.txt"),
            },
        ),
        (
            ":  w  ",
            ExCommand::Write {
                force: false,
                path: None,
            },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(parse(input), Ok(expected), "input {input:?}");
    }
}

#[test]
fn rejected_forms_name_the_problem() {
    let cases: Vec<(&str, ExParseError)> = vec![
        ("", ExParseError::Empty),
        (
            "frobnicate",
            ExParseError::UnknownCommand("frobnicate".to_owned()),
        ),
        ("s/a/b/", ExParseError::UnknownCommand("s".to_owned())),
        ("!ls", ExParseError::UnknownCommand("!ls".to_owned())),
        (
            "source x",
            ExParseError::UnknownCommand("source".to_owned()),
        ),
        ("wq!", ExParseError::UnexpectedForce("wq")),
        ("x!", ExParseError::UnexpectedForce("xit")),
        ("q now", ExParseError::UnexpectedArgument("quit")),
        ("w a b", ExParseError::UnexpectedArgument("write")),
        ("w \"a b\" c", ExParseError::UnexpectedArgument("write")),
        ("saveas", ExParseError::MissingPath("saveas")),
        ("e", ExParseError::MissingPath("edit")),
        ("r", ExParseError::MissingPath("read")),
        ("w \"", ExParseError::UnterminatedQuote),
        ("w \"\"", ExParseError::MissingPath("write")),
        ("w %", ExParseError::UnsupportedExpansion('%')),
        ("e #", ExParseError::UnsupportedExpansion('#')),
        ("e ~/notes", ExParseError::UnsupportedExpansion('~')),
        ("e *.txt", ExParseError::UnsupportedExpansion('*')),
        ("wq1", ExParseError::UnknownCommand("wq1".to_owned())),
        ("set", ExParseError::MissingSetArgument),
        ("set bogus", ExParseError::UnknownOption("bogus".to_owned())),
        (
            "set bogus?",
            ExParseError::UnknownOption("bogus".to_owned()),
        ),
        (
            "set ts=99",
            ExParseError::InvalidValue {
                option: "tabstop",
                value: "99".to_owned(),
            },
        ),
        (
            "set wrap=sideways",
            ExParseError::InvalidValue {
                option: "wrap",
                value: "sideways".to_owned(),
            },
        ),
        ("pwd x", ExParseError::UnexpectedArgument("pwd")),
        ("noh!", ExParseError::UnexpectedForce("nohlsearch")),
    ];
    for (input, expected) in cases {
        assert_eq!(parse(input), Err(expected), "input {input:?}");
    }
}

#[test]
fn backslashes_are_literal_except_before_a_space() {
    assert_eq!(
        parse(r"w C:\Users\me\notes.txt"),
        Ok(ExCommand::Write {
            force: false,
            path: Some(path(r"C:\Users\me\notes.txt"))
        })
    );
    assert_eq!(
        parse(r"e \\server\share\a.txt"),
        Ok(ExCommand::Edit {
            force: false,
            path: Some(path(r"\\server\share\a.txt"))
        })
    );
    assert_eq!(
        parse(r#"w "C:\Program Files\x.txt""#),
        Ok(ExCommand::Write {
            force: false,
            path: Some(path(r"C:\Program Files\x.txt"))
        })
    );
    assert_eq!(
        parse(r"w my\ notes\ v2.txt"),
        Ok(ExCommand::Write {
            force: false,
            path: Some(path("my notes v2.txt"))
        })
    );
}

#[test]
fn unicode_and_quoted_wildcards_are_kept_literally() {
    assert_eq!(
        parse("w ノート/é.txt"),
        Ok(ExCommand::Write {
            force: false,
            path: Some(path("ノート/é.txt"))
        })
    );
    assert_eq!(
        parse("w \"what?.txt\""),
        Ok(ExCommand::Write {
            force: false,
            path: Some(path("what?.txt"))
        })
    );
}

#[test]
fn quote_path_round_trips_through_the_parser() {
    for raw in [
        "plain.txt",
        "with space.txt",
        "~tilde",
        "star*.txt",
        r"C:\a b\c",
    ] {
        let spelled = quote_path(raw).unwrap();
        assert_eq!(
            parse(&format!("w {spelled}")),
            Ok(ExCommand::Write {
                force: false,
                path: Some(path(raw))
            }),
            "{raw:?} spelled as {spelled:?}"
        );
    }
    assert_eq!(quote_path("has\"quote"), None);
}

#[test]
fn path_argument_is_found_only_for_path_commands() {
    let argument = path_argument("e src/ma").unwrap();
    assert_eq!(argument.typed, "src/ma");
    assert!(!argument.quoted);
    let argument = path_argument("saveas \"my do").unwrap();
    assert_eq!(argument.typed, "my do");
    assert!(argument.quoted);
    assert_eq!(path_argument("w a\\ b").unwrap().typed, "a b");
    assert_eq!(path_argument("set wr"), None);
    assert_eq!(path_argument("q x"), None);
    assert_eq!(
        path_argument("w \"done\""),
        None,
        "a closed quote is complete"
    );
    assert_eq!(path_argument("w"), None, "no argument started yet");
}
