//! Crash-recovery snapshots and the launch-time Recover/Discard sheet.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chaus_core::document::DocumentSnapshot;
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, Selection};
use chaus_io::recovery::{RecoveryError, RecoveryRecord};
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

#[derive(Clone)]
pub(crate) struct RecoveryPromptFocus {
    pub(crate) recover: FocusHandle,
    pub(crate) discard: FocusHandle,
}

impl RecoveryPromptFocus {
    pub(crate) fn new(cx: &App) -> Self {
        Self {
            recover: cx.focus_handle(),
            discard: cx.focus_handle(),
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
    pub fn queue_recovery(
        &mut self,
        records: Vec<RecoveryRecord>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if records.is_empty() {
            return;
        }
        self.recovery_queue = records.into();
        window.focus(&self.recovery_focus.recover, cx);
        cx.notify();
    }

    pub(crate) fn schedule_recovery(&mut self, cx: &mut Context<Self>) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        let epoch = self.recovery_epoch;
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(SNAPSHOT_DELAY).await;
            let record = editor
                .update(cx, |editor, _| {
                    if editor.recovery_epoch != epoch {
                        return None;
                    }
                    if !editor.document.is_dirty() {
                        editor.retire_recovery_snapshot(epoch);
                        return None;
                    }
                    Some(editor.recovery_snapshot())
                })
                .ok()
                .flatten();
            if let Some(snapshot) = record {
                let write = cx.background_executor().spawn(async move {
                    let record = snapshot.into_record();
                    chaus_io::recovery::write_with_epoch(&record, epoch)
                });
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
            self.retire_recovery_snapshot(epoch);
            return;
        }
        let snapshot = self.recovery_snapshot();
        let weak = cx.weak_entity();
        cx.spawn(async move |_, cx| {
            let write = cx.background_executor().spawn(async move {
                let record = snapshot.into_record();
                chaus_io::recovery::write_with_epoch(&record, epoch)
            });
            if let Err(error) = write.await {
                let mut async_cx = cx.clone();
                report_recovery_failure(&weak, &mut async_cx, &error);
            }
        })
        .detach();
    }

    pub(crate) fn retire_recovery_snapshot(&self, epoch: u64) {
        if let Err(error) = chaus_io::recovery::retire(self.recovery_key, epoch) {
            eprintln!("chaus: could not retire recovery snapshot: {error}");
        }
    }

    pub(crate) fn remove_recovery_snapshot(&self) {
        if let Err(error) = chaus_io::recovery::remove(self.recovery_key) {
            eprintln!("chaus: could not remove recovery snapshot: {error}");
        }
    }

    pub(crate) fn reset_recovery_key(&mut self) {
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        self.recovery_key = new_recovery_id();
        self.recovery_original_path = None;
        self.recovery_had_utf8_bom = false;
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
        }
    }

    fn recover_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.recovery_queue.pop_front() else {
            return;
        };
        // Keep the on-disk record. The recovered document adopts its id, so a
        // later debounced snapshot replaces this record atomically; saving or
        // discarding retires it. Deleting it here would lose the only durable
        // copy if the process crashed before the next edit or save.
        self.recovery_key = record.id;
        self.recovery_epoch = self.recovery_epoch.wrapping_add(1);
        self.recovery_original_path = record.original_path.clone();
        self.recovery_had_utf8_bom = record.had_utf8_bom;
        let (text, eol) = EolMap::from_source(&record.text);
        self.document.reset(text, eol, record.preferred_eol);
        // A recovered document is a new document generation: a pending open or
        // save from before the reset must never settle against it.
        self.cancel_file_operation();
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
        self.file = None;
        self.reset_highlight(cx);
        self.document.cancel_user_operation();
        self.vim_engine.reset();
        self.rebuild_rows();
        let remaining = self.recovery_queue.len();
        // Opening another recovered document would replace this one before it
        // can be saved. Leave the remaining snapshots on disk for the next
        // launch instead of risking that loss.
        self.recovery_queue.clear();
        window.focus(&self.focus, cx);
        if remaining == 0 {
            self.set_status(format!(
                "Recovered unsaved changes for {}; use Save As to keep them",
                record.display_name()
            ));
        } else {
            self.set_status(format!(
                "Recovered {}; {remaining} other snapshot(s) kept for the next launch",
                record.display_name()
            ));
        }
        cx.notify();
    }

    fn discard_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record) = self.recovery_queue.pop_front() else {
            return;
        };
        if let Err(error) = chaus_io::recovery::remove(record.id) {
            eprintln!("chaus: could not remove discarded snapshot: {error}");
        }
        self.finish_recovery_choice(window, cx);
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
        self.recovery_queue.clear();
        window.focus(&self.focus, cx);
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
                let target = if self.recovery_focus.recover.is_focused(window) {
                    &self.recovery_focus.discard
                } else {
                    &self.recovery_focus.recover
                };
                window.focus(target, cx);
            }
            _ => {}
        }
    }

    pub(crate) fn render_recovery_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(record) = self.recovery_queue.front() else {
            return div().into_any_element();
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
                    .w(px(430.0))
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
                                "chaus found an unsaved snapshot for {}. Recover it or discard it.",
                                record.display_name()
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
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
