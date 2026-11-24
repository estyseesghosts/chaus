//! Typed Ex commands.
//!
//! The command line is parsed into one [`ExCommand`] before anything runs.
//! Unknown names, stray arguments and unsupported `!` forms are rejected here,
//! so a malformed line can never reach a save, close or reload. The parser has
//! no filesystem or GUI access; the editor maps each intent to its existing
//! file and window operations.

use std::fmt;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use chaus_core::settings::WrapMode;

/// Largest accepted `tabstop`, matching the rc-file validator.
const MAX_TAB_STOP: usize = 16;

/// One validated command-line intent.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExCommand {
    /// `:w[!] [path]`. Without a path this saves the current document. With a
    /// path on a named document it writes a copy and keeps the association;
    /// on an untitled document the path becomes its file once the write lands.
    Write {
        force: bool,
        path: Option<PathBuf>,
    },
    /// `:sav[eas][!] path`: write, then associate the document with `path`.
    SaveAs {
        force: bool,
        path: PathBuf,
    },
    /// `:q[uit][!]`.
    Quit {
        force: bool,
    },
    /// `:wq`.
    WriteQuit,
    /// `:x[it]`: close a clean document, otherwise save then close.
    Exit,
    /// `:e[dit] path` or `:e[dit]! [path]`. `path` is `None` only for `:e!`,
    /// which reloads the current file.
    Edit {
        force: bool,
        path: Option<PathBuf>,
    },
    /// `:ene[w][!]`: replace this window's document with an untitled one.
    New {
        force: bool,
    },
    /// `:f[ile]`.
    File,
    /// `:pw[d]`.
    WorkingDirectory,
    /// `:noh[lsearch]`.
    NoHighlight,
    /// `:N` or `:$`.
    GotoLine(LineTarget),
    /// `:se[t] ...`.
    Set(Vec<SetArgument>),
    /// `:format`, `:trim`, `:retab`.
    Format,
    Trim,
    Retab,
    /// `:wa[ll]`.
    WriteAll,
    /// `:qa[ll][!]`.
    QuitAll {
        force: bool,
    },
    /// `:r[ead] path`.
    Read {
        path: PathBuf,
    },
}

/// Logical (hard) line addressed by `:N` or `:$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineTarget {
    /// One-based line. `:0` is accepted as line 1, as in Vim.
    Number(NonZeroUsize),
    /// `:$`, also used when a number is larger than `usize`.
    Last,
}

/// Window-local options reachable through `:set`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOption {
    Wrap,
    ExpandTab,
    TabStop,
}

impl SetOption {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "wrap" => Some(Self::Wrap),
            "expandtab" | "et" => Some(Self::ExpandTab),
            "tabstop" | "ts" => Some(Self::TabStop),
            _ => None,
        }
    }

    /// Name used when reporting a value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Wrap => "wrap",
            Self::ExpandTab => "expandtab",
            Self::TabStop => "tabstop",
        }
    }
}

/// One validated `:set` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetArgument {
    Wrap(WrapMode),
    ExpandTab(bool),
    TabStop(usize),
    Query(SetOption),
}

/// Why a command line was rejected. Nothing runs when parsing fails.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExParseError {
    Empty,
    UnknownCommand(String),
    UnexpectedForce(&'static str),
    UnexpectedArgument(&'static str),
    MissingPath(&'static str),
    UnterminatedQuote,
    UnsupportedExpansion(char),
    MissingSetArgument,
    UnknownOption(String),
    InvalidValue { option: &'static str, value: String },
}

impl fmt::Display for ExParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "empty command"),
            Self::UnknownCommand(name) => write!(formatter, "not a supported command: :{name}"),
            Self::UnexpectedForce(name) => write!(formatter, ":{name} does not take !"),
            Self::UnexpectedArgument(name) => {
                write!(
                    formatter,
                    ":{name}: unexpected argument (quote paths that contain spaces)"
                )
            }
            Self::MissingPath(name) => write!(formatter, ":{name} needs a file path"),
            Self::UnterminatedQuote => write!(formatter, "missing closing quote"),
            Self::UnsupportedExpansion(character) => write!(
                formatter,
                "'{character}' expansion is not supported; quote the path to use it literally"
            ),
            Self::MissingSetArgument => write!(
                formatter,
                "usage: :set wrap=word|char|none|wrap|nowrap|expandtab|noexpandtab|tabstop=N|option?"
            ),
            Self::UnknownOption(name) => write!(formatter, "unknown option '{name}'"),
            Self::InvalidValue { option, value } => {
                write!(formatter, "invalid value for {option}: '{value}'")
            }
        }
    }
}

impl std::error::Error for ExParseError {}

/// Command names, their shortest accepted abbreviation, and whether they take
/// `!`. Order matters only for readability; abbreviations do not overlap.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Name {
    Write,
    SaveAs,
    Quit,
    WriteQuit,
    Exit,
    Edit,
    New,
    File,
    WorkingDirectory,
    NoHighlight,
    Set,
    Format,
    Trim,
    Retab,
    WriteAll,
    QuitAll,
    Read,
}

const NAMES: [(Name, &str, usize, bool); 17] = [
    (Name::Write, "write", 1, true),
    (Name::SaveAs, "saveas", 3, true),
    (Name::Quit, "quit", 1, true),
    (Name::WriteQuit, "wq", 2, false),
    (Name::Exit, "xit", 1, false),
    (Name::Edit, "edit", 1, true),
    (Name::New, "enew", 3, true),
    (Name::File, "file", 1, false),
    (Name::WorkingDirectory, "pwd", 2, false),
    (Name::NoHighlight, "nohlsearch", 3, false),
    (Name::Set, "set", 2, false),
    (Name::Format, "format", 6, false),
    (Name::Trim, "trim", 4, false),
    (Name::Retab, "retab", 5, false),
    (Name::WriteAll, "wall", 2, false),
    (Name::QuitAll, "qall", 2, true),
    (Name::Read, "read", 1, false),
];

fn lookup(name: &str) -> Option<(Name, &'static str, bool)> {
    NAMES
        .iter()
        .find(|(_, full, minimum, _)| name.len() >= *minimum && full.starts_with(name))
        .map(|(kind, full, _, force)| (*kind, *full, *force))
}

/// Split `text` into the command name, its `!` flag and the argument text.
fn split(text: &str) -> Result<(&str, bool, &str), ExParseError> {
    let text = text.trim_start().trim_start_matches(':').trim();
    if text.is_empty() {
        return Err(ExParseError::Empty);
    }
    let name_end = text
        .find(|character: char| !character.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let name = &text[..name_end];
    let rest = &text[name_end..];
    let (force, rest) = match rest.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    Ok((name, force, rest))
}

/// Parse one command line (without requiring the leading `:`).
pub fn parse(text: &str) -> Result<ExCommand, ExParseError> {
    let trimmed = text.trim_start().trim_start_matches(':').trim();
    if trimmed == "$" {
        return Ok(ExCommand::GotoLine(LineTarget::Last));
    }
    if !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        let target = match trimmed.parse::<usize>() {
            Ok(number) => {
                LineTarget::Number(NonZeroUsize::new(number).unwrap_or(NonZeroUsize::MIN))
            }
            Err(_) => LineTarget::Last,
        };
        return Ok(ExCommand::GotoLine(target));
    }

    let (name, force, rest) = split(text)?;
    if name.is_empty() {
        return Err(ExParseError::UnknownCommand(trimmed.to_owned()));
    }
    let Some((kind, full, takes_force)) = lookup(name) else {
        return Err(ExParseError::UnknownCommand(name.to_owned()));
    };
    if force && !takes_force {
        return Err(ExParseError::UnexpectedForce(full));
    }
    // Vim accepts `:w/tmp/x`; this parser requires a separator so a typo such
    // as `:wq1` cannot be read as `:w` with a file named `q1`.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Err(ExParseError::UnknownCommand(format!("{name}{rest}")));
    }
    let argument = rest.trim();
    let no_argument = |command: ExCommand| {
        if argument.is_empty() {
            Ok(command)
        } else {
            Err(ExParseError::UnexpectedArgument(full))
        }
    };

    match kind {
        Name::Write => Ok(ExCommand::Write {
            force,
            path: parse_path(argument, full)?,
        }),
        Name::SaveAs => Ok(ExCommand::SaveAs {
            force,
            path: parse_path(argument, full)?.ok_or(ExParseError::MissingPath(full))?,
        }),
        Name::Quit => no_argument(ExCommand::Quit { force }),
        Name::WriteQuit => no_argument(ExCommand::WriteQuit),
        Name::Exit => no_argument(ExCommand::Exit),
        Name::Edit => {
            let path = parse_path(argument, full)?;
            if path.is_none() && !force {
                return Err(ExParseError::MissingPath(full));
            }
            Ok(ExCommand::Edit { force, path })
        }
        Name::New => no_argument(ExCommand::New { force }),
        Name::File => no_argument(ExCommand::File),
        Name::WorkingDirectory => no_argument(ExCommand::WorkingDirectory),
        Name::NoHighlight => no_argument(ExCommand::NoHighlight),
        Name::Set => parse_set(argument).map(ExCommand::Set),
        Name::Format => no_argument(ExCommand::Format),
        Name::Trim => no_argument(ExCommand::Trim),
        Name::Retab => no_argument(ExCommand::Retab),
        Name::WriteAll => no_argument(ExCommand::WriteAll),
        Name::QuitAll => no_argument(ExCommand::QuitAll { force }),
        Name::Read => Ok(ExCommand::Read {
            path: parse_path(argument, full)?.ok_or(ExParseError::MissingPath(full))?,
        }),
    }
}

fn parse_set(argument: &str) -> Result<Vec<SetArgument>, ExParseError> {
    if argument.is_empty() {
        return Err(ExParseError::MissingSetArgument);
    }
    argument
        .split_whitespace()
        .map(parse_set_argument)
        .collect()
}

fn parse_set_argument(argument: &str) -> Result<SetArgument, ExParseError> {
    if let Some(name) = argument.strip_suffix('?') {
        return SetOption::parse(name)
            .map(SetArgument::Query)
            .ok_or_else(|| ExParseError::UnknownOption(name.to_owned()));
    }
    if let Some((name, value)) = argument.split_once('=') {
        let invalid = |option: &'static str| ExParseError::InvalidValue {
            option,
            value: value.to_owned(),
        };
        return match SetOption::parse(name) {
            Some(SetOption::Wrap) => WrapMode::parse(value)
                .map(SetArgument::Wrap)
                .ok_or_else(|| invalid("wrap")),
            Some(SetOption::TabStop) => match value.parse::<usize>() {
                Ok(tab_stop) if (1..=MAX_TAB_STOP).contains(&tab_stop) => {
                    Ok(SetArgument::TabStop(tab_stop))
                }
                _ => Err(invalid("tabstop")),
            },
            Some(SetOption::ExpandTab) => Err(invalid("expandtab")),
            None => Err(ExParseError::UnknownOption(name.to_owned())),
        };
    }
    match argument {
        "wrap" => Ok(SetArgument::Wrap(WrapMode::Word)),
        "nowrap" => Ok(SetArgument::Wrap(WrapMode::None)),
        "expandtab" | "et" => Ok(SetArgument::ExpandTab(true)),
        "noexpandtab" | "noet" => Ok(SetArgument::ExpandTab(false)),
        _ => Err(ExParseError::UnknownOption(argument.to_owned())),
    }
}

/// Parse the single path argument of a file command.
///
/// Grammar, deliberately small:
/// - `"..."` takes everything up to the next `"` literally, backslashes
///   included. Paths cannot contain `"` (Windows forbids it in file names).
/// - Unquoted, `\ ` is an escaped space. Every other backslash is literal so
///   `C:\Users\me` and `\\server\share` keep their separators.
/// - Unquoted `*`, `?`, and a leading `~`, `%` or `#` would be expanded by
///   Vim; they are rejected instead of being taken as literal names.
/// - Relative paths are kept relative and resolve against the process
///   working directory when the file operation runs.
fn parse_path(argument: &str, command: &'static str) -> Result<Option<PathBuf>, ExParseError> {
    if argument.is_empty() {
        return Ok(None);
    }
    if let Some(quoted) = argument.strip_prefix('"') {
        let Some(end) = quoted.find('"') else {
            return Err(ExParseError::UnterminatedQuote);
        };
        if !quoted[end + 1..].trim().is_empty() {
            return Err(ExParseError::UnexpectedArgument(command));
        }
        let path = &quoted[..end];
        if path.is_empty() {
            return Err(ExParseError::MissingPath(command));
        }
        return Ok(Some(PathBuf::from(path)));
    }
    let (path, rest) = unescape_unquoted(argument);
    if !rest.trim().is_empty() {
        return Err(ExParseError::UnexpectedArgument(command));
    }
    if let Some(first @ ('~' | '%' | '#')) = path.chars().next() {
        return Err(ExParseError::UnsupportedExpansion(first));
    }
    if let Some(wildcard) = path
        .chars()
        .find(|character| matches!(character, '*' | '?'))
    {
        return Err(ExParseError::UnsupportedExpansion(wildcard));
    }
    Ok(Some(PathBuf::from(path)))
}

/// Read one unquoted token, turning `\ ` into a space. Returns the token and
/// the text after it.
fn unescape_unquoted(argument: &str) -> (String, &str) {
    let mut path = String::new();
    let mut characters = argument.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if character == '\\' {
            if let Some((_, ' ')) = characters.peek() {
                characters.next();
                path.push(' ');
                continue;
            }
            path.push('\\');
            continue;
        }
        if character.is_whitespace() {
            return (path, &argument[index..]);
        }
        path.push(character);
    }
    (path, "")
}

/// The partially typed path argument at the end of a command line, for Tab
/// completion. Only `:e`, `:w`, `:saveas` and `:r` take paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathArgument {
    /// Byte offset in the command text where the raw argument starts.
    pub(crate) start: usize,
    /// The argument as the person meant it, with quoting removed.
    pub typed: String,
    /// Whether the argument was opened with `"`.
    pub quoted: bool,
}

/// Locate a path argument that ends at the end of `text`.
#[must_use]
pub fn path_argument(text: &str) -> Option<PathArgument> {
    let leading = text.len() - text.trim_start().trim_start_matches(':').len();
    let body = &text[leading..];
    let (name, _, rest) = split(body).ok()?;
    let (kind, _, _) = lookup(name)?;
    if !matches!(kind, Name::Write | Name::SaveAs | Name::Edit | Name::Read) {
        return None;
    }
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let argument = rest.trim_start();
    let start = text.len() - argument.len();
    if let Some(quoted) = argument.strip_prefix('"') {
        if quoted.contains('"') {
            return None;
        }
        return Some(PathArgument {
            start,
            typed: quoted.to_owned(),
            quoted: true,
        });
    }
    let (typed, after) = unescape_unquoted(argument);
    if !after.is_empty() {
        return None;
    }
    Some(PathArgument {
        start,
        typed,
        quoted: false,
    })
}

/// Spell `path` so [`parse`] reads it back unchanged. Returns `None` for a
/// path that cannot be written in the grammar (it contains `"`).
#[must_use]
pub fn quote_path(path: &str) -> Option<String> {
    if path.contains('"') {
        return None;
    }
    let needs_quotes = path.contains(char::is_whitespace)
        || path.contains(['*', '?'])
        || path.starts_with(['~', '%', '#']);
    if needs_quotes {
        Some(format!("\"{path}\""))
    } else {
        Some(path.to_owned())
    }
}
