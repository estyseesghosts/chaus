//! Background polling for changes made to the open file by other programs.

use std::time::Duration;

use chaus_io::{ConflictCheckError, SaveConflict};
use gpui::{
    div, px, AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window,
};
use silvestris::controls::{Button, ControlSize};
use silvestris::ActiveTheme;

use crate::editor::Editor;

const WATCH_INTERVAL: Duration = Duration::from_secs(2);
/// Force a content hash even when size/mtime/identity are unchanged. Timestamp
/// granularity can hide a same-size edit, so metadata alone is never trusted
/// forever.
const FORCED_HASH_TICKS: u32 = 15;
/// Consecutive failed inspections before the watcher reports one warning.
///
/// One failure is usually a transient lock or a rename in progress. Three in a
/// row mean the file cannot be inspected, but reporting on every two-second
/// tick would turn that into a permanent status flash.
const INSPECTION_FAILURE_WARNING_THRESHOLD: u32 = 3;

/// Bounded per-window state for watcher inspection failures. It lives in the
/// watcher task, so no editor or global state is needed and it cannot grow with
/// the number of failed ticks.
#[derive(Debug, Default)]
struct WatchErrorState {
    consecutive_failures: u32,
    warning: Option<String>,
}

#[derive(Debug)]
enum WatchErrorAction {
    None,
    Warn(String),
    Clear(String),
}

impl WatchErrorState {
    fn observe(&mut self, result: &Result<(), ConflictCheckError>) -> WatchErrorAction {
        match result {
            // A conflict means inspection succeeded; only I/O failures count.
            Ok(()) | Err(ConflictCheckError::Conflict(_)) => {
                let cleared = self.warning.take();
                self.consecutive_failures = 0;
                match cleared {
                    Some(message) => WatchErrorAction::Clear(message),
                    None => WatchErrorAction::None,
                }
            }
            Err(ConflictCheckError::Io { path, source }) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if self.consecutive_failures != INSPECTION_FAILURE_WARNING_THRESHOLD
                    || self.warning.is_some()
                {
                    return WatchErrorAction::None;
                }
                let message = format!(
                    "Cannot inspect {} for changes: {source}; edits may go unnoticed",
                    path.display()
                );
                self.warning = Some(message.clone());
                WatchErrorAction::Warn(message)
            }
            // Future non-exhaustive variants are not actionable yet.
            Err(_) => WatchErrorAction::None,
        }
    }
}

impl Editor {
    /// Clear a watcher warning only if it is still the visible status. A save
    /// or another action may have replaced it, and that message must survive.
    fn clear_watch_warning(&mut self, message: &str, cx: &mut Context<Self>) {
        if self.status.as_deref() == Some(message) {
            self.status = None;
            self.status_epoch = self.status_epoch.wrapping_add(1);
            cx.notify();
        }
    }

    pub(crate) fn start_disk_watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |editor, cx| {
            let mut ticks: u32 = 0;
            let mut error_state = WatchErrorState::default();
            loop {
                cx.background_executor().timer(WATCH_INTERVAL).await;
                let snapshot = editor.update(cx, |editor, _| {
                    if editor.file_operation_active() || editor.disk_check_in_flight {
                        return None;
                    }
                    editor
                        .file
                        .as_ref()
                        .map(|file| (file.meta.clone(), editor.disk_epoch))
                });
                // The entity is gone: stop the timer instead of looping forever
                // against a closed window.
                let Ok(snapshot) = snapshot else {
                    break;
                };
                // Entity still alive but nothing is open (or a save/open is in
                // progress): keep waiting without touching the disk.
                let Some((meta, epoch)) = snapshot else {
                    continue;
                };
                ticks = ticks.wrapping_add(1);
                let force_hash = ticks.is_multiple_of(FORCED_HASH_TICKS);
                if editor
                    .update(cx, |editor, _| {
                        editor.disk_check_in_flight = true;
                    })
                    .is_err()
                {
                    break;
                }
                let task = cx.background_executor().spawn(async move {
                    if force_hash || !chaus_io::metadata_matches(&meta) {
                        chaus_io::check_conflict(&meta)
                    } else {
                        Ok(())
                    }
                });
                let result = task.await;
                let applied = editor.update(cx, |editor, cx| {
                    // A result from a previous file must not move this file's
                    // error state or overwrite its status.
                    if editor.disk_epoch != epoch {
                        return;
                    }
                    match error_state.observe(&result) {
                        WatchErrorAction::Warn(message) => editor.set_status(message),
                        WatchErrorAction::Clear(message) => {
                            editor.clear_watch_warning(&message, cx);
                        }
                        WatchErrorAction::None => {}
                    }
                    editor.apply_disk_check(epoch, result, cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Check the open file once, off the UI thread. Used by the periodic
    /// watcher and by deterministic tests. Always hashes, so it is also the
    /// path that catches same-size edits hidden by coarse timestamps.
    pub fn check_disk_now(&mut self, cx: &mut Context<Self>) {
        let Some((meta, epoch)) = self
            .file
            .as_ref()
            .map(|file| (file.meta.clone(), self.disk_epoch))
        else {
            return;
        };
        if self.disk_check_in_flight {
            return;
        }
        self.disk_check_in_flight = true;
        let task = cx
            .background_executor()
            .spawn(async move { chaus_io::check_conflict(&meta) });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.apply_disk_check(epoch, result, cx);
                })
                .ok();
        })
        .detach();
    }

    fn apply_disk_check(
        &mut self,
        epoch: u64,
        result: Result<(), ConflictCheckError>,
        cx: &mut Context<Self>,
    ) {
        self.disk_check_in_flight = false;
        if epoch != self.disk_epoch {
            return;
        }
        match result {
            Ok(()) => {
                if self.disk_change.take().is_some() {
                    cx.notify();
                }
            }
            Err(ConflictCheckError::Conflict(conflict)) => {
                if self.disk_change != Some(conflict) {
                    self.disk_change = Some(conflict);
                    self.set_status(format!("{conflict}"));
                    cx.notify();
                }
            }
            // A transient read error must not be presented as a change, and
            // future non-exhaustive variants are ignored until routed.
            Err(_) => {}
        }
    }

    fn review_disk_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(conflict) = self.disk_change {
            self.show_conflict_prompt(conflict, window, cx);
        }
    }

    pub(crate) fn render_disk_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(conflict) = self.disk_change else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let message = match conflict {
            SaveConflict::ChangedOnDisk => "File changed on disk",
            SaveConflict::DeletedOnDisk => "File was deleted on disk",
            _ => "File changed on disk",
        };
        div()
            .id("chaus-disk-banner")
            .debug_selector(|| "chaus-disk-banner".to_string())
            .absolute()
            .top(px(8.0))
            .left(px(10.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(silvestris::convert::hsla(theme.colors.error))
            .bg(silvestris::convert::hsla(theme.colors.group_fill))
            .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
            .text_size(px(11.0))
            .child(message)
            .child(
                Button::new("disk-review", "Review")
                    .size(ControlSize::Small)
                    .on_click(cx.listener(|editor, _, window, cx| {
                        editor.review_disk_change(window, cx);
                    })),
            )
            .into_any_element()
    }
}
