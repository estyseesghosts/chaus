//! `:wqa`: save every modified named document, then close all windows.
//!
//! The command is not `:wa` followed by `:qa`. It starts the existing checked
//! save for each dirty named window, marks those windows as participants, and
//! closes only when every participant is idle and clean. Any failure,
//! conflict, untitled dirty document, duplicate target or edit made while a
//! save is in flight leaves all windows open.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, Window, WindowHandle};

use crate::editor::Editor;
use crate::editor_ex::other_editors;
use crate::file_operation::CloseAfter;

/// How often the coordinator checks whether the saves settled.
const WQA_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Default)]
struct WqaSnapshot {
    dirty: bool,
    untitled: bool,
    busy: bool,
    target: Option<PathBuf>,
    failure: Option<String>,
    active: bool,
}

impl Editor {
    fn wqa_snapshot(&self) -> WqaSnapshot {
        WqaSnapshot {
            dirty: self.document.is_dirty(),
            untitled: self.file.is_none(),
            busy: self.file_operation_active(),
            target: self
                .file
                .as_ref()
                .map(|file| file.meta.canonical_target.clone()),
            failure: self.wqa_failure.clone(),
            active: self.wqa_active,
        }
    }

    pub(crate) fn note_wqa_failure(&mut self, message: impl Into<String>) {
        if self.wqa_active && self.wqa_failure.is_none() {
            self.wqa_failure = Some(message.into());
        }
    }

    fn clear_wqa(&mut self) {
        self.wqa_active = false;
        self.wqa_failure = None;
    }

    pub(crate) fn ex_write_quit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.wqa_active {
            self.set_status(":wqa is already running");
            return;
        }
        let others = other_editors(window, cx);
        let mut snapshots: Vec<WqaSnapshot> = Vec::with_capacity(others.len() + 1);
        snapshots.push(self.wqa_snapshot());
        for handle in &others {
            match handle.read_with(cx, |editor, _| editor.wqa_snapshot()) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(_) => {
                    // A window that disappeared is no longer a participant.
                }
            }
        }

        if snapshots.iter().any(|snapshot| snapshot.active) {
            self.set_status(":wqa is already running");
            return;
        }
        if snapshots.iter().any(|snapshot| snapshot.busy) {
            self.set_status(":wqa stopped: a file operation is still running");
            return;
        }
        if snapshots
            .iter()
            .any(|snapshot| snapshot.dirty && snapshot.untitled)
        {
            self.set_status(":wqa stopped: an untitled document has unsaved changes");
            return;
        }
        let mut targets = HashSet::new();
        for snapshot in &snapshots {
            if !snapshot.dirty {
                continue;
            }
            let Some(target) = snapshot.target.clone() else {
                self.set_status(":wqa stopped: a modified window has no file");
                return;
            };
            if !targets.insert(target) {
                self.set_status(":wqa stopped: two windows target the same file");
                return;
            }
        }

        self.wqa_active = true;
        self.wqa_failure = None;
        for handle in &others {
            let _ = handle.update(cx, |editor, _, _| {
                editor.wqa_active = true;
                editor.wqa_failure = None;
            });
        }
        self.start_wqa_save(cx);
        for handle in &others {
            let _ = handle.update(cx, |editor, _, cx| editor.start_wqa_save(cx));
        }

        self.spawn_wqa_coordinator(others, cx);
    }

    fn start_wqa_save(&mut self, cx: &mut Context<Self>) {
        if !self.document.is_dirty() {
            return;
        }
        if self.file.is_none() {
            self.note_wqa_failure("an untitled document has unsaved changes");
            return;
        }
        if !self.save_inner(CloseAfter::Wqa, cx) {
            self.note_wqa_failure("the save could not start");
        }
    }

    fn spawn_wqa_coordinator(&mut self, others: Vec<WindowHandle<Editor>>, cx: &mut Context<Self>) {
        cx.spawn(async move |editor, cx| loop {
            cx.background_executor().timer(WQA_POLL).await;
            let Some(self_snapshot) = editor.read_with(cx, |editor, _| editor.wqa_snapshot()).ok()
            else {
                return;
            };
            let mut failure = self_snapshot.failure.clone();
            let mut all_idle = !self_snapshot.busy;
            let mut all_clean = !self_snapshot.dirty;
            for handle in &others {
                let Ok(snapshot) = handle.read_with(cx, |editor, _| editor.wqa_snapshot()) else {
                    continue;
                };
                if failure.is_none() {
                    failure = snapshot.failure.clone();
                }
                all_idle &= !snapshot.busy;
                all_clean &= !snapshot.dirty;
            }

            if let Some(message) = failure {
                let _ = editor.update(cx, |editor, cx| {
                    editor.clear_wqa();
                    editor.set_status(format!(":wqa stopped: {message}"));
                    cx.notify();
                });
                for handle in &others {
                    let _ = handle.update(cx, |editor, _, _| editor.clear_wqa());
                }
                return;
            }
            if !all_idle {
                continue;
            }
            if !all_clean {
                let _ = editor.update(cx, |editor, cx| {
                    editor.clear_wqa();
                    editor.set_status(":wqa stopped: a document is still modified");
                    cx.notify();
                });
                for handle in &others {
                    let _ = handle.update(cx, |editor, _, _| editor.clear_wqa());
                }
                return;
            }
            for handle in &others {
                let _ = handle.update(cx, |editor, window, cx| {
                    editor.close_for_quit_all(window, cx)
                });
            }
            let _ = editor.update_in(cx, |editor, window, cx| {
                editor.close_for_quit_all(window, cx)
            });
            return;
        })
        .detach();
    }
}
