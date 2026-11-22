//! Background syntax highlighting for one editor window.

use std::sync::Arc;
use std::time::Duration;

use chaus_core::position::DocumentRevision;
use chaus_highlight::{parse_snapshot, InputEdit, LanguageId, Span};
use gpui::Context;

use crate::editor::Editor;

/// Coalesce a burst of keystrokes into one parse.
const HIGHLIGHT_DEBOUNCE: Duration = Duration::from_millis(30);

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

    /// Schedule a revision-tagged parse on a background worker.
    ///
    /// `edit` is `Some` for a single-edit transaction, letting Tree-sitter
    /// reuse the previous tree; multi-edit transactions, undo/redo and language
    /// changes pass `None` and reparse. The result is discarded when the
    /// generation, revision or language no longer match.
    pub(crate) fn schedule_highlight(&mut self, edit: Option<InputEdit>, cx: &mut Context<Self>) {
        self.highlight_generation = self.highlight_generation.wrapping_add(1);
        let generation = self.highlight_generation;
        let revision = self.document.revision();
        let language = self.language;
        if language == LanguageId::PlainText {
            self.highlight_tree = None;
            self.highlights = Arc::new(Vec::new());
            self.highlights_revision = Some(revision);
            return;
        }
        let rope = self.document.rope().clone();
        let tree = self.highlight_tree.take();
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(HIGHLIGHT_DEBOUNCE).await;
            let parsed = cx
                .background_executor()
                .spawn(async move { parse_snapshot(language, &rope, tree, edit, revision) })
                .await;
            editor
                .update(cx, |editor, cx| {
                    if editor.highlight_generation != generation
                        || editor.document.revision() != revision
                        || editor.language != language
                    {
                        return;
                    }
                    match parsed {
                        Ok(parsed) => {
                            editor.highlight_tree = parsed.tree;
                            editor.highlights = parsed.highlight.spans;
                        }
                        Err(error) => {
                            editor.highlight_tree = None;
                            editor.highlights = Arc::new(Vec::new());
                            editor.set_status(format!(
                                "Highlighting off for {}: {error}",
                                language.label()
                            ));
                        }
                    }
                    editor.highlights_revision = Some(revision);
                    cx.notify();
                })
                .ok();
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
