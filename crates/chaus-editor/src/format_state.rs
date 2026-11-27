//! Formatter execution ownership for one editor window.
//!
//! `FormatState` owns the optional test formatter, whether a job is in flight
//! and the generation that validates its completion. It does not own the
//! document, the edit that applies formatter output, or the formatter process:
//! adapters pass the path and result, and `Editor` applies the change.

use std::path::Path;

use chaus_format::Formatter;

/// Formatter override, in-flight flag and generation.
pub(crate) struct FormatState {
    formatter_override: Option<Formatter>,
    in_flight: bool,
    generation: u64,
}

impl Default for FormatState {
    fn default() -> Self {
        Self::new()
    }
}

impl FormatState {
    pub(crate) fn new() -> Self {
        Self {
            formatter_override: None,
            in_flight: false,
            generation: 0,
        }
    }

    pub(crate) fn set_formatter(&mut self, formatter: Formatter) {
        self.formatter_override = Some(formatter);
    }

    /// Formatter to use for `path`: the test override wins over detection.
    pub(crate) fn formatter_for(&self, path: &Path) -> Option<Formatter> {
        self.formatter_override
            .clone()
            .or_else(|| Formatter::for_path(path))
    }

    pub(crate) fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// Claim ownership for a new job. The returned generation must accompany
    /// its result so a late completion can be rejected.
    pub(crate) fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = true;
        self.generation
    }

    /// Invalidate any completion from before a document replacement or an
    /// explicit cancellation.
    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = false;
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation && self.in_flight
    }

    /// Release ownership when a current job finishes.
    pub(crate) fn finish(&mut self, generation: u64) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.in_flight = false;
        true
    }
}
