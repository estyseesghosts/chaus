//! The modal engine and its public boundary.

use std::collections::VecDeque;

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};

use crate::key::Key;
use crate::mapping::Mapping;
use crate::mode::Mode;
use crate::register::Register;

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
    /// A complete Ex command line, without the leading colon.
    Command(String),
    /// A non-blocking explanation for an unknown or rejected command.
    Message(String),
}

/// One half-completed Normal-mode sequence.
pub(crate) enum Pending {
    Operator(crate::operator::Operator),
    G,
    Find,
    Replace,
    TextObject {
        operator: crate::operator::Operator,
        outer: bool,
    },
}

/// How `.` should replay the last change: the keys that produced it,
/// including any count digits, followed by the text a change command inserted.
#[derive(Clone, Debug)]
pub(crate) struct LastChange {
    pub(crate) keys: Vec<Key>,
    pub(crate) inserted: Option<String>,
}

pub(crate) enum CommandKind {
    Search { forward: bool },
    Ex,
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
    command: String,
    command_kind: CommandKind,
    last_search: Option<(String, bool)>,
    last_find: Option<char>,
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
            command: String::new(),
            command_kind: CommandKind::Ex,
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
        self.command.clear();
        self.command_kind = CommandKind::Ex;
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
        self.command.clear();
        self.command_kind = CommandKind::Ex;
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

    pub(crate) fn command_mut(&mut self) -> &mut String {
        &mut self.command
    }

    pub(crate) fn take_command(&mut self) -> String {
        std::mem::take(&mut self.command)
    }

    pub(crate) fn command_kind(&self) -> &CommandKind {
        &self.command_kind
    }

    pub(crate) fn set_command_kind(&mut self, kind: CommandKind) {
        self.command_kind = kind;
    }

    pub(crate) fn last_search(&self) -> Option<&(String, bool)> {
        self.last_search.as_ref()
    }

    pub(crate) fn set_last_search(&mut self, search: Option<(String, bool)>) {
        self.last_search = search;
    }

    pub(crate) fn last_find(&self) -> Option<char> {
        self.last_find
    }

    pub(crate) fn set_last_find(&mut self, character: Option<char>) {
        self.last_find = character;
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
