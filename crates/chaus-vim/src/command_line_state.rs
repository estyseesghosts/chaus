//! The active input line together with its identity, completions and histories.

use crate::command_line::{CommandLine, CommandPrefix, CompletionToken, History};

#[derive(Default)]
pub(crate) struct CommandLineState {
    line: Option<CommandLine>,
    /// Per-engine, monotonically increasing identity of the current input
    /// line. Never shared between editor windows.
    command_generation: u64,
    /// Orders Tab requests inside one line, including ones with no text change.
    completion_sequence: u64,
    /// The only request whose result may still be applied.
    latest_completion: Option<CompletionToken>,
    ex_history: History,
    search_history: History,
}

impl CommandLineState {
    /// Open an empty line, invalidating completion results from earlier lines.
    pub(crate) fn open(&mut self, prefix: CommandPrefix) {
        self.command_generation = self.command_generation.wrapping_add(1);
        self.completion_sequence = 0;
        self.latest_completion = None;
        self.line = Some(CommandLine::new(prefix, self.command_generation));
    }

    /// Discard a partial line and its completion result. Request identities are
    /// left alone so a stale result cannot match a later line.
    pub(crate) fn clear(&mut self) {
        self.line = None;
        self.latest_completion = None;
    }

    pub(crate) fn line_mut(&mut self) -> Option<&mut CommandLine> {
        self.line.as_mut()
    }

    pub(crate) fn line_ref(&self) -> Option<&CommandLine> {
        self.line.as_ref()
    }

    pub(crate) fn take_line(&mut self) -> Option<CommandLine> {
        let line = self.line.take();
        if line.is_some() {
            self.latest_completion = None;
        }
        line
    }

    /// The command line and the history that belongs to its kind.
    pub(crate) fn line_and_history(&mut self) -> Option<(&mut CommandLine, &History)> {
        let prefix = self.line.as_ref()?.prefix();
        let history = match prefix {
            CommandPrefix::Ex => &self.ex_history,
            CommandPrefix::SearchForward | CommandPrefix::SearchBackward => &self.search_history,
        };
        let line = self.line.as_mut()?;
        Some((line, history))
    }

    pub(crate) fn remember(&mut self, prefix: CommandPrefix, text: &str) {
        match prefix {
            CommandPrefix::Ex => self.ex_history.push(text),
            CommandPrefix::SearchForward | CommandPrefix::SearchBackward => {
                self.search_history.push(text);
            }
        }
    }

    /// Issue a new Tab request on the open line. Repeated presses without a
    /// text change get distinct sequence numbers.
    pub(crate) fn issue_completion(&mut self) -> Option<CompletionToken> {
        let sequence = self.completion_sequence.wrapping_add(1);
        let token = self.line.as_ref()?.token(sequence);
        self.completion_sequence = sequence;
        self.latest_completion = Some(token);
        Some(token)
    }

    /// Whether `token` is still the newest request of the open line.
    pub(crate) fn is_current_completion(&self, token: &CompletionToken) -> bool {
        self.latest_completion == Some(*token)
            && self
                .line
                .as_ref()
                .is_some_and(|line| line.token(self.completion_sequence) == *token)
    }
}
