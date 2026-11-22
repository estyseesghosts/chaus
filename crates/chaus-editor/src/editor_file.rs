//! Per-window file session: save, save-as, dirty close policy.

use gpui::{Context, Window};

use crate::editor::Editor;
use crate::editor_keys::{Save, SaveAs};

/// The file the document was loaded from and its conflict baseline.
#[non_exhaustive]
#[derive(Debug)]
pub struct FileState {
    pub meta: chaus_io::FileMeta,
    pub had_utf8_bom: bool,
}

impl Editor {
    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_none() {
            self.save_as(&SaveAs, window, cx);
            return;
        }
        self.save_inner(false, cx);
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, _: &mut Window, cx: &mut Context<Self>) {
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
        let had_utf8_bom = self.file.as_ref().is_some_and(|file| file.had_utf8_bom);
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
                self.file = Some(FileState { meta, had_utf8_bom });
                self.document.mark_saved();
                self.close_confirmed = false;
                self.set_status("Saved");
            }
            Err(error) => self.set_status(format!("Save failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn save_inner(&mut self, force: bool, cx: &mut Context<Self>) -> bool {
        let Some(file) = self.file.as_ref() else {
            self.set_status("Untitled document: use Save As");
            cx.notify();
            return false;
        };
        let text = self.document.text_with_eol();
        let bytes = chaus_io::encode_for_save(&text, file.had_utf8_bom);
        let target = file.meta.canonical_target.clone();
        let requested = file.meta.user_requested_path.clone();
        let baseline = (!force).then(|| file.meta.clone());
        let result = chaus_io::save(chaus_io::SaveRequest::new(
            &target,
            &requested,
            &bytes,
            file.had_utf8_bom,
            self.document.preferred_eol(),
            baseline.as_ref(),
        ));
        match result {
            Ok(meta) => {
                if let Some(file) = self.file.as_mut() {
                    file.meta = meta;
                }
                self.document.mark_saved();
                self.close_confirmed = false;
                self.set_status("Saved");
                cx.notify();
                true
            }
            Err(chaus_io::SaveError::Conflict(conflict)) => {
                let message = match conflict {
                    chaus_io::SaveConflict::ChangedOnDisk => {
                        "File changed on disk. Save again to overwrite."
                    }
                    chaus_io::SaveConflict::DeletedOnDisk => {
                        "File was deleted on disk. Save again to recreate it."
                    }
                    _ => "File changed on disk. Save again to overwrite.",
                };
                self.set_status(message);
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
