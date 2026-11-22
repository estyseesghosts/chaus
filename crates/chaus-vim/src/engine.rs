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

/// How a repeated change should be replayed.
#[derive(Clone, Debug)]
pub(crate) enum LastChange {
    Keys(Vec<Key>),
    Insert(String),
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
    motion_count: usize,
    register: Register,
    mappings: Vec<Mapping>,
    mapping_prefix: Vec<Key>,
    queue: VecDeque<Key>,
    visual_anchor: Option<CharOffset>,
    command: String,
    command_kind: CommandKind,
    last_search: Option<(String, bool)>,
    last_find: Option<char>,
    last_change: Option<LastChange>,
    insert_buffer: String,
    changed: bool,
    message: Option<String>,
    replaying: bool,
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
            insert_buffer: String::new(),
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

    /// Return the last informational message, if any.
    pub fn take_message(&mut self) -> Option<String> {
        self.message.take()
    }

    /// Forget every partial sequence. Used when Vim mode is toggled off or the
    /// editor loses focus to a dialog.
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
        self.insert_buffer.clear();
        self.replaying = false;
    }

    /// Text committed while in Insert mode, used by `.` to repeat an insertion.
    pub fn record_inserted(&mut self, text: &str) {
        self.insert_buffer.push_str(text);
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
        self.queue.push_back(key);
        let mut outcome = Outcome::Handled;
        while let Some(key) = self.queue.pop_front() {
            let step = if self.mode == Mode::Normal
                && self.pending.is_none()
                && self.count == 0
                && self.motion_count == 0
                && !self.mappings.is_empty()
            {
                self.resolve_mapping(key)
            } else {
                MappingStep::Direct(key)
            };
            match step {
                MappingStep::Wait => {}
                MappingStep::Keys(keys) => {
                    for key in keys.into_iter().rev() {
                        self.queue.push_front(key);
                    }
                }
                MappingStep::Fallback(keys) => {
                    let mut keys = keys;
                    let first = keys.remove(0);
                    for key in keys.into_iter().rev() {
                        self.queue.push_front(key);
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

    pub(crate) fn set_last_change(&mut self, change: Option<LastChange>) {
        if !self.replaying {
            self.last_change = change;
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

    pub(crate) fn insert_buffer(&mut self) -> &mut String {
        &mut self.insert_buffer
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
