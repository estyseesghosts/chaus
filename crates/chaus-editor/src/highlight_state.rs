//! Syntax highlighting state for one editor window.
//!
//! `HighlightState` owns the detected/chosen language, accepted spans and tree
//! with their revisions, parse generations, the single worker flag, a pending
//! incremental edit and parse statistics. It owns the single-worker flag and
//! generation bookkeeping, but not the GPUI task, document, status messages or
//! notification policy: the adapter owns the task and passes explicit inputs.

use std::sync::Arc;

use chaus_core::document::Document;
use chaus_core::position::DocumentRevision;
use chaus_highlight::{HighlightError, InputEdit, LanguageId, ParsedHighlight, Span, Tree};
use ropey::Rope;

/// One scheduled edit and the revision its `InputEdit` applies to.
#[derive(Clone, Copy)]
pub(crate) struct PendingHighlightEdit {
    pub(crate) edit: InputEdit,
    pub(crate) base_revision: DocumentRevision,
}

/// Parse counters for diagnostics and regression tests.
///
/// A job is "scheduled" per edit. `coalesced` counts generations superseded
/// before any parse started; `discarded` counts results that finished after
/// the document, language or generation moved on. At most one parse per window
/// is ever in flight because a single worker loop consumes the latest job.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighlightStatistics {
    pub scheduled: u64,
    pub coalesced: u64,
    pub incremental_parses: u64,
    pub full_parses: u64,
    pub published: u64,
    pub discarded: u64,
}

/// One claimed parse job, passed to the background executor by the adapter.
pub(crate) struct HighlightJob {
    pub(crate) generation: u64,
    pub(crate) revision: DocumentRevision,
    pub(crate) language: LanguageId,
    pub(crate) edit: Option<InputEdit>,
    pub(crate) tree: Option<Tree>,
    pub(crate) rope: Rope,
}

/// Result of a completed parse, for `Editor` to publish into the UI.
pub(crate) struct HighlightOutcome {
    /// A message to show when highlighting was disabled by an error.
    pub(crate) error: Option<String>,
    /// Whether the worker must wait for another generation.
    pub(crate) more: bool,
}

/// All highlighting bookkeeping for one editor window.
pub(crate) struct HighlightState {
    language: LanguageId,
    language_override: bool,
    highlights: Arc<Vec<Span>>,
    highlights_revision: Option<DocumentRevision>,
    tree: Option<Tree>,
    tree_revision: Option<DocumentRevision>,
    generation: u64,
    applied_generation: u64,
    worker_running: bool,
    pending_edit: Option<PendingHighlightEdit>,
    statistics: HighlightStatistics,
}

impl Default for HighlightState {
    fn default() -> Self {
        Self::new()
    }
}

impl HighlightState {
    pub(crate) fn new() -> Self {
        Self {
            language: LanguageId::PlainText,
            language_override: false,
            highlights: Arc::new(Vec::new()),
            highlights_revision: None,
            tree: None,
            tree_revision: None,
            generation: 0,
            applied_generation: 0,
            worker_running: false,
            pending_edit: None,
            statistics: HighlightStatistics::default(),
        }
    }

    pub(crate) fn language(&self) -> LanguageId {
        self.language
    }

    pub(crate) fn language_is_override(&self) -> bool {
        self.language_override
    }

    pub(crate) fn applied_generation(&self) -> u64 {
        self.applied_generation
    }

    pub(crate) fn statistics(&self) -> HighlightStatistics {
        self.statistics
    }

    pub(crate) fn syntax_revision(&self) -> Option<DocumentRevision> {
        self.highlights_revision
    }

    /// Spans that are safe to paint for `revision`.
    pub(crate) fn spans(&self, revision: DocumentRevision) -> &[Span] {
        if self.highlights_revision == Some(revision) {
            &self.highlights
        } else {
            &[]
        }
    }

    /// Explicit language choice. Returns `true` when the choice changed the
    /// language and stale highlighting must be discarded.
    pub(crate) fn choose_language(&mut self, language: LanguageId) -> bool {
        self.language_override = true;
        if self.language == language {
            return false;
        }
        self.set_language(language);
        true
    }

    pub(crate) fn clear_override(&mut self) -> bool {
        if !self.language_override {
            return false;
        }
        self.language_override = false;
        true
    }

    /// Re-detect after a path change unless the person chose a language.
    pub(crate) fn reset_language(&mut self, detected: LanguageId) {
        if !self.language_override {
            self.language = detected;
        }
        self.clear_stale();
    }

    /// Refresh detection after Save As. Returns `true` when the language
    /// changed and a fresh parse is needed.
    pub(crate) fn refresh_language(&mut self, detected: LanguageId) -> bool {
        if self.language_override || detected == self.language {
            return false;
        }
        self.set_language(detected);
        true
    }

    fn set_language(&mut self, language: LanguageId) {
        self.language = language;
        self.clear_stale();
    }

    fn clear_stale(&mut self) {
        self.tree = None;
        self.tree_revision = None;
        self.highlights = Arc::new(Vec::new());
        self.highlights_revision = None;
        self.pending_edit = None;
    }

    /// Record a new parse request. `edit` is `Some` for a single-edit
    /// transaction, letting Tree-sitter reuse the previous tree only when that
    /// tree is exactly one revision behind. Returns `true` when the adapter
    /// must start a worker; superseded generations never start a parse.
    pub(crate) fn begin(&mut self, document: &Document, edit: Option<InputEdit>) -> bool {
        self.generation = self.generation.wrapping_add(1);
        self.statistics.scheduled = self.statistics.scheduled.saturating_add(1);
        let revision = document.revision();
        self.pending_edit = edit.map(|edit| PendingHighlightEdit {
            edit,
            base_revision: DocumentRevision(revision.0.wrapping_sub(1)),
        });

        if self.language == LanguageId::PlainText {
            self.clear_stale();
            self.highlights_revision = Some(revision);
            self.applied_generation = self.generation;
            return false;
        }
        if self.worker_running {
            return false;
        }
        self.worker_running = true;
        true
    }

    /// Claim the newest pending generation after the debounce.
    ///
    /// Returning `None` clears the worker flag in the same update, so a
    /// schedule that runs afterwards starts a new worker instead of waiting
    /// forever on a stale flag.
    pub(crate) fn claim_job(&mut self, document: &Document) -> Option<HighlightJob> {
        if self.generation == self.applied_generation {
            self.worker_running = false;
            return None;
        }
        let generation = self.generation;
        let revision = document.revision();
        let language = self.language;
        let skipped = generation
            .wrapping_sub(self.applied_generation)
            .saturating_sub(1);
        self.statistics.coalesced = self.statistics.coalesced.saturating_add(skipped);
        self.applied_generation = generation;

        if language == LanguageId::PlainText {
            self.clear_stale();
            self.highlights_revision = Some(revision);
            self.worker_running = false;
            return None;
        }

        let pending = self.pending_edit.take();
        let (edit, tree) = match pending {
            Some(pending)
                if self.tree_revision == Some(pending.base_revision) && self.tree.is_some() =>
            {
                (Some(pending.edit), self.tree.clone())
            }
            _ => (None, None),
        };
        if edit.is_some() {
            self.statistics.incremental_parses =
                self.statistics.incremental_parses.saturating_add(1);
        } else {
            self.statistics.full_parses = self.statistics.full_parses.saturating_add(1);
        }
        Some(HighlightJob {
            generation,
            revision,
            language,
            edit,
            tree,
            rope: document.rope().clone(),
        })
    }

    /// Publish a parse only when it still describes the live document.
    pub(crate) fn finish_job(
        &mut self,
        generation: u64,
        revision: DocumentRevision,
        language: LanguageId,
        parsed: Result<ParsedHighlight, HighlightError>,
        current_revision: DocumentRevision,
    ) -> HighlightOutcome {
        let current = self.generation == generation
            && current_revision == revision
            && self.language == language;
        let mut error = None;
        if current {
            match parsed {
                Ok(parsed) => {
                    self.tree = parsed.tree;
                    self.tree_revision = Some(revision);
                    self.highlights = parsed.highlight.spans;
                }
                Err(failure) => {
                    self.clear_stale();
                    error = Some(format!(
                        "Highlighting off for {}: {failure}",
                        language.label()
                    ));
                }
            }
            self.highlights_revision = Some(revision);
            self.statistics.published = self.statistics.published.saturating_add(1);
        } else {
            // Keep the accepted tree and spans: they are still the newest
            // complete result, and the next job will reparse from the latest
            // revision rather than pretending this tree is current.
            self.statistics.discarded = self.statistics.discarded.saturating_add(1);
        }
        let more = self.generation != self.applied_generation;
        if !more {
            self.worker_running = false;
        }
        HighlightOutcome { error, more }
    }
}
