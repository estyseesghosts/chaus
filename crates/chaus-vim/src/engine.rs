//! The modal engine and its public boundary.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, LineIndex, Selection};

use crate::command_line::{CommandLine, CommandPrefix, CompletionToken, History};
use crate::command_line_state::CommandLineState;
use crate::ex_command::ExCommand;
use crate::key::Key;
use crate::mapping::Mapping;
use crate::mapping_state::{MappingState, MappingStep};
use crate::mode::Mode;
use crate::motion::FindState;
use crate::register::Register;
use crate::repeat_state::{LastChange, RepeatState};
use crate::search_state::SearchState;
use crate::viewport::ViewportIntent;

/// What the editor layer should do after a key.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key was consumed; the caret may have moved.
    Handled,
    /// The key was consumed and the document text changed.
    Changed,
    /// The key is not part of the modal grammar (Insert mode passes text
    /// through to the platform input service).
    Unhandled,
    /// A validated Ex command. Lines that fail to parse become
    /// [`Outcome::Message`] and never reach the editor as commands.
    Ex(ExCommand),
    /// Tab in an Ex line: the editor may complete the path argument through
    /// [`Engine::completion_request`] and [`Engine::apply_completion`].
    Complete,
    /// A non-blocking explanation for an unknown or rejected command.
    Message(String),
    /// Indent or outdent whole hard lines through the editor's existing
    /// indentation engine. The engine has already left Visual mode.
    Indent {
        outdent: bool,
        first_line: LineIndex,
        last_line: LineIndex,
    },
    /// Place or scroll the viewport without exposing UI geometry here.
    Viewport(ViewportIntent),
}

/// One half-completed Normal-mode sequence.
pub(crate) enum Pending {
    Operator(crate::operator::Operator),
    /// An operator followed by `g`, waiting for `g` or `e`.
    OperatorG(crate::operator::Operator),
    /// `g` in Normal mode: `gg`, `ge`, or `gJ`.
    G,
    Find {
        operator: Option<crate::operator::Operator>,
        direction: crate::motion::FindDirection,
        till: bool,
    },
    Replace,
    TextObject {
        operator: Option<crate::operator::Operator>,
        outer: bool,
    },
    Indent {
        outdent: bool,
    },
    Z,
    Scroll,
}

/// The modal state machine.
pub struct Engine {
    mode: Mode,
    pub(crate) pending: Option<Pending>,
    pub(crate) count: usize,
    pub(crate) motion_count: usize,
    register: Register,
    visual_anchor: Option<CharOffset>,
    mapping_state: MappingState,
    command_line_state: CommandLineState,
    repeat_state: RepeatState,
    search_state: SearchState,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Engine {
    /// Build an engine with validated Normal-mode mappings.
    #[must_use]
    pub fn new(mappings: Vec<Mapping>) -> Self {
        Self {
            mode: Mode::Normal,
            pending: None,
            count: 0,
            motion_count: 0,
            register: Register::default(),
            visual_anchor: None,
            mapping_state: MappingState::new(mappings),
            command_line_state: CommandLineState::default(),
            repeat_state: RepeatState::default(),
            search_state: SearchState::default(),
        }
    }

    /// Replace the mapping table atomically. Pending prefixes are abandoned.
    pub fn set_mappings(&mut self, mappings: Vec<Mapping>) {
        self.mapping_state.set_mappings(mappings);
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether an ambiguous mapping prefix is waiting for another key.
    #[must_use]
    pub fn mapping_prefix_pending(&self) -> bool {
        self.mapping_state.prefix_pending()
    }

    /// Drop a partially typed command, count, mapping prefix, Visual selection
    /// or Ex input without leaving Insert mode. Used when focus moves to a
    /// dialog or another window.
    pub fn cancel_prefix(&mut self) {
        self.pending = None;
        self.count = 0;
        self.motion_count = 0;
        self.mapping_state.clear();
        self.command_line_state.clear();
        self.visual_anchor = None;
        if matches!(
            self.mode,
            Mode::VisualChar | Mode::VisualLine | Mode::CommandLine
        ) {
            self.mode = Mode::Normal;
        }
    }

    /// Forget every partial sequence and return to Normal. Used when Vim mode
    /// is toggled off; focus changes use [`Engine::cancel_prefix`] instead.
    pub fn reset(&mut self) {
        self.mode = Mode::Normal;
        self.pending = None;
        self.count = 0;
        self.motion_count = 0;
        self.mapping_state.clear();
        self.command_line_state.clear();
        self.visual_anchor = None;
        self.repeat_state.clear();
    }

    /// Feed one key. Insert-mode keys other than Escape are unhandled so the
    /// platform text service can insert them.
    pub fn handle_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        if self.mode == Mode::Insert {
            return match key {
                Key::Escape => {
                    self.finish_insert(document, selection);
                    Outcome::Handled
                }
                _ => Outcome::Unhandled,
            };
        }

        self.mapping_state.enqueue(key);
        let mut outcome = Outcome::Handled;
        loop {
            let resolve_mapping = self.mode == Mode::Normal
                && self.pending.is_none()
                && self.count == 0
                && self.motion_count == 0
                && self.mapping_state.has_mappings();
            let Some(step) = self.mapping_state.next_step(resolve_mapping) else {
                break;
            };
            match step {
                MappingStep::Wait => {}
                MappingStep::Keys(keys) => self.mapping_state.queue_expansion(keys),
                MappingStep::Fallback(keys) => {
                    if let Some(first) = self.mapping_state.queue_fallback(keys) {
                        let result = self.dispatch_direct(first, document, selection);
                        outcome = merge_outcome(outcome, result);
                    }
                }
                MappingStep::Direct(key) => {
                    let result = self.dispatch_direct(key, document, selection);
                    outcome = merge_outcome(outcome, result);
                }
            }
        }
        outcome
    }

    /// Resolve an ambiguous mapping prefix after the timeout. Buffered keys are
    /// replayed to the base interpreter (never back through the mapping
    /// table), so a prefix that is also a native command such as `j` still
    /// performs that command; a prefix with no native meaning reports it.
    pub fn resolve_mapping_timeout(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let keys = self.mapping_state.take_prefix();
        if keys.is_empty() {
            return Outcome::Handled;
        }
        let mut outcome = Outcome::Handled;
        for key in keys {
            let result = self.dispatch_direct(key, document, selection);
            outcome = merge_outcome(outcome, result);
        }
        outcome
    }

    pub(crate) fn dispatch_direct(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let key = if self.mode == Mode::CommandLine {
            key
        } else {
            key.as_motion()
        };
        match self.mode {
            Mode::Normal => self.normal_key(key, document, selection),
            Mode::VisualChar | Mode::VisualLine => self.visual_key(key, document, selection),
            Mode::CommandLine => self.command_line_key(key, document, selection),
            Mode::Insert => Outcome::Unhandled,
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.count
            .max(1)
            .saturating_mul(self.motion_count.max(1))
            .min(10_000)
    }

    /// Whether any count digit was typed. `G` needs this: plain `G` is the
    /// last line, while `1G` is line one.
    pub(crate) fn explicit_count(&self) -> bool {
        self.count > 0 || self.motion_count > 0
    }

    pub(crate) fn reset_count(&mut self) {
        self.count = 0;
        self.motion_count = 0;
    }

    pub(crate) fn push_digits(&mut self, digit: usize) {
        let slot = if self.pending.is_some() {
            &mut self.motion_count
        } else {
            &mut self.count
        };
        *slot = slot.saturating_mul(10).saturating_add(digit).min(10_000);
    }

    pub(crate) fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub(crate) fn pending(&mut self) -> &mut Option<Pending> {
        &mut self.pending
    }

    pub(crate) fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    pub(crate) fn register_mut(&mut self) -> &mut Register {
        &mut self.register
    }

    pub(crate) fn register(&self) -> &Register {
        &self.register
    }

    pub(crate) fn visual_anchor(&self) -> Option<CharOffset> {
        self.visual_anchor
    }

    pub(crate) fn set_visual_anchor(&mut self, anchor: Option<CharOffset>) {
        self.visual_anchor = anchor;
    }

    /// Open an empty input line of the given kind.
    pub(crate) fn start_command_line(&mut self, prefix: CommandPrefix) -> Outcome {
        self.set_mode(Mode::CommandLine);
        self.command_line_state.open(prefix);
        self.reset_count();
        Outcome::Handled
    }

    pub(crate) fn command_line_mut(&mut self) -> Option<&mut CommandLine> {
        self.command_line_state.line_mut()
    }

    pub(crate) fn command_line_ref(&self) -> Option<&CommandLine> {
        self.command_line_state.line_ref()
    }

    pub(crate) fn take_command_line(&mut self) -> Option<CommandLine> {
        self.command_line_state.take_line()
    }

    /// The command line and the history that belongs to its kind.
    pub(crate) fn command_line_and_history(&mut self) -> Option<(&mut CommandLine, &History)> {
        self.command_line_state.line_and_history()
    }

    pub(crate) fn remember_command(&mut self, prefix: CommandPrefix, text: &str) {
        self.command_line_state.remember(prefix, text);
    }

    pub(crate) fn issue_completion(&mut self) -> Option<CompletionToken> {
        self.command_line_state.issue_completion()
    }

    pub(crate) fn current_completion(&self, token: &CompletionToken) -> bool {
        self.command_line_state.is_current_completion(token)
    }

    pub(crate) fn last_search(&self) -> Option<&(String, bool)> {
        self.search_state.last_search()
    }

    pub(crate) fn set_last_search(&mut self, search: Option<(String, bool)>) {
        self.search_state.set_last_search(search);
    }

    pub(crate) fn last_find(&self) -> Option<FindState> {
        self.search_state.last_find()
    }

    pub(crate) fn set_last_find(&mut self, state: Option<FindState>) {
        self.search_state.set_last_find(state);
    }

    /// Prefix `keys` with digits for the active count so that replaying them
    /// repeats the same command without needing the count state.
    pub(crate) fn counted_keys(&self, keys: Vec<Key>) -> Vec<Key> {
        let count = self.count();
        if count <= 1 {
            return keys;
        }
        let mut counted: Vec<Key> = count.to_string().chars().map(Key::Char).collect();
        counted.extend(keys);
        counted
    }

    pub(crate) fn record_change(&mut self, keys: Vec<Key>, inserted: Option<String>) {
        self.repeat_state.record_change(keys, inserted);
    }

    pub(crate) fn last_change(&self) -> Option<&LastChange> {
        self.repeat_state.last_change()
    }

    pub(crate) fn begin_replay(&mut self) {
        self.repeat_state.begin_replay();
    }

    pub(crate) fn end_replay(&mut self) {
        self.repeat_state.end_replay();
    }

    pub(crate) fn begin_insert(
        &mut self,
        start: CharOffset,
        keys: Vec<Key>,
        always_record: bool,
        repeat: usize,
    ) {
        self.repeat_state
            .begin_insert(start, keys, always_record, repeat);
    }

    pub(crate) fn take_insert_session(&mut self) -> Option<(CharOffset, Vec<Key>, bool, usize)> {
        self.repeat_state.take_insert_session()
    }
}

/// Merge the outcome of one dispatched key into the outcome of a whole key
/// press. A change outranks a rejection message, which outranks other effects;
/// otherwise the last effect wins. `Handled` and `Unhandled` add nothing.
pub(crate) fn merge_outcome(current: Outcome, next: Outcome) -> Outcome {
    match (current, next) {
        (current, Outcome::Handled | Outcome::Unhandled) => current,
        (Outcome::Changed, _) | (_, Outcome::Changed) => Outcome::Changed,
        (_, next @ Outcome::Message(_)) => next,
        (current @ Outcome::Message(_), _) => current,
        (_, next) => next,
    }
}
