//! Background syntax highlighting for one editor window.
//!
//! This adapter translates editor-level events (language choice, path change,
//! edits) into calls on `HighlightState`, which owns generations, worker
//! ownership and publication decisions. This adapter owns the GPUI worker task,
//! language detection from the current file, error reporting and notification.

use std::time::Duration;

use chaus_core::position::DocumentRevision;
use chaus_highlight::{parse_snapshot, InputEdit, LanguageId, Span};
use gpui::Context;

pub use crate::highlight_state::HighlightStatistics;

use crate::editor::Editor;
use crate::highlight_state::{HighlightJob, HighlightOutcome};

/// Coalesce a burst of keystrokes into one parse. The delay is deliberately
/// not a substitute for cancelling obsolete work: a worker claims only the
/// newest generation when the timer fires.
const HIGHLIGHT_DEBOUNCE: Duration = Duration::from_millis(30);

impl Editor {
    /// Language currently used for colour.
    #[must_use]
    pub fn language(&self) -> LanguageId {
        self.highlight.language()
    }

    /// Whether the language was chosen explicitly rather than detected.
    #[must_use]
    pub fn language_is_override(&self) -> bool {
        self.highlight.language_is_override()
    }

    /// The language detected for the current file, or plain text when untitled.
    fn detected_language(&self) -> LanguageId {
        self.file
            .path()
            .map(LanguageId::from_path)
            .unwrap_or(LanguageId::PlainText)
    }

    /// Choose a language for this window. The choice survives Save As and
    /// open/save until `clear_language_override` is used.
    pub fn set_language(&mut self, language: LanguageId, cx: &mut Context<Self>) {
        if self.highlight.choose_language(language) {
            self.schedule_highlight(None, cx);
        }
        cx.notify();
    }

    /// Return to extension/name detection for this window.
    pub fn clear_language_override(&mut self, cx: &mut Context<Self>) {
        if self.highlight.clear_override() {
            self.reset_highlight(cx);
        }
    }

    /// Re-detect after a path change and start from a fresh tree.
    pub(crate) fn reset_highlight(&mut self, cx: &mut Context<Self>) {
        let language = self.detected_language();
        self.highlight.reset_language(language);
        self.schedule_highlight(None, cx);
    }

    /// Refresh detection after Save As without resetting the document.
    pub(crate) fn refresh_language(&mut self, cx: &mut Context<Self>) {
        let language = self.detected_language();
        if self.highlight.refresh_language(language) {
            self.schedule_highlight(None, cx);
        }
    }

    /// Safe spans for the current revision, for tests and diagnostics.
    #[must_use]
    pub fn syntax_spans(&self) -> &[Span] {
        self.highlight_spans()
    }

    /// Revision the stored spans belong to, even when they are not paintable.
    #[must_use]
    pub fn syntax_revision(&self) -> Option<DocumentRevision> {
        self.highlight.syntax_revision()
    }

    /// Spans that are safe to paint for the current document revision.
    #[must_use]
    pub(crate) fn highlight_spans(&self) -> &[Span] {
        self.highlight.spans(self.document.revision())
    }

    /// Parse counters for this window, for diagnostics and tests.
    #[must_use]
    pub fn highlight_statistics(&self) -> HighlightStatistics {
        self.highlight.statistics()
    }

    /// Generation of the last accepted parse, used by the paint cache key.
    #[must_use]
    pub(crate) fn highlight_applied_generation(&self) -> u64 {
        self.highlight.applied_generation()
    }

    /// Schedule a revision-tagged parse on the window's single worker loop.
    ///
    /// `edit` is `Some` for a single-edit transaction, letting Tree-sitter
    /// reuse the previous tree only when that tree is exactly one revision
    /// behind. `HighlightState` records the request; this adapter owns the GPUI
    /// task, waits for the debounce and claims only the newest generation.
    pub(crate) fn schedule_highlight(&mut self, edit: Option<InputEdit>, cx: &mut Context<Self>) {
        if !self.highlight.begin(&self.document, edit) {
            return;
        }
        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(HIGHLIGHT_DEBOUNCE).await;
            let job =
                match editor.update(cx, |editor, _| editor.highlight.claim_job(&editor.document)) {
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
            let outcome = editor
                .update(cx, |editor, cx| {
                    let current_revision = editor.document.revision();
                    let outcome = editor.highlight.finish_job(
                        generation,
                        revision,
                        language,
                        parsed,
                        current_revision,
                    );
                    if let Some(message) = &outcome.error {
                        editor.set_status(message.clone());
                    }
                    cx.notify();
                    outcome
                })
                .unwrap_or(HighlightOutcome {
                    error: None,
                    more: false,
                });
            if !outcome.more {
                return;
            }
        })
        .detach();
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
