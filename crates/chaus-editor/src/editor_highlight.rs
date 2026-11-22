//! Background syntax highlighting for one editor window.

use std::sync::Arc;
use std::time::Duration;

use chaus_core::position::DocumentRevision;
use chaus_highlight::{
    parse_snapshot, HighlightError, InputEdit, LanguageId, ParsedHighlight, Span, Tree,
};
use gpui::Context;
use ropey::Rope;

use crate::editor::Editor;

/// Coalesce a burst of keystrokes into one parse. The delay is deliberately
/// not a substitute for cancelling obsolete work: a worker claims only the
/// newest generation when the timer fires.
const HIGHLIGHT_DEBOUNCE: Duration = Duration::from_millis(30);

/// One scheduled edit and the revision its `InputEdit` applies to.
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

struct HighlightJob {
    generation: u64,
    revision: DocumentRevision,
    language: LanguageId,
    edit: Option<InputEdit>,
    tree: Option<Tree>,
    rope: Rope,
}

impl Editor {
    /// Language currently used for colour.
    #[must_use]
    pub fn language(&self) -> LanguageId {
        self.language
    }

    /// Whether the language was chosen explicitly rather than detected.
    #[must_use]
    pub fn language_is_override(&self) -> bool {
        self.language_override
    }

    /// Choose a language for this window. The choice survives Save As and
    /// open/save until `clear_language_override` is used.
    pub fn set_language(&mut self, language: LanguageId, cx: &mut Context<Self>) {
        self.language_override = true;
        if self.language == language {
            return;
        }
        self.language = language;
        self.highlight_tree = None;
        self.highlight_tree_revision = None;
        self.highlights = Arc::new(Vec::new());
        self.highlights_revision = None;
        self.schedule_highlight(None, cx);
        cx.notify();
    }

    /// Return to extension/name detection for this window.
    pub fn clear_language_override(&mut self, cx: &mut Context<Self>) {
        if !self.language_override {
            return;
        }
        self.language_override = false;
        self.reset_highlight(cx);
    }

    /// Re-detect after a path change and start from a fresh tree.
    pub(crate) fn reset_highlight(&mut self, cx: &mut Context<Self>) {
        if !self.language_override {
            self.language = self
                .file
                .as_ref()
                .map(|file| LanguageId::from_path(&file.meta.user_requested_path))
                .unwrap_or(LanguageId::PlainText);
        }
        self.highlight_tree = None;
        self.highlight_tree_revision = None;
        self.highlights = Arc::new(Vec::new());
        self.highlights_revision = None;
        self.schedule_highlight(None, cx);
    }

    /// Refresh detection after Save As without resetting the document.
    pub(crate) fn refresh_language(&mut self, cx: &mut Context<Self>) {
        if self.language_override {
            return;
        }
        let language = self
            .file
            .as_ref()
            .map(|file| LanguageId::from_path(&file.meta.user_requested_path))
            .unwrap_or(LanguageId::PlainText);
        if language == self.language {
            return;
        }
        self.language = language;
        self.highlight_tree = None;
        self.highlight_tree_revision = None;
        self.highlights = Arc::new(Vec::new());
        self.highlights_revision = None;
        self.schedule_highlight(None, cx);
    }

    /// Safe spans for the current revision, for tests and diagnostics.
    #[must_use]
    pub fn syntax_spans(&self) -> &[Span] {
        self.highlight_spans()
    }

    /// Revision the stored spans belong to, even when they are not paintable.
    #[must_use]
    pub fn syntax_revision(&self) -> Option<DocumentRevision> {
        self.highlights_revision
    }

    /// Spans that are safe to paint for the current document revision.
    #[must_use]
    pub(crate) fn highlight_spans(&self) -> &[Span] {
        if self.highlights_revision == Some(self.document.revision()) {
            &self.highlights
        } else {
            &[]
        }
    }

    /// Parse counters for this window, for diagnostics and tests.
    #[must_use]
    pub fn highlight_statistics(&self) -> HighlightStatistics {
        self.highlight_statistics
    }

    /// Schedule a revision-tagged parse on the window's single worker loop.
    ///
    /// `edit` is `Some` for a single-edit transaction, letting Tree-sitter
    /// reuse the previous tree only when that tree is exactly one revision
    /// behind. Multi-edit transactions, undo/redo and language changes pass
    /// `None` and reparse. The worker waits for the debounce, then claims the
    /// newest generation; superseded generations never start a parse. The
    /// accepted tree stays in place until a result is published, so a stale job
    /// cannot destroy the incrementality of the next one.
    pub(crate) fn schedule_highlight(&mut self, edit: Option<InputEdit>, cx: &mut Context<Self>) {
        self.highlight_generation = self.highlight_generation.wrapping_add(1);
        self.highlight_statistics.scheduled = self.highlight_statistics.scheduled.saturating_add(1);
        let revision = self.document.revision();
        let language = self.language;
        self.highlight_pending_edit = edit.map(|edit| PendingHighlightEdit {
            edit,
            base_revision: DocumentRevision(revision.0.wrapping_sub(1)),
        });

        if language == LanguageId::PlainText {
            self.highlight_tree = None;
            self.highlight_tree_revision = None;
            self.highlight_pending_edit = None;
            self.highlights = Arc::new(Vec::new());
            self.highlights_revision = Some(revision);
            self.highlight_applied_generation = self.highlight_generation;
            return;
        }
        if self.highlight_worker_running {
            return;
        }
        self.highlight_worker_running = true;
        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(HIGHLIGHT_DEBOUNCE).await;
            let job = match editor.update(cx, |editor, _| editor.claim_highlight_job()) {
                Ok(job) => job,
                Err(_) => return,
            };
            let Some(job) = job else {
                return;
            };
            let HighlightJob {
                generation,
                revision,
                language,
                edit,
                tree,
                rope,
            } = job;
            let parsed = cx
                .background_executor()
                .spawn(async move { parse_snapshot(language, &rope, tree, edit, revision) })
                .await;
            let more = editor
                .update(cx, |editor, cx| {
                    editor.finish_highlight_job(generation, revision, language, parsed, cx)
                })
                .unwrap_or(false);
            if !more {
                return;
            }
        })
        .detach();
    }

    /// Claim the newest pending generation after the debounce.
    ///
    /// Returning `None` clears the worker flag in the same update, so a
    /// schedule that runs afterwards starts a new worker instead of waiting
    /// forever on a stale flag.
    fn claim_highlight_job(&mut self) -> Option<HighlightJob> {
        if self.highlight_generation == self.highlight_applied_generation {
            self.highlight_worker_running = false;
            return None;
        }
        let generation = self.highlight_generation;
        let revision = self.document.revision();
        let language = self.language;
        let skipped = generation
            .wrapping_sub(self.highlight_applied_generation)
            .saturating_sub(1);
        self.highlight_statistics.coalesced =
            self.highlight_statistics.coalesced.saturating_add(skipped);
        self.highlight_applied_generation = generation;

        if language == LanguageId::PlainText {
            self.highlight_tree = None;
            self.highlight_tree_revision = None;
            self.highlight_pending_edit = None;
            self.highlights = Arc::new(Vec::new());
            self.highlights_revision = Some(revision);
            self.highlight_worker_running = false;
            return None;
        }

        let pending = self.highlight_pending_edit.take();
        let (edit, tree) = match pending {
            Some(pending)
                if self.highlight_tree_revision == Some(pending.base_revision)
                    && self.highlight_tree.is_some() =>
            {
                (Some(pending.edit), self.highlight_tree.clone())
            }
            _ => (None, None),
        };
        if edit.is_some() {
            self.highlight_statistics.incremental_parses = self
                .highlight_statistics
                .incremental_parses
                .saturating_add(1);
        } else {
            self.highlight_statistics.full_parses =
                self.highlight_statistics.full_parses.saturating_add(1);
        }
        Some(HighlightJob {
            generation,
            revision,
            language,
            edit,
            tree,
            rope: self.document.rope().clone(),
        })
    }

    /// Publish a parse only when it still describes the live document.
    ///
    /// Returns `true` when the worker should wait for another generation.
    fn finish_highlight_job(
        &mut self,
        generation: u64,
        revision: DocumentRevision,
        language: LanguageId,
        parsed: Result<ParsedHighlight, HighlightError>,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.highlight_generation == generation
            && self.document.revision() == revision
            && self.language == language;
        if current {
            match parsed {
                Ok(parsed) => {
                    self.highlight_tree = parsed.tree;
                    self.highlight_tree_revision = Some(revision);
                    self.highlights = parsed.highlight.spans;
                }
                Err(error) => {
                    self.highlight_tree = None;
                    self.highlight_tree_revision = None;
                    self.highlights = Arc::new(Vec::new());
                    self.set_status(format!(
                        "Highlighting off for {}: {error}",
                        language.label()
                    ));
                }
            }
            self.highlights_revision = Some(revision);
            self.highlight_statistics.published =
                self.highlight_statistics.published.saturating_add(1);
            cx.notify();
        } else {
            // Keep the accepted tree and spans: they are still the newest
            // complete result, and the next job will reparse from the latest
            // revision rather than pretending this tree is current.
            self.highlight_statistics.discarded =
                self.highlight_statistics.discarded.saturating_add(1);
        }
        if self.highlight_generation == self.highlight_applied_generation {
            self.highlight_worker_running = false;
            false
        } else {
            true
        }
    }

    /// Apply a completed edit and schedule the next parse.
    pub(crate) fn after_highlight_edit(
        &mut self,
        result: &chaus_core::document::EditResult,
        cx: &mut Context<Self>,
    ) {
        let edit = chaus_highlight::edit_for_transaction(&result.edits);
        self.schedule_highlight(edit, cx);
    }
}
