//! Running validated Ex commands.
//!
//! Every file-touching command routes through the shared file session in
//! `editor_file.rs`; this module only chooses which operation and how its
//! completion should treat the window.

use std::path::{Path, PathBuf};

use chaus_core::document::{EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, LineIndex, Selection};
use chaus_vim::ex_command::{LineTarget, SetArgument, SetOption};
use chaus_vim::{ExCommand, ExPath};
use gpui::{Context, Window, WindowHandle};

use crate::editor::Editor;
use crate::file_operation::{CloseAfter, FileOperation, WriteAssociation};

const NO_WRITE: &str = "No write since last change (add ! to override)";
const BUSY: &str = "A file operation is in progress";

/// Resolve the one path form the Ex parser keeps symbolic. Quoted paths never
/// expand; unquoted `~` goes through the shared I/O helper.
fn resolve_ex_path(path: &ExPath) -> PathBuf {
    if path.expands_home() {
        chaus_io::expand_home(path.as_path())
    } else {
        path.as_path().to_path_buf()
    }
}

impl Editor {
    pub(crate) fn run_ex_command(
        &mut self,
        command: ExCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            ExCommand::Write { force, path } => {
                self.ex_write(force, path.as_ref().map(resolve_ex_path), window, cx);
            }
            ExCommand::SaveAs { force, path } => {
                self.write_to_path(
                    resolve_ex_path(&path),
                    WriteAssociation::Associate,
                    force,
                    CloseAfter::No,
                    cx,
                );
            }
            ExCommand::Quit { force } => self.ex_quit(force, window, cx),
            ExCommand::WriteQuit => self.ex_write_quit(window, cx),
            ExCommand::Exit => {
                if self.document.is_dirty() {
                    self.ex_write_quit(window, cx);
                } else {
                    self.ex_quit(false, window, cx);
                }
            }
            ExCommand::Edit { force, path } => {
                self.ex_edit(force, path.as_ref().map(resolve_ex_path), cx);
            }
            ExCommand::New { force } => self.ex_new(force, cx),
            ExCommand::File => {
                let message = self.file_summary();
                self.flash_status(message, cx);
            }
            ExCommand::WorkingDirectory => match std::env::current_dir() {
                Ok(directory) => self.flash_status(directory.display().to_string(), cx),
                Err(error) => self.set_status(format!("pwd failed: {error}")),
            },
            // Vim search moves the caret and paints no match decoration, so
            // there is nothing to hide. The pattern for `n`/`N` is kept.
            ExCommand::NoHighlight => self.flash_status("No search highlight is shown", cx),
            ExCommand::GotoLine(target) => self.ex_goto_line(target),
            ExCommand::Set(arguments) => self.ex_set(&arguments, cx),
            ExCommand::Format => self.format_document(cx),
            ExCommand::Trim => self.trim_trailing_whitespace(cx),
            ExCommand::Retab => self.convert_indentation_to_spaces(cx),
            ExCommand::WriteAll => self.ex_write_all(window, cx),
            ExCommand::QuitAll { force } => self.ex_quit_all(force, window, cx),
            ExCommand::WriteQuitAll => self.ex_write_quit_all(window, cx),
            ExCommand::Read { path } => self.ex_read(resolve_ex_path(&path), cx),
            _ => self.set_status("This command is not supported in this build"),
        }
        cx.notify();
    }

    fn ex_write(
        &mut self,
        force: bool,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (path, self.file.is_some()) {
            // `:w!` on the current file is the ordinary save. External-change
            // protection still applies; the conflict sheet is the override.
            (None, true) => {
                self.save_inner(CloseAfter::No, cx);
            }
            (None, false) => {
                self.save_as_inner(CloseAfter::No, window, cx);
            }
            (Some(path), true) => {
                if self.is_current_file(&path) {
                    self.save_inner(CloseAfter::No, cx);
                } else {
                    self.write_to_path(path, WriteAssociation::Copy, force, CloseAfter::No, cx);
                }
            }
            (Some(path), false) => {
                self.write_to_path(path, WriteAssociation::Associate, force, CloseAfter::No, cx);
            }
        }
    }

    /// Lexical comparison only: the UI thread does not touch the disk. An
    /// alias that is not recognised here is treated as another file, so
    /// without `!` it is refused as existing rather than silently replaced.
    fn is_current_file(&self, path: &Path) -> bool {
        let Some(file) = self.file.as_ref() else {
            return false;
        };
        let Ok(absolute) = std::path::absolute(path) else {
            return false;
        };
        let requested = std::path::absolute(&file.meta.user_requested_path).ok();
        absolute == file.meta.canonical_target || Some(&absolute) == requested.as_ref()
    }

    fn ex_quit(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        if force {
            if matches!(self.file_operation, FileOperation::Saving { .. }) {
                self.set_status("A save is still running; :q! waits until it finishes");
                return;
            }
            self.cancel_file_operation();
            self.discard_and_close(window, cx);
            return;
        }
        if self.file_operation_active() {
            self.set_status(BUSY);
            return;
        }
        if self.document.is_dirty() {
            self.set_status(NO_WRITE);
            return;
        }
        self.remove_recovery_snapshot(cx);
        window.remove_window();
    }

    fn ex_write_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file.is_some() {
            self.save_inner(CloseAfter::Ex, cx);
        } else {
            self.save_as_inner(CloseAfter::Ex, window, cx);
        }
    }

    fn ex_edit(&mut self, force: bool, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let path = match path {
            Some(path) => path,
            None => match self.file.as_ref() {
                Some(file) => file.meta.user_requested_path.clone(),
                None => {
                    self.set_status("No file name to reload");
                    return;
                }
            },
        };
        self.open_path(&path, force, cx);
    }

    fn ex_new(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.file_operation_active() {
            self.set_status(BUSY);
            return;
        }
        if self.document.is_dirty() && !force {
            self.set_status(NO_WRITE);
            return;
        }
        self.install_untitled(cx);
        self.flash_status("New document", cx);
    }

    fn file_summary(&self) -> String {
        let name = self.file.as_ref().map_or_else(
            || "[No Name]".to_owned(),
            |file| format!("\"{}\"", file.meta.user_requested_path.display()),
        );
        let modified = if self.document.is_dirty() {
            " [Modified]"
        } else {
            ""
        };
        let lines = self.document.len_lines();
        let unit = if lines == 1 { "line" } else { "lines" };
        format!("{name}{modified} {lines} {unit}")
    }

    /// `:N` addresses a hard line, not a soft-wrapped row. Lines past the end
    /// clamp to the last line, as in Vim.
    fn ex_goto_line(&mut self, target: LineTarget) {
        let last = self.document.len_lines().saturating_sub(1);
        let line = match target {
            LineTarget::Number(number) => (number.get() - 1).min(last),
            LineTarget::Last => last,
        };
        let caret = chaus_vim::first_non_space(&self.document, LineIndex(line));
        self.selection = Selection::caret(caret);
        self.preferred_column = None;
        self.scroll_to_caret();
    }

    fn ex_set(&mut self, arguments: &[SetArgument], cx: &mut Context<Self>) {
        let mut settings = self.settings.clone();
        let mut changed = false;
        let mut answers = Vec::new();
        for argument in arguments {
            match *argument {
                SetArgument::Wrap(mode) => {
                    settings.wrap = mode;
                    changed = true;
                }
                SetArgument::ExpandTab(expand) => {
                    settings.expand_tabs = expand;
                    changed = true;
                }
                SetArgument::TabStop(tab_stop) => {
                    settings.tab_stop = tab_stop;
                    changed = true;
                }
                SetArgument::Query(option) => {
                    let value = match option {
                        SetOption::Wrap => format!("wrap={}", settings.wrap.as_str()),
                        SetOption::ExpandTab if settings.expand_tabs => "expandtab".to_owned(),
                        SetOption::ExpandTab => "noexpandtab".to_owned(),
                        SetOption::TabStop => format!("tabstop={}", settings.tab_stop),
                    };
                    answers.push(value);
                }
            }
        }
        if changed {
            self.set_settings(settings, cx);
        }
        if !answers.is_empty() {
            self.flash_status(answers.join("  "), cx);
        } else if changed {
            self.flash_status("Settings updated for this window", cx);
        }
    }

    /// Save every modified named document in every window. Untitled windows
    /// are reported and skipped; no picker is opened on anyone's behalf.
    fn ex_write_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut started = 0;
        let mut untitled = 0;
        let mut busy = 0;
        let mut tally = |editor: &mut Editor, cx: &mut Context<Editor>| {
            if !editor.document.is_dirty() {
                return;
            }
            if editor.file.is_none() {
                untitled += 1;
            } else if editor.file_operation_active() {
                busy += 1;
            } else if editor.save_inner(CloseAfter::No, cx) {
                started += 1;
            }
        };
        tally(self, cx);
        for handle in other_editors(window, cx) {
            let _ = handle.update(cx, |editor, _, cx| tally(editor, cx));
        }
        let mut message = format!("Saving {started} modified file(s)");
        if untitled > 0 {
            message.push_str(&format!("; {untitled} untitled not written"));
        }
        if busy > 0 {
            message.push_str(&format!("; {busy} busy"));
        }
        self.set_status(message);
    }

    /// Close every window. Without `!`, nothing closes unless every window is
    /// clean and idle. With `!`, unsaved edits are discarded, but a window
    /// with a write in flight still blocks the whole command.
    fn ex_quit_all(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let others = other_editors(window, cx);
        let mut dirty = usize::from(self.document.is_dirty());
        let mut busy = usize::from(self.file_operation_active());
        let mut saving = usize::from(matches!(self.file_operation, FileOperation::Saving { .. }));
        for handle in &others {
            let _ = handle.update(cx, |editor, _, _| {
                dirty += usize::from(editor.document.is_dirty());
                busy += usize::from(editor.file_operation_active());
                saving += usize::from(matches!(
                    editor.file_operation,
                    FileOperation::Saving { .. }
                ));
            });
        }
        if saving > 0 {
            self.set_status(format!(
                "{saving} window(s) still saving; nothing was closed"
            ));
            return;
        }
        if !force && (dirty > 0 || busy > 0) {
            self.set_status(format!(
                "{dirty} window(s) have unsaved changes, {busy} busy; nothing was closed (add ! to discard)"
            ));
            return;
        }
        for handle in others {
            let _ = handle.update(cx, |editor, window, cx| {
                editor.close_for_quit_all(window, cx)
            });
        }
        self.close_for_quit_all(window, cx);
    }

    pub(crate) fn close_for_quit_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_file_operation();
        self.discard_and_close(window, cx);
    }
    /// Insert another file's text after the current hard line as one undo
    /// step. The read runs off the UI thread; if the document changes before
    /// it returns, nothing is inserted.
    fn ex_read(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let generation = self.document_generation;
        let revision = self.document.revision();
        let line = self.document.line_of(self.selection.head);
        self.set_status(format!("Reading {}...", path.display()));
        let load_path = path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { chaus_io::load(&load_path) });
        cx.spawn(async move |editor, cx| {
            let result = task.await;
            editor
                .update(cx, |editor, cx| {
                    if editor.document_generation != generation
                        || editor.document.revision() != revision
                    {
                        editor.set_status(":r cancelled: the document changed while reading");
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(loaded) => editor.insert_read_text(line, &path, loaded.text, cx),
                        Err(error) => editor.set_status(format!("Read failed: {error}")),
                    }
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// The loaded text has `\n` breaks. It becomes whole lines below `line`:
    /// one trailing newline is dropped so no blank line appears, and the
    /// insertion starts with a break at the end of `line`'s content.
    fn insert_read_text(
        &mut self,
        line: LineIndex,
        path: &Path,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let body = text.strip_suffix('\n').unwrap_or(&text);
        if text.is_empty() {
            self.flash_status(format!("{}: empty file, nothing read", path.display()), cx);
            return;
        }
        let at = self.document.line_content_end(line);
        let insert = format!("\n{body}");
        let first_new_line = CharOffset(at.0 + 1);
        let lines = body.split('\n').count();
        let transaction = EditTransaction::new(
            EditCause::Paste,
            TextEdit::insertion(at, insert),
            self.selection,
            Selection::caret(first_new_line),
        );
        match self.document.apply(transaction) {
            Ok(result) => {
                self.selection = Selection::caret(first_new_line);
                self.after_change(&result, cx);
                self.flash_status(format!("Read {lines} line(s) from {}", path.display()), cx);
            }
            Err(_) => self.set_status("Read failed: the insertion was rejected"),
        }
    }
}

/// Live editor windows other than `window`, by stable handle.
pub(crate) fn other_editors(window: &Window, cx: &Context<Editor>) -> Vec<WindowHandle<Editor>> {
    let current = window.window_handle();
    cx.windows()
        .into_iter()
        .filter(|handle| *handle != current)
        .filter_map(|handle| handle.downcast::<Editor>())
        .collect()
}
