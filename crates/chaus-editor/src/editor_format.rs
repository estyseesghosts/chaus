//! Explicit Format Document action.
//!
//! Detection and process execution live in `chaus_io::format`; this module
//! only snapshots the document, dispatches the work off the UI thread, rejects
//! results for a stale revision and applies the replacement as one undo step.

use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, DocumentRevision};
use gpui::{Context, Window};

use crate::editor::Editor;
use crate::editor_edit::normalize_newlines;
use crate::editor_keys::FormatDocument;

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
        let Some(file) = self.file.as_ref() else {
            self.set_status("Save the file before formatting");
            cx.notify();
            return;
        };
        let path = file.meta.user_requested_path.clone();
        let Some(formatter) = chaus_io::format::Formatter::for_path(&path) else {
            self.set_status("No formatter is configured for this file type");
            cx.notify();
            return;
        };
        let program = formatter.program.clone();
        let source = self.document.text();
        let revision = self.document.revision();
        self.set_status(format!("Formatting with {program}..."));
        cx.notify();

        let task = cx
            .background_executor()
            .spawn(async move { chaus_io::format::run(&formatter, &source) });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_format(revision, program, result, cx);
                })
                .ok();
        })
        .detach();
    }

    fn finish_format(
        &mut self,
        revision: DocumentRevision,
        program: String,
        result: Result<String, chaus_io::format::FormatError>,
        cx: &mut Context<Self>,
    ) {
        if self.document.revision() != revision {
            self.set_status("Document changed while formatting; result discarded");
            cx.notify();
            return;
        }
        let formatted = match result {
            Ok(text) => text,
            Err(error) => {
                self.set_status(format!("Format failed: {error}"));
                cx.notify();
                return;
            }
        };
        let formatted = normalize_newlines(&formatted);
        if self.apply_formatted_text(revision, &formatted, cx) {
            self.flash_status(format!("Formatted with {program}"), cx);
            cx.notify();
        }
    }

    /// Apply formatter output only when the document is still at `revision`.
    ///
    /// Returns `true` when the text was applied. A stale result is reported and
    /// discarded; the document and its undo history are left untouched.
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
        if formatted == self.document.text() {
            self.flash_status("Already formatted", cx);
            cx.notify();
            return false;
        }
        let before = self.selection;
        let len = self.document.len_chars();
        let range = CharRange::new(CharOffset::ZERO, CharOffset(len));
        let after = before.clamp(formatted.chars().count());
        let transaction = EditTransaction::new(
            EditCause::Formatter,
            TextEdit::new(range, formatted.to_owned()),
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
