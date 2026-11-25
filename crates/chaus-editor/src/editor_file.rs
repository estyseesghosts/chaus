//! Per-window file session: save, save-as, dirty close policy.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use chaus_core::position::{CharOffset, Selection};
use chaus_io::{FileMeta, LoadedFile, SaveError};
use gpui::{px, Context, Window};
use silvestris::file_panel::{ExtensionPolicy, PanelOutcome, SaveDisposition, SavePanel};
use silvestris::modal::ModalResult;

use crate::editor::Editor;
use crate::editor_keys::{Save, SaveAs};
use crate::file_dialogs::FileDialogs;
use crate::file_operation::{
    CloseAfter, FileOperation, FileRequest, FileRequestId, SaveTarget, WriteAssociation,
};

/// Map the panel's result onto the save state machine. The panel never writes;
/// only an accepted target with a host path proceeds.
fn save_outcome(outcome: &PanelOutcome) -> Result<(PathBuf, SaveDisposition), Option<String>> {
    if outcome.result != ModalResult::Accepted {
        return Err(None);
    }
    let Some(path) = outcome.first_path().cloned() else {
        return Err(Some(
            "the Save panel returned no host path for this target".to_string(),
        ));
    };
    let disposition = outcome.disposition.unwrap_or(SaveDisposition::New);
    Ok((path, disposition))
}

/// Test seam: hold the next background save so an edit, quit or second command
/// can be dispatched while the write is still in flight.
static SAVE_GATE: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);

/// Install a one-shot gate in front of the next background save.
pub fn set_save_gate_for_test(receiver: Option<mpsc::Receiver<()>>) {
    if let Ok(mut gate) = SAVE_GATE.lock() {
        *gate = receiver;
    }
}

fn wait_for_save_gate() {
    let receiver = SAVE_GATE.lock().ok().and_then(|mut gate| gate.take());
    if let Some(receiver) = receiver {
        let _ = receiver.recv();
    }
}

/// Test seam: pause the next write to a named target for `delay`, without
/// blocking the executor. The deterministic test scheduler fires the timer when
/// the test advances its clock, so a test can edit the document or create a
/// competing file between `start_save`'s snapshot and the commit.
static SAVE_DELAY: Mutex<Vec<(PathBuf, Duration)>> = Mutex::new(Vec::new());

/// Arm the delay for the write whose target ends with `target`. Entries are
/// keyed per target so tests that run in parallel cannot consume each other's
/// delay.
pub fn set_save_delay_for_test(target: PathBuf, delay: Duration) {
    if let Ok(mut slots) = SAVE_DELAY.lock() {
        slots.push((target, delay));
    }
}

fn take_save_delay_for_test(target: &Path) -> Option<Duration> {
    let mut slots = SAVE_DELAY.lock().ok()?;
    let index = slots
        .iter()
        .position(|(expected, _)| target.ends_with(expected) || expected.ends_with(target))?;
    Some(slots.remove(index).1)
}

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
            Ok(loaded) => self.install_loaded(&opened_path, loaded, cx),
            Err(error) => self.set_status(format!("Open failed: {error}")),
        }
        cx.notify();
    }

    /// Install a file that was loaded elsewhere, for asynchronous startup.
    /// A dirty document is never replaced by a deferred load.
    pub fn adopt_loaded(&mut self, path: &Path, loaded: LoadedFile, cx: &mut Context<Self>) {
        if self.document.is_dirty() {
            self.set_status("Open cancelled: the document has unsaved edits");
            cx.notify();
            return;
        }
        self.install_loaded(path, loaded, cx);
        cx.notify();
    }

    fn install_loaded(&mut self, path: &Path, loaded: LoadedFile, cx: &mut Context<Self>) {
        self.install_document(
            loaded.text,
            loaded.eol,
            loaded.preferred_eol,
            Some(FileState { meta: loaded.meta }),
            cx,
        );
        self.flash_status(format!("Opened {}", path.display()), cx);
    }

    /// Replace this window's document with an empty untitled one (`:enew`).
    /// Window preferences, theme and Vim mode stay; everything tied to the
    /// old document is reset.
    pub(crate) fn install_untitled(&mut self, cx: &mut Context<Self>) {
        self.cancel_file_operation();
        self.install_document(
            String::new(),
            chaus_core::eol::EolMap::default(),
            chaus_core::eol::Eol::Lf,
            None,
            cx,
        );
    }

    fn install_document(
        &mut self,
        text: String,
        eol: chaus_core::eol::EolMap,
        preferred_eol: chaus_core::eol::Eol,
        file: Option<FileState>,
        cx: &mut Context<Self>,
    ) {
        // Retire the old document's recovery snapshot under its own key
        // before the key is reset for the new document.
        self.remove_recovery_snapshot(cx);
        self.document.reset(text, eol, preferred_eol);
        self.cancel_format_job();
        self.document_generation.bump();
        self.file = file;
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
        self.reset_recovery_key();
        self.document.cancel_user_operation();
        self.vim_engine.reset();
        self.rebuild_rows();
    }

    pub(crate) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_none() {
            self.save_as(&SaveAs, window, cx);
            return;
        }
        self.save_inner(CloseAfter::No, cx);
    }

    pub(crate) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.save_as_inner(CloseAfter::No, window, cx);
    }

    /// Ask for a path without blocking the GPUI update. The picker belongs to
    /// the same request as the eventual write, so a second Save dispatched
    /// while the dialog is open is rejected instead of opening another one.
    pub(crate) fn save_as_inner(
        &mut self,
        close_after: CloseAfter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = self.begin_request(cx) else {
            self.close_not_started(close_after, window, cx);
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

        let dialogs = FileDialogs::ensure(cx);
        let absolute = suggested
            .as_deref()
            .map(|path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
        let directory = absolute
            .as_deref()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| dialogs.start_path());
        let suggested_name = suggested
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| "Untitled".to_string());
        let owner = window.window_handle();
        let target_editor = cx.weak_entity();
        let mut panel = SavePanel::new(dialogs.provider())
            .name(suggested_name)
            // Chaus edits arbitrary plaintext: never append or reject an
            // extension on the user's behalf.
            .extension_policy(ExtensionPolicy::Any)
            .state_store(dialogs.save_state())
            .on_finish(move |outcome, cx| {
                let result = save_outcome(outcome);
                // The panel can finish while its owner window is mid-update.
                // Defer the editor hand-off, and update the owner rather than
                // assuming the editor is the window's root view.
                let editor = target_editor.clone();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |_, window, cx| {
                        let _ = editor.update(cx, |editor, cx| {
                            editor.finish_save_selection(request, result, window, cx);
                        });
                    });
                });
            });
        if let Some(directory) = directory {
            panel = panel.directory_path(directory);
        }
        let panel = panel.begin_sheet(owner, window, cx);
        dialogs.remember_panel(&panel);
        true
    }

    fn finish_save_selection(
        &mut self,
        request: FileRequest,
        outcome: Result<(PathBuf, SaveDisposition), Option<String>>,
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
        let (path, disposition) = match outcome {
            Ok(selection) => selection,
            Err(Some(error)) => {
                self.file_operation = FileOperation::Idle;
                self.set_status(format!("Save As failed: {error}"));
                self.close_not_started(close_after, window, cx);
                cx.notify();
                return;
            }
            Err(None) => {
                self.file_operation = FileOperation::Idle;
                self.set_status("Save cancelled");
                self.close_not_started(close_after, window, cx);
                cx.notify();
                return;
            }
        };
        // The panel checked existence before returning. A target it reported
        // as new keeps create-new semantics, so an unrelated process that
        // creates the file in the meantime is a conflict, not a clobber.
        let refuse_existing = disposition != SaveDisposition::ReplaceConfirmed;
        self.start_save(
            request,
            SaveTarget {
                target: path.clone(),
                requested: path,
                had_utf8_bom,
                baseline: None,
                close_after,
                association: WriteAssociation::Associate,
                refuse_existing,
            },
            cx,
        );
    }

    /// A close-after-save that could not start or was cancelled. The GUI
    /// sheet comes back; an Ex `:wq` only reports, as the status already does.
    pub(crate) fn close_not_started(
        &mut self,
        close_after: CloseAfter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if close_after == CloseAfter::Sheet {
            self.restore_close_prompt(window, cx);
        }
    }

    /// Ex write to an explicit path. `association` decides whether the path
    /// becomes the document's file; `force` only permits replacing an
    /// existing file and never skips the current file's conflict check.
    pub(crate) fn write_to_path(
        &mut self,
        path: PathBuf,
        association: WriteAssociation,
        force: bool,
        close_after: CloseAfter,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(request) = self.begin_request(cx) else {
            return false;
        };
        let target = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        let had_utf8_bom = self
            .file
            .as_ref()
            .is_some_and(|file| file.meta.had_utf8_bom);
        self.force_next_save = false;
        self.start_save(
            request,
            SaveTarget {
                target,
                requested: path,
                had_utf8_bom,
                baseline: None,
                close_after,
                association,
                refuse_existing: !force,
            },
            cx,
        )
    }

    /// Save the current document through the normal conflict-checked path.
    /// Returns `true` when a background write was started.
    pub fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_inner(CloseAfter::No, cx)
    }

    /// Save the document, or overwrite a known external change on the second
    /// attempt. Returns `true` when a background write was started.
    pub(crate) fn save_inner(&mut self, close_after: CloseAfter, cx: &mut Context<Self>) -> bool {
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
                association: WriteAssociation::Associate,
                refuse_existing: false,
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
            association,
            refuse_existing,
        } = destination;
        let executor = cx.background_executor().clone();
        let task = cx.background_executor().spawn(async move {
            if let Some(delay) = take_save_delay_for_test(&target) {
                executor.timer(delay).await;
            }
            let text = snapshot.text_with_eol();
            let bytes = chaus_io::encode_for_save(&text, had_utf8_bom);
            let mut save_request = chaus_io::SaveRequest::new(
                &target,
                &requested,
                &bytes,
                had_utf8_bom,
                preferred_eol,
                baseline.as_ref(),
            );
            if refuse_existing {
                save_request = save_request.refusing_existing();
            }
            wait_for_save_gate();
            chaus_io::save(save_request)
        });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update_in(cx, |editor, window, cx| {
                    if association == WriteAssociation::Copy {
                        editor.finish_write_copy(request, result, cx);
                    } else {
                        editor.finish_save(request, result, close_after, window, cx);
                    }
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
        close_after: CloseAfter,
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
                if self.wqa_active && self.document.is_dirty() {
                    self.note_wqa_failure("the document changed while its save was running");
                }
                // A save only covers the captured revision. If the person
                // typed while the write was in flight, keep the durable
                // snapshot (and refresh it now) instead of deleting the only
                // crash-recovery copy.
                self.write_recovery_now(cx);
                if close_after.closes() && !self.document.is_dirty() {
                    window.remove_window();
                    return;
                }
                match close_after {
                    CloseAfter::Sheet => {
                        self.set_status("Saved the previous revision; save again to quit");
                        self.restore_close_prompt(window, cx);
                    }
                    CloseAfter::Ex => {
                        self.set_status("Saved the previous revision; the window stays open");
                    }
                    CloseAfter::No => self.flash_status("Saved", cx),
                    CloseAfter::Wqa => self.flash_status("Saved", cx),
                }
            }
            Err(SaveError::DurabilityUnconfirmed { source, meta, .. }) => {
                // The new bytes are on disk, so adopt them as the baseline; a
                // later save must not see its own write as an external change.
                // The document stays dirty and the recovery snapshot is kept
                // because the rename may not survive a power loss.
                self.file = Some(FileState { meta: *meta });
                self.force_next_save = false;
                self.conflict_prompt = None;
                self.disk_change = None;
                self.disk_epoch = self.disk_epoch.wrapping_add(1);
                self.write_recovery_now(cx);
                self.note_wqa_failure(format!("the folder could not be synced ({source})"));
                self.set_status(format!(
                    "Written, but the folder could not be synced ({source}); save again to confirm"
                ));
                self.close_not_started(close_after, window, cx);
            }
            Err(SaveError::Conflict(conflict)) => {
                self.note_wqa_failure(conflict.to_string());
                self.disk_change = Some(conflict);
                self.show_conflict_prompt(conflict, window, cx);
                self.set_status(format!("{conflict}"));
                if close_after == CloseAfter::Sheet {
                    self.close_prompt = true;
                }
            }
            Err(error) => {
                self.note_wqa_failure(format!("save failed: {error}"));
                self.set_status(format!("Save failed: {error}"));
                self.close_not_started(close_after, window, cx);
            }
        }
        cx.notify();
    }

    /// Completion of `:w path` on a named document. The bytes are a copy: the
    /// document keeps its file, dirty state and recovery snapshot whatever
    /// the outcome, even when the copy happens to match the saved text.
    fn finish_write_copy(
        &mut self,
        request: FileRequest,
        result: Result<FileMeta, SaveError>,
        cx: &mut Context<Self>,
    ) {
        if !self.owns_request(request) || self.document_generation != request.document {
            return;
        }
        self.file_operation = FileOperation::Idle;
        match result {
            Ok(meta) => {
                self.flash_status(format!("Wrote {}", meta.user_requested_path.display()), cx);
            }
            Err(SaveError::DurabilityUnconfirmed { path, source, .. }) => {
                self.set_status(format!(
                    "Wrote {}, but the folder could not be synced ({source})",
                    path.display()
                ));
            }
            Err(error) => self.set_status(format!("Write failed: {error}")),
        }
        cx.notify();
    }
}
