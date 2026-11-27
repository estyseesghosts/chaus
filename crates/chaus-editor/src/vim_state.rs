//! Modal editing state for one editor window.
//!
//! `VimState` owns the per-window live Vim mode, the modal engine and the
//! generation that retires a mapping timeout. It does not own the document or
//! selection: callers pass those explicitly to engine operations, and `Editor`
//! performs the shared post-mutation bookkeeping. Settings-default Vim mode is
//! deliberately not stored here.

use chaus_core::document::Document;
use chaus_core::position::Selection;
use chaus_vim::{Engine, Key, Mode, Outcome};

/// Live mode, modal engine and mapping timeout generation.
pub(crate) struct VimState {
    enabled: bool,
    engine: Engine,
    mapping_epoch: u64,
}

impl VimState {
    pub(crate) fn new(mappings: Vec<chaus_vim::Mapping>, enabled: bool) -> Self {
        Self {
            enabled,
            engine: Engine::new(mappings),
            mapping_epoch: 0,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn mode(&self) -> Mode {
        self.engine.mode()
    }

    pub(crate) fn accepts_text(&self) -> bool {
        self.enabled && self.engine.mode().accepts_text()
    }

    pub(crate) fn command_line_active(&self) -> bool {
        self.enabled && self.engine.mode() == Mode::CommandLine
    }

    pub(crate) fn visual_mode(&self) -> bool {
        matches!(self.engine.mode(), Mode::VisualChar | Mode::VisualLine)
    }

    pub(crate) fn mode_label(&self) -> Option<&'static str> {
        self.enabled.then(|| self.engine.mode().label())
    }

    pub(crate) fn reset(&mut self) {
        self.engine.reset();
    }

    pub(crate) fn cancel_prefix(&mut self) {
        self.engine.cancel_prefix();
    }

    pub(crate) fn mapping_prefix_pending(&self) -> bool {
        self.engine.mapping_prefix_pending()
    }

    pub(crate) fn handle_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        self.engine.handle_key(key, document, selection)
    }

    pub(crate) fn resolve_mapping_timeout(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        self.engine.resolve_mapping_timeout(document, selection)
    }

    pub(crate) fn command_line(&self) -> Option<chaus_vim::CommandLineView<'_>> {
        self.engine.command_line()
    }

    pub(crate) fn set_command_preedit(&mut self, text: Option<String>) {
        self.engine.set_command_preedit(text);
    }

    pub(crate) fn insert_command_text(&mut self, text: &str) -> Outcome {
        self.engine.insert_command_text(text)
    }

    pub(crate) fn completion_request(&mut self) -> Option<chaus_vim::CompletionRequest> {
        self.engine.completion_request()
    }

    pub(crate) fn is_completion_current(&self, token: &chaus_vim::CompletionToken) -> bool {
        self.engine.is_completion_current(token)
    }

    pub(crate) fn apply_completion(
        &mut self,
        token: &chaus_vim::CompletionToken,
        completed: &str,
        finished: bool,
    ) -> Option<String> {
        self.engine.apply_completion(token, completed, finished)
    }

    /// Start a new mapping timeout. The returned epoch invalidates an older
    /// timer when another prefix arrives.
    pub(crate) fn begin_mapping_timeout(&mut self) -> u64 {
        self.mapping_epoch = self.mapping_epoch.wrapping_add(1);
        self.mapping_epoch
    }

    pub(crate) fn is_current_timeout(&self, epoch: u64) -> bool {
        self.mapping_epoch == epoch
    }
}
