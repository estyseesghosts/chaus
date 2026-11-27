//! Explicit Format Document action.
//!
//! Detection and process execution live in `chaus-format`. At most one job
//! owns a document: text expansion and the bounded diff run on the background
//! executor, while the editor applies a revision-checked, single undo step.

use chaus_core::diff::TextChange;
use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, DocumentRevision, Selection};
use chaus_format::{run, FormatError};
use gpui::{Context, Window};

use crate::editor::Editor;
use crate::editor_edit::normalize_newlines;
use crate::editor_keys::FormatDocument;

/// Computed off the UI thread from the snapshot the job started with.
type FormatPlan = Result<Option<TextChange>, FormatError>;

impl Editor {
    pub(crate) fn format_document_action(
        &mut self,
        _: &FormatDocument,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format_document(cx);
    }

    /// Run the formatter detected for the current file, if any.
    pub fn format_document(&mut self, cx: &mut Context<Self>) {
        if self.format.in_flight() {
            self.set_status("Format already in progress");
            cx.notify();
            return;
        }
        let Some(file) = self.file.state() else {
            self.set_status("Save the file before formatting");
            cx.notify();
            return;
        };
        let path = file.meta.user_requested_path.clone();
        let formatter = self.format.formatter_for(&path);
        let Some(formatter) = formatter else {
            self.set_status("No formatter is configured for this file type");
            cx.notify();
            return;
        };
        let program = formatter.program.clone();
        let snapshot = self.document.snapshot();
        let revision = self.document.revision();
        let generation = self.format.begin();
        self.set_status(format!("Formatting with {program}..."));
        cx.notify();

        let task = cx.background_executor().spawn(async move {
            let text = snapshot.text_with_eol();
            let formatted = run(&formatter, &text)?;
            let formatted = normalize_newlines(&formatted);
            Ok(chaus_core::diff::common_change(&text, &formatted))
        });
        cx.spawn(async move |editor, cx| {
            let result: FormatPlan = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_format(generation, revision, program, result, cx);
                })
                .ok();
        })
        .detach();
    }

    /// A replacement of the document, an explicit cancellation or a new job
    /// invalidates the previous generation.
    pub(crate) fn cancel_format_job(&mut self) {
        self.format.cancel();
    }

    fn finish_format(
        &mut self,
        generation: u64,
        revision: DocumentRevision,
        program: String,
        result: FormatPlan,
        cx: &mut Context<Self>,
    ) {
        if !self.format.finish(generation) {
            return;
        }
        if self.document.revision() != revision {
            self.set_status("Document changed while formatting; result discarded");
            cx.notify();
            return;
        }
        match result {
            Ok(change) => {
                if self.apply_format_change(revision, change, cx) {
                    self.flash_status(format!("Formatted with {program}"), cx);
                    cx.notify();
                }
            }
            Err(error) => {
                self.set_status(format!("Format failed: {error}"));
                cx.notify();
            }
        }
    }

    /// Apply formatter output only when the document is still at `revision`.
    /// Tests use this entry point with already-normalized text.
    pub fn apply_formatted_text(
        &mut self,
        revision: DocumentRevision,
        formatted: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.document.revision() != revision {
            self.set_status("Document changed while formatting; result discarded");
            cx.notify();
            return false;
        }
        let change = chaus_core::diff::common_change(&self.document.text(), formatted);
        self.apply_format_change(revision, change, cx)
    }

    fn apply_format_change(
        &mut self,
        revision: DocumentRevision,
        change: Option<TextChange>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.document.revision() != revision {
            self.set_status("Document changed while formatting; result discarded");
            cx.notify();
            return false;
        }
        let Some(mut change) = change else {
            self.flash_status("Already formatted", cx);
            cx.notify();
            return false;
        };
        // Never start the edit inside a grapheme cluster. The common prefix is
        // identical in both texts, so widening it by whole characters and
        // re-inserting them keeps the output byte-for-byte the same while
        // keeping the caret and invalidation on a cluster boundary.
        let boundary = self.document.prev_grapheme_boundary(change.old.start);
        if boundary < change.old.start {
            let keep = change.old.start.0 - boundary.0;
            let mut prefix = String::with_capacity(change.insert.len() + keep);
            prefix.push_str(
                &self
                    .document
                    .slice(CharRange::new(boundary, change.old.start)),
            );
            prefix.push_str(&change.insert);
            change.insert = prefix;
            change.old.start = boundary;
        }

        let before = self.selection;
        let old_len = self.document.len_chars();
        let new_len = change.new_len(old_len);
        let after = remap_selection(before, change.old, change.inserted_len(), new_len);
        let transaction = EditTransaction::new(
            EditCause::Formatter,
            TextEdit::new(change.old, change.insert),
            before,
            after,
        );
        if let Ok(applied) = self.document.apply(transaction) {
            self.selection = after;
            self.after_change(&applied, cx);
            cx.notify();
            true
        } else {
            self.set_status("Format failed: the document rejected the replacement");
            cx.notify();
            false
        }
    }
}

/// Keep the selection anchored to the text it was on when only a middle span
/// changes: offsets before the span stay put, offsets after it move by the
/// length delta, and offsets inside it land at the end of the replacement.
fn remap_selection(
    before: Selection,
    replaced: CharRange,
    inserted: usize,
    new_len: usize,
) -> Selection {
    let map = |offset: CharOffset| {
        let mapped = if offset <= replaced.start {
            offset
        } else if offset >= replaced.end {
            CharOffset(offset.0 + inserted - replaced.len())
        } else {
            CharOffset(replaced.start.0 + inserted)
        };
        CharOffset(mapped.0.min(new_len))
    };
    Selection::new(map(before.anchor), map(before.head))
}
