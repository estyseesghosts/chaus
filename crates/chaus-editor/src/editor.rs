//! The `Editor` entity: document, caret, scroll and per-window settings.

use std::collections::VecDeque;
use std::ops::Range;

use chaus_core::document::{Document, LineRange};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use chaus_core::settings::Settings;
use chaus_io::recovery::RecoveryRecord;
use chaus_io::LoadedFile;
use chaus_vim::Engine;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, App, Context, CursorStyle, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, Render, ScrollEvent, Styled as _, Window,
};
use silvestris::ActiveTheme;

use crate::editor_close::ClosePromptFocus;
use crate::editor_conflict::ConflictPromptFocus;
use crate::editor_element::EditorElement;
use crate::editor_file::FileState;
use crate::editor_find::FindBar;
use crate::editor_keys::{
    Backspace, Copy, Cut, Delete, DeleteWordLeft, DeleteWordRight, DocumentEnd, DocumentStart,
    Down, End, Escape, Find, FindNext, FindPrevious, Home, InsertNewline, InsertTab, Left, Outdent,
    Paste, Redo, Replace, Right, Save, SaveAs, SelectAll, SelectDocumentEnd, SelectDocumentStart,
    SelectDown, SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp, SelectWordLeft,
    SelectWordRight, ToggleVim, Undo, Up, WordLeft, WordRight, KEY_CONTEXT,
};
use crate::editor_recovery::{new_recovery_id, RecoveryPromptFocus};
use crate::geometry::Geometry;

/// One editor window.
pub struct Editor {
    pub(crate) document: Document,
    pub(crate) file: Option<FileState>,
    pub(crate) selection: Selection,
    pub(crate) preferred_column: Option<usize>,
    pub(crate) scroll_row: f32,
    pub(crate) scroll_x: Pixels,
    pub(crate) focus: FocusHandle,
    pub(crate) marked: Option<CharRange>,
    pub(crate) preedit_start: Option<CharOffset>,
    pub(crate) preedit_active: bool,
    pub(crate) selecting: bool,
    pub(crate) geometry: Option<Geometry>,
    pub(crate) line_rows: Vec<usize>,
    pub(crate) prefix_rows: Vec<usize>,
    pub(crate) prefix_valid: bool,
    pub(crate) rows_columns: Option<usize>,
    pub(crate) settings: Settings,
    pub(crate) status: Option<String>,
    pub(crate) find_bar: Option<FindBar>,
    pub(crate) close_prompt: bool,
    pub(crate) close_focus: ClosePromptFocus,
    pub(crate) conflict_prompt: Option<chaus_io::SaveConflict>,
    pub(crate) conflict_focus: ConflictPromptFocus,
    pub(crate) recovery_queue: VecDeque<RecoveryRecord>,
    pub(crate) recovery_focus: RecoveryPromptFocus,
    pub(crate) recovery_key: u64,
    pub(crate) recovery_epoch: u64,
    pub(crate) disk_change: Option<chaus_io::SaveConflict>,
    pub(crate) disk_epoch: u64,
    pub(crate) force_next_save: bool,
    pub(crate) vim: bool,
    pub(crate) vim_engine: Engine,
    pub(crate) caret_visible: bool,
    pub(crate) blink_epoch: u64,
    pub(crate) blink_key: Option<(Selection, bool)>,
    pub(crate) mapping_epoch: u64,
    pub(crate) saving: bool,
    pub(crate) opening: bool,
}

impl Editor {
    /// Build an editor for a loaded file or an untitled document.
    pub fn new(cx: &mut Context<Self>, loaded: Option<LoadedFile>, settings: Settings) -> Self {
        let (document, file) = match loaded {
            Some(loaded) => {
                let document =
                    Document::from_normalized_text(loaded.text, loaded.eol, loaded.preferred_eol);
                let state = FileState { meta: loaded.meta };
                (document, Some(state))
            }
            None => (
                Document::from_normalized_text(String::new(), EolMap::default(), Eol::Lf),
                None,
            ),
        };
        let vim = settings.vim;
        let (mappings, diagnostics) =
            chaus_vim::parse_mappings(&settings.normal_mappings, settings.mapleader);
        for diagnostic in diagnostics {
            eprintln!("chaus: {diagnostic}");
        }
        let close_focus = ClosePromptFocus::new(cx);
        let conflict_focus = ConflictPromptFocus::new(cx);
        let recovery_focus = RecoveryPromptFocus::new(cx);
        let mut editor = Self {
            document,
            file,
            selection: Selection::caret(CharOffset::ZERO),
            preferred_column: None,
            scroll_row: 0.0,
            scroll_x: px(0.),
            focus: cx.focus_handle(),
            marked: None,
            preedit_start: None,
            preedit_active: false,
            selecting: false,
            geometry: None,
            line_rows: Vec::new(),
            prefix_rows: Vec::new(),
            prefix_valid: false,
            rows_columns: None,
            settings,
            status: None,
            find_bar: None,
            close_prompt: false,
            close_focus,
            conflict_prompt: None,
            conflict_focus,
            recovery_queue: VecDeque::new(),
            recovery_focus,
            recovery_key: new_recovery_id(),
            recovery_epoch: 0,
            disk_change: None,
            disk_epoch: 0,
            force_next_save: false,
            vim,
            vim_engine: Engine::new(mappings),
            caret_visible: true,
            blink_epoch: 0,
            blink_key: None,
            mapping_epoch: 0,
            saving: false,
            opening: false,
        };
        editor.rebuild_rows();
        editor.start_disk_watch(cx);
        editor
    }

    #[must_use]
    pub fn document(&self) -> &Document {
        &self.document
    }

    #[must_use]
    pub fn selection(&self) -> Selection {
        self.selection
    }

    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.rebuild_rows();
    }

    #[must_use]
    pub fn vim_enabled(&self) -> bool {
        self.vim
    }

    /// Human-readable title for chrome and the OS window.
    #[must_use]
    pub fn title(&self) -> String {
        let name = self
            .file
            .as_ref()
            .map(|file| {
                file.meta
                    .user_requested_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.meta.user_requested_path.display().to_string())
            })
            .unwrap_or_else(|| "Untitled".to_string());
        if self.document.is_dirty() {
            format!("{name} — Edited")
        } else {
            name
        }
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
    }

    /// Show a transient message in the window corner.
    pub fn show_message(&mut self, message: impl Into<String>) {
        self.set_status(message);
    }

    // ---- line/row caches --------------------------------------------------

    pub(crate) fn rebuild_rows(&mut self) {
        let columns = self.rows_columns;
        self.line_rows.clear();
        for line in 0..self.document.len_lines() {
            let text = self.document.line_text(LineIndex(line));
            self.line_rows.push(chaus_layout::row_count(
                &text,
                columns,
                self.settings.tab_stop,
            ));
        }
        self.prefix_valid = false;
    }

    pub(crate) fn refresh_rows(&mut self, affected: LineRange) {
        if self.line_rows.len() != self.document.len_lines() {
            self.rebuild_rows();
            return;
        }
        let end = affected.end.0.min(self.line_rows.len());
        for line in affected.start.0..end {
            let text = self.document.line_text(LineIndex(line));
            self.line_rows[line] =
                chaus_layout::row_count(&text, self.rows_columns, self.settings.tab_stop);
        }
        self.prefix_valid = false;
    }

    pub(crate) fn ensure_prefix(&mut self) {
        if self.prefix_valid && self.prefix_rows.len() == self.line_rows.len() + 1 {
            return;
        }
        self.prefix_rows.clear();
        self.prefix_rows.reserve(self.line_rows.len() + 1);
        let mut total = 0;
        for rows in &self.line_rows {
            self.prefix_rows.push(total);
            total += rows;
        }
        self.prefix_rows.push(total);
        self.prefix_valid = true;
    }

    pub(crate) fn total_rows(&mut self) -> usize {
        self.ensure_prefix();
        self.prefix_rows.last().copied().unwrap_or(1).max(1)
    }

    pub(crate) fn set_wrap_columns(&mut self, columns: Option<usize>) {
        if self.rows_columns == columns {
            return;
        }
        self.rows_columns = columns;
        self.rebuild_rows();
    }

    pub(crate) fn segments_for(&self, line: LineIndex) -> Vec<chaus_layout::Segment> {
        let text = self.document.line_text(line);
        chaus_layout::segments(&text, self.rows_columns, self.settings.tab_stop)
    }

    pub(crate) fn local_row_of(&self, offset: CharOffset, line: LineIndex) -> usize {
        let segments = self.segments_for(line);
        let line_start = self.document.line_start(line);
        let local_char = offset.0.saturating_sub(line_start.0);
        segments
            .iter()
            .position(|segment| local_char < segment.char_end)
            .unwrap_or(segments.len().saturating_sub(1))
    }

    pub(crate) fn row_of_offset(&mut self, offset: CharOffset) -> (LineIndex, usize) {
        let line = self.document.line_of(offset);
        self.ensure_prefix();
        let row_in_line = self.local_row_of(offset, line);
        let row = self.prefix_rows.get(line.0).copied().unwrap_or(0) + row_in_line;
        (line, row)
    }

    /// Resolve a document-wide visual row to its hard line and the visual
    /// columns covered by that row.
    pub(crate) fn row_span(&mut self, row: usize) -> (LineIndex, Range<usize>) {
        let line = self.line_at_row(row);
        let local_row = row.saturating_sub(self.prefix_rows.get(line.0).copied().unwrap_or(0));
        let segments = self.segments_for(line);
        let span = segments
            .get(local_row)
            .or_else(|| segments.last())
            .map_or(0..0, |segment| segment.start_column..segment.end_column());
        (line, span)
    }

    pub(crate) fn caret_column(&self, offset: CharOffset) -> usize {
        let line = self.document.line_of(offset);
        let start = self.document.line_start(line);
        let text = self.document.line_text(line);
        chaus_layout::column_at(
            &text,
            offset.0.saturating_sub(start.0),
            self.settings.tab_stop,
        )
    }

    pub(crate) fn offset_at(&mut self, line: LineIndex, column: usize) -> CharOffset {
        let text = self.document.line_text(line);
        let local = chaus_layout::char_at_column(&text, column, self.settings.tab_stop);
        CharOffset(self.document.line_start(line).0 + local)
    }

    pub(crate) fn clamp_scroll(&mut self) {
        let total = self.total_rows();
        let visible = self
            .geometry
            .map(|geometry| geometry.visible_rows())
            .unwrap_or(1);
        let max_row = total.saturating_sub(visible);
        self.scroll_row = self.scroll_row.clamp(0.0, max_row as f32);
        self.scroll_x = self.scroll_x.max(px(0.));
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_blink(cx);
        let theme = cx.theme().clone();
        let title = self.title();
        window.set_window_title(&title);
        let background = silvestris::convert::hsla(theme.colors.content);
        let status = self.status.clone();
        let disk_banner = self
            .disk_change
            .is_some()
            .then(|| self.render_disk_banner(cx));
        let find_bar = self.find_bar.is_some().then(|| self.render_find_bar(cx));
        let close_prompt = self.close_prompt.then(|| self.render_close_prompt(cx));
        let conflict_prompt = self
            .conflict_prompt
            .is_some()
            .then(|| self.render_conflict_prompt(cx));
        let recovery_prompt =
            (!self.recovery_queue.is_empty()).then(|| self.render_recovery_prompt(cx));
        let vim_label = self.vim_mode_label();
        let content = div()
            .id("chaus-editor")
            .track_focus(&self.focus)
            // The focus handle must be tracked by an element in the rendered
            // tree, otherwise GPUI never adds `ChausEditor` to the key-dispatch
            // path and every action binding silently does nothing.
            .key_context(KEY_CONTEXT)
            .size_full()
            .relative()
            .bg(background)
            .text_color(silvestris::convert::hsla(theme.colors.text))
            .font_family(theme.type_.mono)
            .text_size(px(self.settings.font_size))
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|editor, event: &MouseDownEvent, window, cx| {
                    editor.on_mouse_down(event, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|editor, event: &MouseMoveEvent, window, cx| {
                editor.on_mouse_move(event, window, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|editor, event: &MouseUpEvent, window, cx| {
                    editor.on_mouse_up(event, window, cx);
                }),
            )
            .on_scroll(cx.listener(|editor, event: &ScrollEvent, window, cx| {
                editor.on_scroll(event, window, cx);
            }))
            .on_key_down(cx.listener(|editor, event: &KeyDownEvent, window, cx| {
                if editor.handle_vim_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|editor, action: &Backspace, window, cx| {
                editor.backspace(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Delete, window, cx| {
                editor.delete(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &DeleteWordLeft, window, cx| {
                editor.delete_word_left(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &DeleteWordRight, window, cx| {
                editor.delete_word_right(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Left, window, cx| {
                editor.left(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Right, window, cx| {
                editor.right(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Up, window, cx| {
                editor.up(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Down, window, cx| {
                editor.down(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectLeft, window, cx| {
                editor.select_left(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectRight, window, cx| {
                editor.select_right(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectUp, window, cx| {
                editor.select_up(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectDown, window, cx| {
                editor.select_down(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &WordLeft, window, cx| {
                editor.word_left_action(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &WordRight, window, cx| {
                editor.word_right_action(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectWordLeft, window, cx| {
                editor.select_word_left(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectWordRight, window, cx| {
                editor.select_word_right(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Home, window, cx| {
                editor.home(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &End, window, cx| {
                editor.end(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectHome, window, cx| {
                editor.select_home(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SelectEnd, window, cx| {
                editor.select_end(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &DocumentStart, window, cx| {
                editor.document_start(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &DocumentEnd, window, cx| {
                editor.document_end(action, window, cx);
            }))
            .on_action(
                cx.listener(|editor, action: &SelectDocumentStart, window, cx| {
                    editor.select_document_start(action, window, cx);
                }),
            )
            .on_action(
                cx.listener(|editor, action: &SelectDocumentEnd, window, cx| {
                    editor.select_document_end(action, window, cx);
                }),
            )
            .on_action(cx.listener(|editor, action: &SelectAll, window, cx| {
                editor.select_all(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Copy, window, cx| {
                editor.copy(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Cut, window, cx| {
                editor.cut(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Paste, window, cx| {
                editor.paste(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Undo, window, cx| {
                editor.undo(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Redo, window, cx| {
                editor.redo(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &InsertNewline, window, cx| {
                editor.insert_newline(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &InsertTab, window, cx| {
                editor.insert_tab(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Outdent, window, cx| {
                editor.outdent(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Save, window, cx| {
                editor.save(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &SaveAs, window, cx| {
                editor.save_as(action, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &Find, window, cx| {
                editor.open_find(false, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &Replace, window, cx| {
                editor.open_find(true, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &FindNext, _, cx| {
                editor.find_next(true, cx);
            }))
            .on_action(cx.listener(|editor, _: &FindPrevious, _, cx| {
                editor.find_next(false, cx);
            }))
            .on_action(cx.listener(|editor, action: &ToggleVim, window, cx| {
                editor.toggle_vim(action, window, cx);
            }))
            .on_action(cx.listener(|editor, action: &Escape, window, cx| {
                editor.escape(action, window, cx);
            }))
            .child(EditorElement {
                editor: cx.entity(),
            })
            .when_some(vim_label, |element, label| {
                element.child(
                    div()
                        .id("chaus-vim-mode")
                        .absolute()
                        .bottom(px(6.))
                        .left(px(10.))
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(silvestris::convert::hsla(theme.colors.group_fill))
                        .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                        .text_size(px(11.))
                        .child(label),
                )
            })
            .when_some(status, |element, message| {
                element.child(
                    div()
                        .id("chaus-status")
                        .absolute()
                        .bottom(px(6.))
                        .right(px(10.))
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(silvestris::convert::hsla(theme.colors.group_fill))
                        .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                        .text_size(px(11.))
                        .child(message),
                )
            })
            .when_some(disk_banner, |element, banner| element.child(banner))
            .when_some(find_bar, |element, bar| element.child(bar))
            .when_some(close_prompt, |element, sheet| element.child(sheet))
            .when_some(conflict_prompt, |element, sheet| element.child(sheet))
            .when_some(recovery_prompt, |element, sheet| element.child(sheet));
        silvestris::chrome::WindowFrame::new(title).child(content)
    }
}
