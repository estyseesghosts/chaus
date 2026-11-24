//! Per-window file session: save, save-as, dirty close policy.

use std::path::{Path, PathBuf};

use chaus_core::position::{CharOffset, Selection};
use chaus_io::{FileMeta, LoadedFile, SaveError};
use gpui::{px, Context, Window};

use crate::editor::Editor;
use crate::editor_keys::{Save, SaveAs};
use crate::file_operation::{FileOperation, FileRequest, FileRequestId, SaveTarget};

/// Deterministic stand-in for a background load. Tests use it to hold a load
/// open while they dispatch UI events, then release it and observe which
/// completion still owns the editor.
pub type LoadHook = fn(&Path) -> Result<LoadedFile, chaus_io::LoadError>;

/// The file the document was loaded from and its conflict baseline.
#[non_exhaustive]
#[derive(Debug)]
pub struct FileState {
    pub meta: FileMeta,
}

impl Editor {
    /// Start a request that owns the editor's file operation until it settles.
    /// Rejecting a second request is what keeps a re-entrant picker or a late
    /// completion from retargeting a live document.
    fn begin_request(&mut self, cx: &mut Context<Self>) -> Option<FileRequest> {
        if !self.file_operation.is_idle() {
            self.set_status("A file operation is already in progress");
            cx.notify();
            return None;
        }
        let request = FileRequest {
            id: FileRequestId::next(&mut self.file_request_next),
            document: self.document_generation,
            state: self.document.state_token(),
        };
        Some(request)
    }

    fn owns_request(&self, request: FileRequest) -> bool {
        self.file_operation.request() == Some(request)
    }

    /// Abandon an in-flight operation when the document underneath it is
    /// replaced outside the file session. Its completion must no longer settle.
    pub(crate) fn cancel_file_operation(&mut self) {
        self.file_request_next = self.file_request_next.wrapping_add(1);
        self.file_operation = FileOperation::Idle;
    }

    /// Whether an open, a picker or a write owns this editor's file session.
    #[must_use]
    pub(crate) fn file_operation_active(&self) -> bool {
        !self.file_operation.is_idle()
    }

    /// Replace this window's document with a loaded file. Dirty documents are
    /// refused unless `force` is set (the `:e!` form).
    pub fn open_path(&mut self, path: &Path, force: bool, cx: &mut Context<Self>) -> bool {
        let Some(request) = self.begin_request(cx) else {
            return false;
        };
        if self.document.is_dirty() && !force {
            self.set_status("No write since last change (add ! to override)");
            cx.notify();
            return false;
        }
        let requested = path.to_path_buf();
        let load_path = requested.clone();
        self.file_operation = FileOperation::Opening {
            request,
            path: requested.clone(),
            force,
        };
        self.set_status(format!("Opening {}...", requested.display()));
        cx.notify();
        let load_hook = self.load_hook;
        let task = cx.background_executor().spawn(async move {
            match load_hook {
                Some(hook) => hook(&load_path),
                None => chaus_io::load(&load_path),
            }
        });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_open(request, result, cx);
                })
                .ok();
        })
        .detach();
        true
    }

    fn finish_open(
        &mut self,
        request: FileRequest,
        result: Result<LoadedFile, chaus_io::LoadError>,
        cx: &mut Context<Self>,
    ) {
        if !self.owns_request(request) || self.document_generation != request.document {
            return;
        }
        let (force, opened_path) = match &self.file_operation {
            FileOperation::Opening { force, path, .. } => (*force, path.clone()),
            _ => return,
        };
        self.file_operation = FileOperation::Idle;
        // A forced reload skips the dirty precondition at start time, but it
        // must not overwrite edits made while the load was in flight.
        if self.document.state_token() != request.state || (!force && self.document.is_dirty()) {
            self.set_status("Open cancelled: the document changed while loading");
            cx.notify();
            return;
        }
        match result {
            Ok(loaded) => {
                self.document
                    .reset(loaded.text, loaded.eol, loaded.preferred_eol);
                self.cancel_format_job();
                self.document_generation.bump();
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
                self.remove_recovery_snapshot(cx);
                self.reset_recovery_key();
                self.document.cancel_user_operation();
                self.vim_engine.reset();
                self.rebuild_rows();
                self.flash_status(format!("Opened {}", opened_path.display()), cx);
            }
            Err(error) => self.set_status(format!("Open failed: {error}")),
        }
        cx.notify();
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_none() {
            self.save_as(&SaveAs, window, cx);
            return;
        }
        self.save_inner(false, cx);
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save_as_inner(false, window, cx);
    }

    /// Ask for a path without blocking the GPUI update. The picker belongs to
    /// the same request as the eventual write, so a second Save dispatched
    /// while the dialog is open is rejected instead of opening another one.
    pub(crate) fn save_as_inner(
        &mut self,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = self.begin_request(cx) else {
            if close_after {
                self.restore_close_prompt(window, cx);
            }
            return false;
        };
        // Choosing a new target abandons a pending conflict overwrite; a
        // cancelled picker must not leave the old force armed either.
        self.force_next_save = false;
        let suggested = self
            .file
            .as_ref()
            .map(|file| file.meta.user_requested_path.clone());
        let had_utf8_bom = self
            .file
            .as_ref()
            .is_some_and(|file| file.meta.had_utf8_bom);
        self.file_operation = FileOperation::ChoosingSaveTarget {
            request,
            close_after,
            had_utf8_bom,
        };
        self.set_status("Save As...");
        cx.notify();

        let directory = suggested
            .as_deref()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let suggested_name = suggested
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        let receiver = cx.prompt_for_new_path(&directory, suggested_name.as_deref());
        cx.spawn_in(window, async move |editor, cx| {
            let outcome = match receiver.await {
                Ok(Ok(Some(path))) => Ok(path),
                Ok(Ok(None)) => Err(None),
                Ok(Err(error)) => Err(Some(format!("{error:#}"))),
                Err(_) => Err(None),
            };
            editor
                .update_in(cx, |editor, window, cx| {
                    editor.finish_save_picker(request, outcome, window, cx);
                })
                .ok();
        })
        .detach();
        true
    }

    fn finish_save_picker(
        &mut self,
        request: FileRequest,
        outcome: Result<PathBuf, Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.owns_request(request) {
            return;
        }
        let (close_after, had_utf8_bom) = match &self.file_operation {
            FileOperation::ChoosingSaveTarget {
                close_after,
                had_utf8_bom,
                ..
            } => (*close_after, *had_utf8_bom),
            _ => return,
        };
        let path = match outcome {
            Ok(path) => path,
            Err(Some(error)) => {
                self.file_operation = FileOperation::Idle;
                self.set_status(format!("Save As failed: {error}"));
                if close_after {
                    self.restore_close_prompt(window, cx);
                } else {
                    cx.notify();
                }
                return;
            }
            Err(None) => {
                self.file_operation = FileOperation::Idle;
                self.set_status("Save cancelled");
                if close_after {
                    self.restore_close_prompt(window, cx);
                } else {
                    cx.notify();
                }
                return;
            }
        };
        self.start_save(
            request,
            SaveTarget {
                target: path.clone(),
                requested: path,
                had_utf8_bom,
                baseline: None,
                close_after,
            },
            cx,
        );
    }

    /// Save the current document through the normal conflict-checked path.
    /// Returns `true` when a background write was started.
    pub fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_inner(false, cx)
    }

    /// Save the document, or overwrite a known external change on the second
    /// attempt. Returns `true` when a background write was started.
    pub(crate) fn save_inner(&mut self, close_after: bool, cx: &mut Context<Self>) -> bool {
        let Some(request) = self.begin_request(cx) else {
            return false;
        };
        let Some(file) = self.file.as_ref() else {
            self.set_status("Untitled document: use Save As");
            cx.notify();
            return false;
        };
        let force = std::mem::take(&mut self.force_next_save);
        let baseline = (!force).then(|| file.meta.clone());
        self.start_save(
            request,
            SaveTarget {
                target: file.meta.canonical_target.clone(),
                requested: file.meta.user_requested_path.clone(),
                had_utf8_bom: file.meta.had_utf8_bom,
                baseline,
                close_after,
            },
            cx,
        )
    }

    /// Snapshot the document and write it on the background executor. Late
    /// completion is bound to the captured request and document generation:
    /// edits made while the write is in flight stay dirty, while a replacement
    /// of the document cannot be re-tagged as saved by the old write.
    fn start_save(
        &mut self,
        request: FileRequest,
        destination: SaveTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        let snapshot = self.document.snapshot();
        let preferred_eol = self.document.preferred_eol();
        self.file_operation = FileOperation::Saving { request };
        self.set_status("Saving...");
        cx.notify();

        let SaveTarget {
            target,
            requested,
            had_utf8_bom,
            baseline,
            close_after,
        } = destination;
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
                    editor.finish_save(request, result, close_after, window, cx);
                })
                .ok();
        })
        .detach();
        true
    }

    fn finish_save(
        &mut self,
        request: FileRequest,
        result: Result<FileMeta, SaveError>,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.owns_request(request) || self.document_generation != request.document {
            return;
        }
        self.file_operation = FileOperation::Idle;
        match result {
            Ok(meta) => {
                self.file = Some(FileState { meta });
                self.refresh_language(cx);
                self.force_next_save = false;
                self.conflict_prompt = None;
                self.disk_change = None;
                self.disk_epoch = self.disk_epoch.wrapping_add(1);
                self.document.mark_saved_at(request.state);
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
