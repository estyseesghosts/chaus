//! Mapping table, ambiguous prefixes and the queue of keys awaiting dispatch.

use std::collections::VecDeque;

use crate::key::Key;
use crate::mapping::Mapping;

/// One step of mapping resolution for the key that was just dequeued.
pub(crate) enum MappingStep {
    Wait,
    Keys(Vec<Key>),
    Fallback(Vec<Key>),
    Direct(Key),
}

struct QueuedKey {
    key: Key,
    /// Mapping expansions are dispatched directly; only keys that arrived from
    /// the platform may start another user mapping.
    resolve_mapping: bool,
}

pub(crate) struct MappingState {
    mappings: Vec<Mapping>,
    prefix: Vec<Key>,
    queue: VecDeque<QueuedKey>,
}

impl MappingState {
    pub(crate) fn new(mappings: Vec<Mapping>) -> Self {
        Self {
            mappings,
            prefix: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    /// Replace the mapping table atomically. Pending prefixes are abandoned,
    /// but keys already queued from a previous expansion still run.
    pub(crate) fn set_mappings(&mut self, mappings: Vec<Mapping>) {
        self.mappings = mappings;
        self.prefix.clear();
    }

    pub(crate) fn has_mappings(&self) -> bool {
        !self.mappings.is_empty()
    }

    pub(crate) fn prefix_pending(&self) -> bool {
        !self.prefix.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.prefix.clear();
        self.queue.clear();
    }

    pub(crate) fn enqueue(&mut self, key: Key) {
        self.queue.push_back(QueuedKey {
            key,
            resolve_mapping: true,
        });
    }

    /// Choose the next step. `resolve_mapping` is the caller's answer to
    /// whether this key is in a context where a user mapping may start.
    pub(crate) fn next_step(&mut self, resolve_mapping: bool) -> Option<MappingStep> {
        let queued = self.queue.pop_front()?;
        if queued.resolve_mapping && resolve_mapping {
            Some(self.resolve(queued.key))
        } else {
            Some(MappingStep::Direct(queued.key))
        }
    }

    /// Queue a mapping's replacement keys so they run as typed keys, never
    /// through the mapping table again.
    pub(crate) fn queue_expansion(&mut self, keys: Vec<Key>) {
        for key in keys.into_iter().rev() {
            self.queue.push_front(QueuedKey {
                key,
                resolve_mapping: false,
            });
        }
    }

    /// Queue the rest of an ambiguous prefix for direct dispatch and return
    /// its first key.
    pub(crate) fn queue_fallback(&mut self, mut keys: Vec<Key>) -> Option<Key> {
        let first = keys.first().copied()?;
        for key in keys.drain(1..).rev() {
            self.queue.push_front(QueuedKey {
                key,
                resolve_mapping: true,
            });
        }
        Some(first)
    }

    pub(crate) fn take_prefix(&mut self) -> Vec<Key> {
        std::mem::take(&mut self.prefix)
    }

    fn resolve(&mut self, key: Key) -> MappingStep {
        self.prefix.push(key);
        if let Some(mapping) = self
            .mappings
            .iter()
            .find(|mapping| mapping.keys() == self.prefix.as_slice())
        {
            let rhs = mapping.rhs().to_vec();
            self.prefix.clear();
            return MappingStep::Keys(rhs);
        }
        if self
            .mappings
            .iter()
            .any(|mapping| mapping.keys().starts_with(&self.prefix))
        {
            return MappingStep::Wait;
        }
        MappingStep::Fallback(self.take_prefix())
    }
}
