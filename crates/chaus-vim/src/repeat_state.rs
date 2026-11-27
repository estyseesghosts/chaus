//! Dot repeat and the Insert-mode session that feeds it.

use chaus_core::position::CharOffset;

use crate::key::Key;

/// How `.` should replay the last change: the keys that produced it,
/// including any count digits, followed by the text a change command inserted.
#[derive(Clone, Debug)]
pub(crate) struct LastChange {
    keys: Vec<Key>,
    inserted: Option<String>,
}

impl LastChange {
    pub(crate) fn into_parts(self) -> (Vec<Key>, Option<String>) {
        (self.keys, self.inserted)
    }
}

pub(crate) struct RepeatState {
    last_change: Option<LastChange>,
    insert_start: Option<CharOffset>,
    insert_keys: Option<Vec<Key>>,
    insert_always_records: bool,
    insert_repeat: usize,
    replaying: bool,
}

impl Default for RepeatState {
    fn default() -> Self {
        Self {
            last_change: None,
            insert_start: None,
            insert_keys: None,
            insert_always_records: false,
            insert_repeat: 1,
            replaying: false,
        }
    }
}

impl RepeatState {
    /// Forget an unfinished Insert session and replay flag, but keep the last
    /// change: toggling Vim off must not erase `.`.
    pub(crate) fn clear(&mut self) {
        self.insert_start = None;
        self.insert_keys = None;
        self.insert_always_records = false;
        self.replaying = false;
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
