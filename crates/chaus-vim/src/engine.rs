//! The modal engine and its public boundary.

use std::collections::VecDeque;

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, LineIndex, Selection};

use crate::command_line::{CommandLine, CommandPrefix, CompletionToken, History};
use crate::ex_command::ExCommand;
use crate::key::Key;
use crate::mapping::Mapping;
use crate::mode::Mode;
use crate::motion::{FindDirection, FindState};
use crate::register::Register;
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
        direction: FindDirection,
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

/// How `.` should replay the last change: the keys that produced it,
/// including any count digits, followed by the text a change command inserted.
#[derive(Clone, Debug)]
pub(crate) struct LastChange {
    pub(crate) keys: Vec<Key>,
    pub(crate) inserted: Option<String>,
}

/// The modal state machine.
pub struct Engine {
    mode: Mode,
    pub(crate) pending: Option<Pending>,
    pub(crate) count: usize,
    pub(crate) motion_count: usize,
    register: Register,
    mappings: Vec<Mapping>,
    mapping_prefix: Vec<Key>,
    queue: VecDeque<QueuedKey>,
    visual_anchor: Option<CharOffset>,
    command_line: Option<CommandLine>,
    /// Per-engine, monotonically increasing identity of the current input
    /// line. Never shared between editor windows.
    pub(crate) command_generation: u64,
    /// Orders Tab requests inside one line, including ones with no text change.
    pub(crate) completion_sequence: u64,
    /// The only request whose result may still be applied.
    pub(crate) latest_completion: Option<CompletionToken>,
    ex_history: History,
    search_history: History,
    last_search: Option<(String, bool)>,
    last_find: Option<FindState>,
    last_change: Option<LastChange>,
    insert_start: Option<CharOffset>,
    insert_keys: Option<Vec<Key>>,
    insert_always_records: bool,
    insert_repeat: usize,
    changed: bool,
    message: Option<String>,
    replaying: bool,
}

struct QueuedKey {
    key: Key,
    /// Mapping expansions are dispatched directly; only keys that arrived from
    /// the platform may start another user mapping.
    resolve_mapping: bool,
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
            mappings,
            mapping_prefix: Vec::new(),
            queue: VecDeque::new(),
            visual_anchor: None,
            command_line: None,
            command_generation: 0,
            completion_sequence: 0,
            latest_completion: None,
            ex_history: History::default(),
            search_history: History::default(),
            last_search: None,
            last_find: None,
            last_change: None,
            insert_start: None,
            insert_keys: None,
            insert_always_records: false,
            insert_repeat: 1,
            changed: false,
            message: None,
            replaying: false,
        }
    }

    /// Replace the mapping table atomically. Pending prefixes are abandoned.
    pub fn set_mappings(&mut self, mappings: Vec<Mapping>) {
        self.mappings = mappings;
        self.mapping_prefix.clear();
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether an ambiguous mapping prefix is waiting for another key.
    #[must_use]
    pub fn mapping_prefix_pending(&self) -> bool {
        !self.mapping_prefix.is_empty()
    }

    /// Drop a partially typed command, count, mapping prefix, Visual selection
    /// or Ex input without leaving Insert mode. Used when focus moves to a
    /// dialog or another window.
    pub fn cancel_prefix(&mut self) {
        self.pending = None;
        self.count = 0;
        self.motion_count = 0;
        self.mapping_prefix.clear();
        self.queue.clear();
        self.command_line = None;
        self.latest_completion = None;
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
        self.mapping_prefix.clear();
        self.queue.clear();
        self.command_line = None;
        self.latest_completion = None;
        self.visual_anchor = None;
        self.insert_start = None;
        self.insert_keys = None;
        self.insert_always_records = false;
        self.replaying = false;
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

        self.changed = false;
        self.queue.push_back(QueuedKey {
            key,
            resolve_mapping: true,
        });
        let mut outcome = Outcome::Handled;
        while let Some(queued) = self.queue.pop_front() {
            let step = if queued.resolve_mapping
                && self.mode == Mode::Normal
                && self.pending.is_none()
                && self.count == 0
                && self.motion_count == 0
                && !self.mappings.is_empty()
            {
                self.resolve_mapping(queued.key)
            } else {
                MappingStep::Direct(queued.key)
            };
            match step {
                MappingStep::Wait => {}
                MappingStep::Keys(keys) => {
                    // A mapping target is executed as typed keys, not fed
                    // back through the mapping table (non-recursive mapping).
                    for key in keys.into_iter().rev() {
                        self.queue.push_front(QueuedKey {
                            key,
                            resolve_mapping: false,
                        });
                    }
                }
                MappingStep::Fallback(keys) => {
                    let mut keys = keys;
                    let first = keys.remove(0);
                    for key in keys.into_iter().rev() {
                        self.queue.push_front(QueuedKey {
                            key,
                            resolve_mapping: true,
                        });
                    }
                    let result = self.dispatch_direct(first, document, selection);
                    outcome = merge_outcome(outcome, result);
                }
                MappingStep::Direct(key) => {
                    let result = self.dispatch_direct(key, document, selection);
                    outcome = merge_outcome(outcome, result);
                }
            }
        }

        if self.changed {
            return Outcome::Changed;
        }
        if let Some(message) = self.message.take() {
            return Outcome::Message(message);
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
        let keys = std::mem::take(&mut self.mapping_prefix);
        if keys.is_empty() {
            return Outcome::Handled;
        }
        self.changed = false;
        self.message = None;
        let mut outcome = Outcome::Handled;
        for key in keys {
            let result = self.dispatch_direct(key, document, selection);
            outcome = merge_outcome(outcome, result);
        }
        if self.changed {
            return Outcome::Changed;
        }
        if let Some(message) = self.message.take() {
            return Outcome::Message(message);
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

    fn resolve_mapping(&mut self, key: Key) -> MappingStep {
        self.mapping_prefix.push(key);
        if let Some(mapping) = self
            .mappings
            .iter()
            .find(|mapping| mapping.keys() == self.mapping_prefix.as_slice())
        {
            let rhs = mapping.rhs().to_vec();
            self.mapping_prefix.clear();
            return MappingStep::Keys(rhs);
        }
        if self
            .mappings
            .iter()
            .any(|mapping| mapping.keys().starts_with(&self.mapping_prefix))
        {
            return MappingStep::Wait;
        }
        MappingStep::Fallback(std::mem::take(&mut self.mapping_prefix))
    }

    pub(crate) fn message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
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
        self.command_generation = self.command_generation.wrapping_add(1);
        self.completion_sequence = 0;
        self.latest_completion = None;
        self.command_line = Some(CommandLine::new(prefix, self.command_generation));
        self.reset_count();
        Outcome::Handled
    }

    pub(crate) fn command_line_mut(&mut self) -> Option<&mut CommandLine> {
        self.command_line.as_mut()
    }

    pub(crate) fn command_line_ref(&self) -> Option<&CommandLine> {
        self.command_line.as_ref()
    }

    pub(crate) fn take_command_line(&mut self) -> Option<CommandLine> {
        let line = self.command_line.take();
        if line.is_some() {
            self.latest_completion = None;
        }
        line
    }

    /// The command line and the history that belongs to its kind.
    pub(crate) fn command_line_and_history(&mut self) -> Option<(&mut CommandLine, &History)> {
        let line = self.command_line.as_mut()?;
        let history = match line.prefix() {
            CommandPrefix::Ex => &self.ex_history,
            CommandPrefix::SearchForward | CommandPrefix::SearchBackward => &self.search_history,
        };
        Some((line, history))
    }

    pub(crate) fn remember_command(&mut self, prefix: CommandPrefix, text: &str) {
        match prefix {
            CommandPrefix::Ex => self.ex_history.push(text),
            CommandPrefix::SearchForward | CommandPrefix::SearchBackward => {
                self.search_history.push(text);
            }
        }
    }

    pub(crate) fn last_search(&self) -> Option<&(String, bool)> {
        self.last_search.as_ref()
    }

    pub(crate) fn set_last_search(&mut self, search: Option<(String, bool)>) {
        self.last_search = search;
    }

    pub(crate) fn last_find(&self) -> Option<FindState> {
        self.last_find
    }

    pub(crate) fn set_last_find(&mut self, state: Option<FindState>) {
        self.last_find = state;
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

    /// Remember a completed change for `.`. Replaying never overwrites it.
    pub(crate) fn record_change(&mut self, keys: Vec<Key>, inserted: Option<String>) {
        if !self.replaying {
            self.last_change = Some(LastChange { keys, inserted });
        }
    }

    pub(crate) fn last_change(&self) -> Option<&LastChange> {
        self.last_change.as_ref()
    }

    pub(crate) fn set_changed(&mut self) {
        self.changed = true;
    }

    pub(crate) fn begin_replay(&mut self) {
        self.replaying = true;
    }

    pub(crate) fn end_replay(&mut self) {
        self.replaying = false;
    }

    /// Start an Insert-mode session. `keys` are the Normal-mode keys that
    /// entered the mode, and `always_record` keeps an empty change (for
    /// operators and `o`/`O`) available to `.`.
    pub(crate) fn begin_insert(
        &mut self,
        start: CharOffset,
        keys: Vec<Key>,
        always_record: bool,
        repeat: usize,
    ) {
        self.insert_start = Some(start);
        self.insert_keys = Some(keys);
        self.insert_always_records = always_record;
        self.insert_repeat = repeat.max(1);
    }

    pub(crate) fn take_insert_session(&mut self) -> Option<(CharOffset, Vec<Key>, bool, usize)> {
        let start = self.insert_start.take()?;
        let keys = self.insert_keys.take().unwrap_or_default();
        let always_record = std::mem::take(&mut self.insert_always_records);
        let repeat = std::mem::replace(&mut self.insert_repeat, 1);
        Some((start, keys, always_record, repeat))
    }
}

enum MappingStep {
    Wait,
    Keys(Vec<Key>),
    Fallback(Vec<Key>),
    Direct(Key),
}

fn merge_outcome(current: Outcome, next: Outcome) -> Outcome {
    match next {
        Outcome::Unhandled => current,
        Outcome::Handled => current,
        other => other,
    }
}
