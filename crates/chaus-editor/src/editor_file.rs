//! Per-window file session: save, save-as, dirty close policy.

use std::path::{Path, PathBuf};

use chaus_core::position::{CharOffset, Selection};
use chaus_io::{FileMeta, LoadedFile, SaveError};
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
        if self.saving || self.opening {
            self.set_status("A file operation is already in progress");
            cx.notify();
            return false;
        }
        if self.document.is_dirty() && !force {
            self.set_status("No write since last change (add ! to override)");
            cx.notify();
            return false;
        }
        let state = self.document.state_token();
        let requested = path.to_path_buf();
        let load_path = requested.clone();
        self.opening = true;
        self.set_status(format!("Opening {}...", requested.display()));
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { chaus_io::load(&load_path) });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_open(&requested, force, state, result, cx);
                })
                .ok();
        })
        .detach();
        true
    }

    fn finish_open(
        &mut self,
        path: &Path,
        force: bool,
        state: u64,
        result: Result<LoadedFile, chaus_io::LoadError>,
        cx: &mut Context<Self>,
    ) {
        self.opening = false;
        if !force && (self.document.state_token() != state || self.document.is_dirty()) {
            self.set_status("Open cancelled: the document changed while loading");
            cx.notify();
            return;
        }
        match result {
            Ok(loaded) => {
                self.document
                    .reset(loaded.text, loaded.eol, loaded.preferred_eol);
                self.file = Some(FileState { meta: loaded.meta });
                self.reset_highlight(cx);
                self.selection = Selection::caret(CharOffset::ZERO);
                self.preferred_column = None;
                self.scroll_row = 0.0;
                self.scroll_x = px(0.);
                self.marked = None;
                self.preedit_start = None;
                self.preedit_active = false;
                self.force_next_save = false;
                self.disk_change = None;
                self.disk_epoch = self.disk_epoch.wrapping_add(1);
                self.remove_recovery_snapshot();
                self.reset_recovery_key();
                self.document.cancel_user_operation();
                self.vim_engine.reset();
                self.rebuild_rows();
                self.flash_status(format!("Opened {}", path.display()), cx);
            }
            Err(error) => self.set_status(format!("Open failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            self.set_status("Save already in progress");
            cx.notify();
            return;
        }
        if self.file.is_none() {
            self.save_as(&SaveAs, window, cx);
            return;
        }
        self.save_inner(false, cx);
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save_as_inner(false, window, cx);
    }

    pub(crate) fn save_as_inner(
        &mut self,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.saving {
            self.set_status("Save already in progress");
            if close_after {
                self.restore_close_prompt(window, cx);
            } else {
                cx.notify();
            }
            return false;
        }
        if self.save_picker.is_none() && !chaus_dialog::is_available() {
            self.set_status("Save As needs a native file picker, which is not available yet");
            if close_after {
                self.restore_close_prompt(window, cx);
            } else {
                cx.notify();
            }
            return false;
        }
        // Choosing a new target abandons a pending conflict overwrite; a
        // cancelled picker must not leave the old force armed either.
        self.force_next_save = false;
        let suggested = self
            .file
            .as_ref()
            .map(|file| file.meta.user_requested_path.clone());
        let picked = match self.save_picker {
            Some(picker) => picker(suggested.as_deref()),
            None => chaus_dialog::choose_file(
                chaus_dialog::Choice::Save,
                "Save As",
                suggested.as_deref(),
            ),
        };
        let Some(path) = picked else {
            self.set_status("Save cancelled");
            if close_after {
                self.restore_close_prompt(window, cx);
            } else {
                cx.notify();
            }
            return false;
        };
        let had_utf8_bom = self
            .file
            .as_ref()
            .is_some_and(|file| file.meta.had_utf8_bom);
        self.start_save(path.clone(), path, had_utf8_bom, None, close_after, cx)
    }

    /// Save the current document through the normal conflict-checked path.
    /// Returns `true` when a background write was started.
    pub fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_inner(false, cx)
    }

    /// Save the document, or overwrite a known external change on the second
    /// attempt. Returns `true` when a background write was started.
    pub(crate) fn save_inner(&mut self, close_after: bool, cx: &mut Context<Self>) -> bool {
        if self.saving {
            self.set_status("Save already in progress");
            cx.notify();
            return false;
        }
        let Some(file) = self.file.as_ref() else {
            self.set_status("Untitled document: use Save As");
            cx.notify();
            return false;
        };
        let force = std::mem::take(&mut self.force_next_save);
        let baseline = (!force).then(|| file.meta.clone());
        self.start_save(
            file.meta.canonical_target.clone(),
            file.meta.user_requested_path.clone(),
            file.meta.had_utf8_bom,
            baseline,
            close_after,
            cx,
        )
    }

    /// Snapshot the document and write it on the background executor. Late
    /// completion is bound to the captured history token: edits made while the
    /// write is in flight stay dirty even though the file itself was updated.
    fn start_save(
        &mut self,
        target: PathBuf,
        requested: PathBuf,
        had_utf8_bom: bool,
        baseline: Option<FileMeta>,
        close_after: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.saving {
            self.set_status("Save already in progress");
            cx.notify();
            return false;
        }
        let snapshot = self.document.snapshot();
        let state = self.document.state_token();
        let preferred_eol = self.document.preferred_eol();
        self.saving = true;
        self.set_status("Saving...");
        cx.notify();

        let task = cx.background_executor().spawn(async move {
            let text = snapshot.text_with_eol();
            let bytes = chaus_io::encode_for_save(&text, had_utf8_bom);
            chaus_io::save(chaus_io::SaveRequest::new(
                &target,
                &requested,
                &bytes,
                had_utf8_bom,
                preferred_eol,
                baseline.as_ref(),
            ))
        });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.finish_save(result, state, close_after, window, cx);
                })
                .ok();
        })
        .detach();
        true
    }

    fn finish_save(
        &mut self,
        result: Result<FileMeta, SaveError>,
        state: u64,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saving = false;
        match result {
            Ok(meta) => {
                self.file = Some(FileState { meta });
                self.refresh_language(cx);
                self.force_next_save = false;
                self.conflict_prompt = None;
                self.disk_change = None;
                self.disk_epoch = self.disk_epoch.wrapping_add(1);
                self.document.mark_saved_at(state);
                // A save only covers the captured revision. If the person
                // typed while the write was in flight, keep the durable
                // snapshot (and refresh it now) instead of deleting the only
                // crash-recovery copy.
                self.write_recovery_now(cx);
                if close_after && !self.document.is_dirty() {
                    window.remove_window();
                    return;
                }
                if close_after {
                    self.set_status("Saved the previous revision; save again to quit");
                    self.restore_close_prompt(window, cx);
                } else {
                    self.flash_status("Saved", cx);
                }
            }
            Err(SaveError::Conflict(conflict)) => {
                self.disk_change = Some(conflict);
                self.show_conflict_prompt(conflict, window, cx);
                self.set_status(format!("{conflict}"));
                if close_after {
                    self.close_prompt = true;
                }
            }
            Err(error) => {
                self.set_status(format!("Save failed: {error}"));
                if close_after {
                    self.restore_close_prompt(window, cx);
                }
            }
        }
        cx.notify();
    }
}
