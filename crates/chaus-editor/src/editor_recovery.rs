//! Crash-recovery snapshots and the launch-time Recover/Discard sheet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chaus_core::document::DocumentSnapshot;
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, Selection};
use chaus_io::recovery::{PendingRecoveryOp, RecoveryEntry, RecoveryError, RecoveryRecord};
use gpui::{
    div, px, AnyElement, App, AsyncApp, Context, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window,
};
use silvestris::controls::Button;
use silvestris::ActiveTheme;

use crate::editor::Editor;

const SNAPSHOT_DELAY: Duration = Duration::from_millis(750);
static RECOVERY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A failed snapshot is a persistent, visible condition: the dirty document
/// has no durable copy until the next successful write.
fn report_recovery_failure(editor: &WeakEntity<Editor>, cx: &mut AsyncApp, error: &RecoveryError) {
    let _ = editor.update(cx, |editor, cx| {
        editor.set_status(format!("Recovery unavailable: {error}"));
        cx.notify();
    });
}

/// Retire on the background executor. The epoch watermark in `chaus-io` is the
/// ordering barrier: a stale write that reaches the lock after retirement is
/// discarded instead of recreating the record. The operation ticket lets the
/// application drain retirement before the process exits.
fn spawn_recovery_retire(
    key: u64,
    epoch: u64,
    path: Option<PathBuf>,
    editor: &WeakEntity<Editor>,
    cx: &mut Context<Editor>,
) {
    let ticket = PendingRecoveryOp::begin();
    let task = cx.background_executor().spawn(async move {
        let result = match path {
            Some(path) => chaus_io::recovery::retire_path(&path, key, epoch),
            None => chaus_io::recovery::retire(key, epoch),
        };
        // The ticket is released only after the filesystem work finished, so
        // the shutdown drain observes a real completion, not a queued call.
        drop(ticket);
        result
    });
    let weak = editor.clone();
    cx.spawn(async move |_, cx| {
        if let Err(error) = task.await {
            let mut async_cx = cx.clone();
            let _ = weak.update(&mut async_cx, |editor, cx| {
                editor.set_status(format!("Recovery cleanup failed: {error}"));
                cx.notify();
            });
        }
    })
    .detach();
}

fn spawn_recovery_retire_many(
    entries: Vec<RecoveryEntry>,
    epoch: u64,
    editor: &WeakEntity<Editor>,
    cx: &mut Context<Editor>,
) {
    if entries.is_empty() {
        return;
    }
    let ticket = PendingRecoveryOp::begin();
    let task = cx.background_executor().spawn(async move {
        let result = chaus_io::recovery::retire_entries(&entries, epoch);
        drop(ticket);
        result
    });
    let weak = editor.clone();
    cx.spawn(async move |_, cx| {
        if let Err(error) = task.await {
            let mut async_cx = cx.clone();
            let _ = weak.update(&mut async_cx, |editor, cx| {
                editor.set_status(format!("Recovery cleanup failed: {error}"));
                cx.notify();
            });
        }
    })
    .detach();
}

/// Track a background snapshot write for the shutdown drain.
fn tracked_write(
    executor: gpui::BackgroundExecutor,
    snapshot: RecoverySnapshot,
    epoch: u64,
) -> gpui::Task<Result<PathBuf, RecoveryError>> {
    let ticket = PendingRecoveryOp::begin();
    executor.spawn(async move {
        let directory = snapshot.directory.clone();
        let record = snapshot.into_record();
        let result = match directory {
            Some(directory) => chaus_io::recovery::write_to_with_epoch(&directory, &record, epoch),
            None => chaus_io::recovery::write_with_epoch(&record, epoch),
        };
        drop(ticket);
        result
    })
}

#[derive(Clone)]
pub(crate) struct RecoveryPromptFocus {
    pub(crate) recover: FocusHandle,
    pub(crate) discard: FocusHandle,
    pub(crate) discard_all: FocusHandle,
}

impl RecoveryPromptFocus {
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            recover: cx.focus_handle(),
            discard: cx.focus_handle(),
            discard_all: cx.focus_handle(),
        }
    }
}

/// A process-unique key for one editor window's snapshot.
pub(crate) fn new_recovery_id() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let sequence = RECOVERY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    nanos ^ (u64::from(std::process::id()) << 32) ^ sequence
}

/// Everything a recovery record needs, captured cheaply on the UI thread.
///
/// The expanded text and the on-disk encoding are produced later on the
/// background executor by [`RecoverySnapshot::into_record`].
struct RecoverySnapshot {
    id: u64,
    original_path: Option<PathBuf>,
    document: DocumentSnapshot,
    preferred_eol: Eol,
    had_utf8_bom: bool,
    revision: u64,
    /// Pin a recovered document's snapshot to the store it came from.
    directory: Option<PathBuf>,
}

impl RecoverySnapshot {
    fn into_record(self) -> RecoveryRecord {
        RecoveryRecord::new(
            self.id,
            self.original_path,
            self.document.text_with_eol(),
            self.preferred_eol,
            self.had_utf8_bom,
            self.revision,
        )
    }
}

impl Editor {
    /// Queue snapshots found at launch. The sheet shows them one at a time.
    ///
    /// Returns `false` when this window already holds unsaved work. Recovery
    /// must never replace text the person typed while the background scan was
    /// running; the caller then opens a separate recovery window.
    pub fn queue_recovery(
        &mut self,
        records: Vec<RecoveryEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if records.is_empty() {
            return false;
        }
        if self.document.is_dirty() || self.file_operation_active() {
            return false;
        }
        self.recovery_queue = records.into();
        window.focus(&self.recovery_focus.recover, cx);
        // The sheet has not rendered yet, so its button focus handle is not in
        // the key dispatch tree at this instant. Re-apply focus after the
        // effect cycle; otherwise Tab and typing fall through to the editor
        // behind the modal sheet.
        let focus = self.recovery_focus.recover.clone();
        window.defer(cx, move |window, cx| {
            window.focus(&focus, cx);
        });
        cx.notify();
        true
    }

    pub(crate) fn schedule_recovery(&mut self, cx: &mut Context<Self>) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        let epoch = self.recovery_epoch;
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(SNAPSHOT_DELAY).await;
            let record = editor
                .update(cx, |editor, cx| {
                    if editor.recovery_epoch != epoch {
                        return None;
                    }
                    if !editor.document.is_dirty() {
                        let key = editor.recovery_key;
                        let path = editor.recovery_path.clone();
                        spawn_recovery_retire(key, epoch, path, &cx.weak_entity(), cx);
                        return None;
                    }
                    Some(editor.recovery_snapshot())
                })
                .ok()
                .flatten();
            if let Some(snapshot) = record {
                let write = tracked_write(cx.background_executor().clone(), snapshot, epoch);
                if let Err(error) = write.await {
                    let mut async_cx = cx.clone();
                    report_recovery_failure(&editor, &mut async_cx, &error);
                }
            }
        })
        .detach();
    }

    /// Snapshot the document immediately instead of waiting for the debounce.
    /// Used when a background save completes while newer edits remain.
    pub(crate) fn write_recovery_now(&mut self, cx: &mut Context<Self>) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        let epoch = self.recovery_epoch;
        if !self.document.is_dirty() {
            self.retire_recovery_snapshot(epoch, cx);
            return;
        }
        let snapshot = self.recovery_snapshot();
        let weak = cx.weak_entity();
        let write = tracked_write(cx.background_executor().clone(), snapshot, epoch);
        cx.spawn(async move |_, cx| {
            if let Err(error) = write.await {
                let mut async_cx = cx.clone();
                report_recovery_failure(&weak, &mut async_cx, &error);
            }
        })
        .detach();
    }

    pub(crate) fn retire_recovery_snapshot(&self, epoch: u64, cx: &mut Context<Self>) {
        spawn_recovery_retire(
            self.recovery_key,
            epoch,
            self.recovery_path.clone(),
            &cx.weak_entity(),
            cx,
        );
    }

    /// Remove the snapshot for the current document. Bumping the epoch first
    /// invalidates any write already in flight for the old revision.
    pub(crate) fn remove_recovery_snapshot(&mut self, cx: &mut Context<Self>) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        spawn_recovery_retire(
            self.recovery_key,
            self.recovery_epoch,
            self.recovery_path.clone(),
            &cx.weak_entity(),
            cx,
        );
    }

    /// Test entry point for the retirement path used by close/clear handlers.
    pub fn retire_snapshot_for_test(&mut self, cx: &mut Context<Self>) {
        self.remove_recovery_snapshot(cx);
    }

    /// Snapshot key of this window, for deterministic filesystem fixtures.
    #[must_use]
    pub fn recovery_key_for_test(&self) -> u64 {
        self.recovery_key
    }

    /// Retained original path and BOM of a recovered document, for tests that
    /// prove Save As still sees them after the record is installed.
    #[must_use]
    pub fn recovery_metadata_for_test(&self) -> (Option<PathBuf>, bool) {
        (
            self.recovery_original_path.clone(),
            self.recovery_had_utf8_bom,
        )
    }

    /// A successful associate-save makes a recovered document a normal named
    /// document. Retire the adopted `.rec` and start a new recovery identity so
    /// later debounced snapshots go to the normal store instead of replacing
    /// the record this window no longer owns.
    pub(crate) fn finish_recovery_after_save(&mut self, cx: &mut Context<Self>) {
        if self.recovery_path.is_none() && self.recovery_original_path.is_none() {
            return;
        }
        let old_key = self.recovery_key;
        let old_path = self.recovery_path.take();
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        let epoch = self.recovery_epoch;
        spawn_recovery_retire(old_key, epoch, old_path, &cx.weak_entity(), cx);
        self.recovery_key = new_recovery_id();
        self.recovery_original_path = None;
        self.recovery_had_utf8_bom = false;
        self.recovery_directory = None;
    }

    pub(crate) fn reset_recovery_key(&mut self) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        self.recovery_key = new_recovery_id();
        self.recovery_original_path = None;
        self.recovery_had_utf8_bom = false;
        self.recovery_directory = None;
        self.recovery_path = None;
    }

    /// Cheap capture for the background writer. Cloning the Rope and the EOL
    /// map does not copy the text, so the debounce callback never expands a
    /// multi-MiB document on the UI thread.
    fn recovery_snapshot(&self) -> RecoverySnapshot {
        let (original_path, had_utf8_bom) = self.file.as_ref().map_or(
            (
                self.recovery_original_path.clone(),
                self.recovery_had_utf8_bom,
            ),
            |file| {
                (
                    Some(file.meta.user_requested_path.clone()),
                    file.meta.had_utf8_bom,
                )
            },
        );
        RecoverySnapshot {
            id: self.recovery_key,
            original_path,
            document: self.document.snapshot(),
            preferred_eol: self.document.preferred_eol(),
            had_utf8_bom,
            revision: self.document.state_token(),
            directory: self.recovery_directory.clone(),
        }
    }

    fn recover_next(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.recovery_queue.pop_front() else {
            return;
        };
        if self.document.is_dirty() {
            // The sheet should still be modal, but never overwrite typed work
            // even if a future input path slips through.
            self.recovery_queue.push_front(entry);
            self.set_status("Recovery paused: save or discard your changes first");
            cx.notify();
            return;
        }
        self.set_status(format!(
            "Loading recovery snapshot {}...",
            entry.display_detail()
        ));
        cx.notify();
        let generation = self.document_generation;
        let state = self.document.state_token();
        let fallback = entry.clone();
        let weak = cx.weak_entity();
        let task = cx.background_executor().spawn(async move { entry.read() });
        cx.spawn(async move |_, cx| {
            let record = task.await;
            weak.update_in(cx, |editor, window, cx| {
                if editor.document_generation != generation
                    || editor.document.state_token() != state
                    || editor.document.is_dirty()
                {
                    editor.recovery_queue.push_front(fallback.clone());
                    editor.set_status("Recovery cancelled: the document changed while loading");
                    editor.finish_recovery_choice(window, cx);
                    return;
                }
                let Some(record) = record else {
                    editor.set_status(format!(
                        "Recovery snapshot for {} could not be read",
                        fallback.display_detail()
                    ));
                    editor.finish_recovery_choice(window, cx);
                    return;
                };
                editor.recovery_path = Some(fallback.path.clone());
                editor.recovery_directory = fallback.path.parent().map(Path::to_path_buf);
                editor.install_recovered(record, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn install_recovered(
        &mut self,
        record: RecoveryRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keep the on-disk record. The recovered document adopts its id, so a
        // later debounced snapshot replaces this record atomically; saving or
        // discarding retires it. Deleting it here would lose the only durable
        // copy if the process crashed before the next edit or save.
        let name = record.display_detail();
        self.recovery_key = record.id;
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        self.recovery_original_path = record.original_path.clone();
        self.recovery_had_utf8_bom = record.had_utf8_bom;
        let (text, eol) = EolMap::from_source(&record.text);
        self.document.reset(text, eol, record.preferred_eol);
        // A recovered document is a new document generation: a pending open or
        // save from before the reset must never settle against it.
        self.cancel_file_operation();
        self.cancel_format_job();
        self.document_generation.bump();
        self.document.mark_unsaved();
        self.selection = Selection::caret(CharOffset::ZERO);
        self.preferred_column = None;
        self.scroll_row = 0.0;
        self.scroll_x = px(0.0);
        self.marked = None;
        self.preedit_start = None;
        self.preedit_active = false;
        self.force_next_save = false;
        self.pending_close_after = None;
        self.pending_open_close = None;
        self.file = None;
        self.reset_highlight(cx);
        self.document.cancel_user_operation();
        self.vim_engine.reset();
        self.rebuild_rows();
        let remaining = self.recovery_queue.len();
        // Opening another recovered document would replace this one before it
        // can be saved. Leave the remaining snapshots on disk for the next
        // launch; Discard All can clean them out if they are not wanted.
        self.recovery_queue.clear();
        window.focus(&self.focus, cx);
        if remaining == 0 {
            self.set_status(format!("Recovered unsaved changes for {name}"));
        } else {
            self.set_status(format!(
                "Recovered {name}; {remaining} other snapshot(s) remain on disk for the next launch"
            ));
        }
        cx.notify();
    }

    fn discard_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.recovery_queue.pop_front() else {
            return;
        };
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        spawn_recovery_retire_many(vec![record], self.recovery_epoch, &cx.weak_entity(), cx);
        self.finish_recovery_choice(window, cx);
    }

    fn discard_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recovery_queue.is_empty() {
            return;
        }
        let entries: Vec<RecoveryEntry> = self.recovery_queue.drain(..).collect();
        let count = entries.len();
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        spawn_recovery_retire_many(entries, self.recovery_epoch, &cx.weak_entity(), cx);
        window.focus(&self.focus, cx);
        self.set_status(format!("Discarded {count} recovery snapshot(s)"));
        cx.notify();
    }

    fn finish_recovery_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recovery_queue.is_empty() {
            window.focus(&self.focus, cx);
        } else {
            window.focus(&self.recovery_focus.recover, cx);
        }
        cx.notify();
    }

    pub(crate) fn dismiss_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.recovery_queue.len();
        self.recovery_queue.clear();
        window.focus(&self.focus, cx);
        if count > 0 {
            self.set_status(format!(
                "{count} recovery snapshot(s) kept for the next launch"
            ));
        }
        cx.notify();
    }

    pub(crate) fn recovery_prompt_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.dismiss_recovery(window, cx),
            "tab" => {
                cx.stop_propagation();
                let reverse = event.keystroke.modifiers.shift;
                let order = [
                    &self.recovery_focus.recover,
                    &self.recovery_focus.discard,
                    &self.recovery_focus.discard_all,
                ];
                let current = order
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(0);
                let next = if reverse {
                    (current + order.len() - 1) % order.len()
                } else {
                    (current + 1) % order.len()
                };
                window.focus(order[next], cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_recovery_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(record) = self.recovery_queue.front() else {
            return div().into_any_element();
        };
        let total = self.recovery_queue.len();
        let position = if total == 1 {
            "Snapshot".to_string()
        } else {
            format!("Snapshot 1 of {total}")
        };
        div()
            .id("chaus-recovery-sheet")
            .debug_selector(|| "chaus-recovery-sheet".to_string())
            .role(gpui::Role::AlertDialog)
            .aria_label("Recover unsaved changes")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(58.0))
            .bg(silvestris::convert::hsla(theme.colors.scrim))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|editor, event: &KeyDownEvent, window, cx| {
                cx.stop_propagation();
                editor.recovery_prompt_key(event, window, cx);
            }))
            .child(
                div()
                    .w(px(500.0))
                    .max_w_full()
                    .p(px(18.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(silvestris::convert::hsla(theme.colors.group_border))
                    .bg(silvestris::convert::hsla(theme.colors.window))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Recover unsaved changes?"),
                    )
                    .child(
                        div()
                            .text_size(px(theme.type_.size_small))
                            .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                            .child(format!(
                                "{position} for {} ({}). Recover it, discard it, or discard all snapshots.",
                                record.display_detail(),
                                record.saved_label()
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new("recovery-discard-all", "Discard All")
                                    .focus_handle(self.recovery_focus.discard_all.clone())
                                    .destructive(true)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.discard_all(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("recovery-discard", "Discard")
                                    .focus_handle(self.recovery_focus.discard.clone())
                                    .destructive(true)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.discard_next(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("recovery-recover", "Recover")
                                    .focus_handle(self.recovery_focus.recover.clone())
                                    .default_button(true)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.recover_next(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
