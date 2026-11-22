//! Per-window file session: save, save-as, dirty close policy.

use std::path::Path;

use chaus_core::position::{CharOffset, Selection};
use chaus_io::FileMeta;
use gpui::{px, Context, Window};

use crate::editor::Editor;
use crate::editor_keys::{Save, SaveAs};

/// The file the document was loaded from and its conflict baseline.
#[non_exhaustive]
#[derive(Debug)]
pub struct FileState {
    pub meta: FileMeta,
}

impl Editor {
    /// Replace this window's document with a loaded file. Dirty documents are
    /// refused unless `force` is set (the `:e!` form).
    pub(crate) fn open_path(&mut self, path: &Path, force: bool, cx: &mut Context<Self>) -> bool {
        if self.document.is_dirty() && !force {
            self.set_status("No write since last change (add ! to override)");
            cx.notify();
            return false;
        }
        match chaus_io::load(path) {
            Ok(loaded) => {
                self.document
                    .reset(loaded.text, loaded.eol, loaded.preferred_eol);
                self.file = Some(FileState { meta: loaded.meta });
                self.selection = Selection::caret(CharOffset::ZERO);
                self.preferred_column = None;
                self.scroll_row = 0.0;
                self.scroll_x = px(0.);
                self.marked = None;
                self.preedit_start = None;
                self.preedit_active = false;
                self.close_confirmed = false;
                self.force_next_save = false;
                self.document.cancel_user_operation();
                self.vim_engine.reset();
                self.rebuild_rows();
                self.set_status(format!("Opened {}", path.display()));
                cx.notify();
                true
            }
            Err(error) => {
                self.set_status(format!("Open failed: {error}"));
                cx.notify();
                false
            }
        }
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_none() {
            self.save_as(&SaveAs, window, cx);
            return;
        }
        self.save_inner(cx);
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, _: &mut Window, cx: &mut Context<Self>) {
        if !chaus_dialog::is_available() {
            self.set_status("Save As needs a native file picker, which is not available yet");
            cx.notify();
            return;
        }
        // Choosing a new target abandons a pending conflict overwrite; a
        // cancelled picker must not leave the old force armed either.
        self.force_next_save = false;
        let suggested = self
            .file
            .as_ref()
            .map(|file| file.meta.user_requested_path.clone());
        let Some(path) =
            chaus_dialog::choose_file(chaus_dialog::Choice::Save, "Save As", suggested.as_deref())
        else {
            self.set_status("Save cancelled");
            cx.notify();
            return;
        };
        let had_utf8_bom = self
            .file
            .as_ref()
            .is_some_and(|file| file.meta.had_utf8_bom);
        let text = self.document.text_with_eol();
        let bytes = chaus_io::encode_for_save(&text, had_utf8_bom);
        let result = chaus_io::save(chaus_io::SaveRequest::new(
            &path,
            &path,
            &bytes,
            had_utf8_bom,
            self.document.preferred_eol(),
            None,
        ));
        match result {
            Ok(meta) => {
                self.file = Some(FileState { meta });
                self.document.mark_saved();
                self.close_confirmed = false;
                self.set_status("Saved");
            }
            Err(error) => self.set_status(format!("Save failed: {error}")),
        }
        cx.notify();
    }

    /// Save the document, or overwrite a known external change on the second
    /// attempt. Returns `true` only when the file was written.
    pub(crate) fn save_inner(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(file) = self.file.as_ref() else {
            self.set_status("Untitled document: use Save As");
            cx.notify();
            return false;
        };
        let text = self.document.text_with_eol();
        let bytes = chaus_io::encode_for_save(&text, file.meta.had_utf8_bom);
        let target = file.meta.canonical_target.clone();
        let requested = file.meta.user_requested_path.clone();
        let force = std::mem::take(&mut self.force_next_save);
        let baseline = (!force).then(|| file.meta.clone());
        let result = chaus_io::save(chaus_io::SaveRequest::new(
            &target,
            &requested,
            &bytes,
            file.meta.had_utf8_bom,
            self.document.preferred_eol(),
            baseline.as_ref(),
        ));
        match result {
            Ok(meta) => {
                if let Some(file) = self.file.as_mut() {
                    file.meta = meta;
                }
                self.force_next_save = false;
                self.document.mark_saved();
                self.close_confirmed = false;
                self.set_status("Saved");
                cx.notify();
                true
            }
            Err(chaus_io::SaveError::Conflict(conflict)) => {
                self.force_next_save = true;
                self.set_status(format!("{conflict}. Save again to overwrite."));
                cx.notify();
                false
            }
            Err(error) => {
                self.set_status(format!("Save failed: {error}"));
                cx.notify();
                false
            }
        }
    }

    /// Whether closing needs an explicit confirmation because of unsaved
    /// edits. The first attempt reports; a second attempt discards.
    #[must_use]
    pub fn confirm_close(&mut self) -> bool {
        if !self.document.is_dirty() || self.close_confirmed {
            return true;
        }
        self.close_confirmed = true;
        self.set_status("Unsaved changes: close again to discard");
        false
    }
}
