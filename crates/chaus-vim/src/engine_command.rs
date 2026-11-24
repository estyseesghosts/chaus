//! Keys and text routed to the `:`, `/` and `?` input line.

use chaus_core::document::Document;
use chaus_core::position::Selection;

use crate::command_line::{CommandFull, CommandPrefix, MAX_COMMAND_CHARS};
use crate::engine::{Engine, Outcome};
use crate::ex_command;
use crate::key::Key;
use crate::mode::Mode;
use crate::motion;

/// What the editor draws for the active input line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLineView<'a> {
    pub prefix: CommandPrefix,
    pub before_caret: &'a str,
    pub after_caret: &'a str,
    /// IME composition shown at the caret. It is not part of the command.
    pub preedit: Option<&'a str>,
}

/// A path argument that Tab may complete. `revision` ties the eventual answer
/// to the exact line it was computed for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    pub revision: u64,
    /// The partial path, with command-line quoting removed.
    pub typed: String,
}

fn full_message() -> String {
    format!("command line is limited to {MAX_COMMAND_CHARS} characters")
}

impl Engine {
    pub(crate) fn command_line_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match key {
            Key::Escape => {
                self.take_command_line();
                self.set_mode(Mode::Normal);
                Outcome::Handled
            }
            Key::Enter => self.submit_command_line(document, selection),
            Key::Backspace => {
                // Backspace on an empty line leaves it, as in Vim.
                if self
                    .command_line_ref()
                    .is_some_and(|line| line.text().is_empty())
                {
                    self.take_command_line();
                    self.set_mode(Mode::Normal);
                } else if let Some(line) = self.command_line_mut() {
                    line.backspace();
                }
                Outcome::Handled
            }
            Key::Delete => self.edit_line(|line| line.delete()),
            Key::Left => self.edit_line(|line| line.left()),
            Key::Right => self.edit_line(|line| line.right()),
            Key::Home => self.edit_line(|line| line.home()),
            Key::End => self.edit_line(|line| line.end()),
            Key::Up => {
                if let Some((line, history)) = self.command_line_and_history() {
                    line.history_older(history);
                }
                Outcome::Handled
            }
            Key::Down => {
                if let Some((line, history)) = self.command_line_and_history() {
                    line.history_newer(history);
                }
                Outcome::Handled
            }
            Key::Tab => {
                let is_ex = self
                    .command_line_ref()
                    .is_some_and(|line| line.prefix() == CommandPrefix::Ex);
                if is_ex {
                    Outcome::Complete
                } else {
                    Outcome::Handled
                }
            }
            Key::Char(character) => {
                let mut buffer = [0; 4];
                self.insert_command_text(character.encode_utf8(&mut buffer))
            }
            Key::Control(_) => Outcome::Handled,
        }
    }

    fn edit_line(&mut self, edit: impl FnOnce(&mut crate::command_line::CommandLine)) -> Outcome {
        if let Some(line) = self.command_line_mut() {
            edit(line);
        }
        Outcome::Handled
    }

    fn submit_command_line(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some(line) = self.take_command_line() else {
            self.set_mode(Mode::Normal);
            return Outcome::Handled;
        };
        self.set_mode(Mode::Normal);
        let prefix = line.prefix();
        let text = line.text().to_owned();
        self.remember_command(prefix, &text);
        match prefix {
            CommandPrefix::SearchForward | CommandPrefix::SearchBackward => {
                let forward = prefix == CommandPrefix::SearchForward;
                if text.is_empty() {
                    return Outcome::Handled;
                }
                if let Some(offset) = motion::find_literal(document, selection.head, &text, forward)
                {
                    *selection = Selection::caret(offset);
                    self.set_last_search(Some((text, forward)));
                    Outcome::Handled
                } else {
                    Outcome::Message(format!("pattern not found: {text}"))
                }
            }
            CommandPrefix::Ex => {
                if text.trim().trim_start_matches(':').trim().is_empty() {
                    return Outcome::Handled;
                }
                match ex_command::parse(&text) {
                    Ok(command) => Outcome::Ex(command),
                    Err(error) => Outcome::Message(error.to_string()),
                }
            }
        }
    }

    /// Text for the active command line from paste or the platform text
    /// service (IME commit, dead keys, AltGr). Never touches the document.
    pub fn insert_command_text(&mut self, text: &str) -> Outcome {
        let Some(line) = self.command_line_mut() else {
            return Outcome::Unhandled;
        };
        match line.insert(text) {
            Ok(()) => Outcome::Handled,
            Err(CommandFull) => Outcome::Message(full_message()),
        }
    }

    /// Show or clear an uncommitted IME composition on the command line.
    pub fn set_command_preedit(&mut self, preedit: Option<String>) {
        if let Some(line) = self.command_line_mut() {
            line.set_preedit(preedit);
        }
    }

    /// The input line to draw, if one is open.
    #[must_use]
    pub fn command_line(&self) -> Option<CommandLineView<'_>> {
        self.command_line_ref().map(|line| CommandLineView {
            prefix: line.prefix(),
            before_caret: line.before_caret(),
            after_caret: line.after_caret(),
            preedit: line.preedit(),
        })
    }

    /// Path completion is offered only for the path argument of `:e`, `:w`,
    /// `:saveas` and `:r`, and only with the caret at the end of the line.
    #[must_use]
    pub fn completion_request(&self) -> Option<CompletionRequest> {
        let line = self.command_line_ref()?;
        if line.prefix() != CommandPrefix::Ex || !line.caret_at_end() {
            return None;
        }
        let argument = ex_command::path_argument(line.text())?;
        Some(CompletionRequest {
            revision: line.revision(),
            typed: argument.typed,
        })
    }

    /// Replace the path argument with `completed`, if the line is unchanged
    /// since `revision`. `finished` closes an opened quote (a file was
    /// chosen); a directory leaves it open for more typing. Returns a message
    /// when the result cannot be applied.
    pub fn apply_completion(
        &mut self,
        revision: u64,
        completed: &str,
        finished: bool,
    ) -> Option<String> {
        let line = self.command_line_mut()?;
        if line.revision() != revision {
            return None;
        }
        let argument = ex_command::path_argument(line.text())?;
        let spelled = if argument.quoted {
            if completed.contains('"') {
                return Some("cannot complete a path that contains '\"'".to_owned());
            }
            let closing = if finished { "\"" } else { "" };
            format!("\"{completed}{closing}")
        } else {
            match ex_command::quote_path(completed) {
                // An unterminated quote keeps the line open for more typing.
                Some(quoted) if quoted.starts_with('"') && !finished => {
                    quoted.trim_end_matches('"').to_owned()
                }
                Some(spelled) => spelled,
                None => return Some("cannot complete a path that contains '\"'".to_owned()),
            }
        };
        match line.replace_tail(argument.start, &spelled) {
            Ok(()) => None,
            Err(CommandFull) => Some(full_message()),
        }
    }
}
