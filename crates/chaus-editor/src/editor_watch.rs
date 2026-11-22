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

impl Editor {
    pub(crate) fn start_disk_watch(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(WATCH_INTERVAL).await;
            let snapshot = editor
                .update(cx, |editor, _| {
                    if editor.saving || editor.opening {
                        return None;
                    }
                    editor
                        .file
                        .as_ref()
                        .map(|file| (file.meta.clone(), editor.disk_epoch))
                })
                .ok()
                .flatten();
            let Some((meta, epoch)) = snapshot else {
                continue;
            };
            let task = cx
                .background_executor()
                .spawn(async move { chaus_io::check_conflict(&meta) });
            let result = task.await;
            if editor
                .update(cx, |editor, cx| {
                    editor.apply_disk_check(epoch, result, cx);
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }

    /// Check the open file once, off the UI thread. Used by the periodic
    /// watcher and by deterministic tests.
    pub fn check_disk_now(&mut self, cx: &mut Context<Self>) {
        let Some((meta, epoch)) = self
            .file
            .as_ref()
            .map(|file| (file.meta.clone(), self.disk_epoch))
        else {
            return;
        };
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
