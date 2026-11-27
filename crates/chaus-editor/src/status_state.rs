//! Corner status message for one editor window.
//!
//! `StatusState` owns the visible message and the epoch that retires old
//! transient timers. It does not own the notification channel, the text being
//! displayed elsewhere, or any file/recovery status: callers pass the message
//! they want shown and `Editor` decides when `cx.notify()` is needed.

/// Visible corner message and the epoch that stamps it.
pub(crate) struct StatusState {
    message: Option<String>,
    epoch: u64,
}

impl Default for StatusState {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusState {
    pub(crate) fn new() -> Self {
        Self {
            message: None,
            epoch: 0,
        }
    }

    /// Replace the visible message. Persistent and transient messages share
    /// the epoch so an old timer cannot clear a newer one.
    pub(crate) fn set(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(crate) fn clear(&mut self) {
        self.message = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(crate) fn text(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub(crate) fn is_message(&self, expected: &str) -> bool {
        self.message.as_deref() == Some(expected)
    }

    /// Clear only when `expected` is still the visible message. Used when a
    /// watcher warning is withdrawn; a newer save message must survive.
    pub(crate) fn clear_message(&mut self, expected: &str) -> bool {
        if self.is_message(expected) {
            self.clear();
            true
        } else {
            false
        }
    }

    /// Clear the transient message whose timer carried `epoch`. The epoch is
    /// deliberately not bumped: another timer with the same stamp may still be
    /// waiting, and bumping here would let it clear a later message.
    pub(crate) fn clear_if_epoch(&mut self, epoch: u64) -> bool {
        if self.epoch == epoch && self.message.is_some() {
            self.message = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
}
