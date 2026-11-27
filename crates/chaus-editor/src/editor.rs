//! The `Editor` entity: document, caret, scroll and per-window settings.
//!
//! `Editor` coordinates its subsystem owners. It keeps the central document,
//! selection, settings and focus handle directly, owns the cross-subsystem
//! mutation-completion path and decides which background work to spawn. The
//! owners keep their own primitive fields private; adapters call narrow methods
//! on the owner or on `Editor` instead of reaching through them.

use std::ops::Range;
use std::path::Path;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use chaus_core::document::{Document, LineRange};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, LineIndex, Selection};
use chaus_core::settings::{Settings, WrapMode};
use chaus_format::Formatter;
use chaus_highlight::LanguageId;
use chaus_io::LoadedFile;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, App, Context, CursorStyle, Div, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, ScrollEvent, Styled as _,
    Window,
};
use silvestris::ActiveTheme;

use crate::blink_state::BlinkState;
use crate::editor_close::ClosePromptFocus;
use crate::editor_conflict::ConflictPromptFocus;
use crate::editor_element::EditorElement;
use crate::editor_file::FileState;
use crate::editor_find::FindBar;
use crate::editor_keys::{
    Backspace, ConvertIndentationToSpaces, ConvertIndentationToTabs, Copy, Cut, Delete,
    DeleteWordLeft, DeleteWordRight, DocumentEnd, DocumentStart, Down, End, Escape, Find, FindNext,
    FindPrevious, FormatDocument, Home, InsertNewline, InsertTab, Left, Outdent, Paste, Redo,
    Replace, Right, Save, SaveAs, SelectAll, SelectDocumentEnd, SelectDocumentStart, SelectDown,
    SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp, SelectWordLeft, SelectWordRight,
    ToggleVim, TrimTrailingWhitespace, Undo, Up, WordLeft, WordRight, ZoomIn, ZoomOut, ZoomReset,
    KEY_CONTEXT, MODAL_KEY_CONTEXT, VIM_OWNED_KEY_CONTEXT,
};
use crate::file_session::FileSession;
use crate::format_state::FormatState;
use crate::geometry::Geometry;
use crate::highlight_state::HighlightState;
use crate::menu_state::MenuState;
use crate::pointer_state::PointerState;
use crate::recovery_state::RecoveryState;
use crate::status_state::StatusState;
use crate::text_input_state::TextInputState;
use crate::viewport_state::{LayoutWorkCounters, ReflowOutcome, ViewportState};
use crate::vim_state::VimState;

/// Routine confirmations (`Saved`, `Copied`) disappear after this long.
const TRANSIENT_STATUS: Duration = Duration::from_secs(4);

/// Test seam: hold the next reflow measurement so an edit can land between
/// request and completion.
static REFLOW_GATE: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);

/// Install a one-shot gate in front of the next background reflow.
pub fn set_reflow_gate_for_test(receiver: Option<mpsc::Receiver<()>>) {
    if let Ok(mut gate) = REFLOW_GATE.lock() {
        *gate = receiver;
    }
}

fn wait_for_reflow_gate() {
    let receiver = REFLOW_GATE.lock().ok().and_then(|mut gate| gate.take());
    if let Some(receiver) = receiver {
        let _ = receiver.recv();
    }
}

/// One editor window.
pub struct Editor {
    pub(crate) document: Document,
    pub(crate) selection: Selection,
    pub(crate) settings: Settings,
    pub(crate) focus: FocusHandle,
    pub(crate) viewport: ViewportState,
    pub(crate) pointer: PointerState,
    pub(crate) text_input: TextInputState,
    pub(crate) file: FileSession,
    pub(crate) recovery: RecoveryState,
    pub(crate) vim: VimState,
    pub(crate) blink: BlinkState,
    pub(crate) format: FormatState,
    pub(crate) menu: MenuState,
    pub(crate) highlight: HighlightState,
    pub(crate) status: StatusState,
    pub(crate) find_bar: Option<FindBar>,
    pub(crate) close_prompt: bool,
    pub(crate) close_focus: ClosePromptFocus,
    pub(crate) conflict_prompt: Option<chaus_io::SaveConflict>,
    pub(crate) conflict_focus: ConflictPromptFocus,
}

impl Editor {
    /// Build an editor for a loaded file or an untitled document.
    pub fn new(cx: &mut Context<Self>, loaded: Option<LoadedFile>, settings: Settings) -> Self {
        let (document, file_state) = match loaded {
            Some(loaded) => {
                let document =
                    Document::from_normalized_text(loaded.text, loaded.eol, loaded.preferred_eol);
                (document, Some(FileState { meta: loaded.meta }))
            }
            None => (
                Document::from_normalized_text(String::new(), EolMap::default(), Eol::Lf),
                None,
            ),
        };
        // Language detection runs exactly once, before the highlight owner is
        // constructed. Later path changes re-detect through `refresh_language`.
        let language = file_state
            .as_ref()
            .map(|file| LanguageId::from_path(&file.meta.user_requested_path))
            .unwrap_or(LanguageId::PlainText);
        let vim = settings.vim;
        let (mappings, diagnostics) =
            chaus_vim::parse_mappings(&settings.normal_mappings, settings.mapleader);
        for diagnostic in diagnostics {
            eprintln!("chaus: {diagnostic}");
        }
        let close_focus = ClosePromptFocus::new(cx);
        let conflict_focus = ConflictPromptFocus::new(cx);
        let commands = Self::create_command_registry(cx);
        let mut editor = Self {
            document,
            selection: Selection::caret(CharOffset::ZERO),
            settings,
            focus: cx.focus_handle(),
            viewport: ViewportState::new(),
            pointer: PointerState::new(),
            text_input: TextInputState::new(),
            file: FileSession::new(file_state),
            recovery: RecoveryState::new(cx),
            vim: VimState::new(mappings, vim),
            blink: BlinkState::new(),
            format: FormatState::new(),
            menu: MenuState::new(commands),
            highlight: HighlightState::new(),
            status: StatusState::new(),
            find_bar: None,
            close_prompt: false,
            close_focus,
            conflict_prompt: None,
            conflict_focus,
        };
        let weak = cx.weak_entity();
        editor.register_editor_commands(weak, cx);
        // Command chords belong to the registry, not to `bind_keys`. Applying
        // them here keeps a standalone editor (tests, embedding) consistent
        // while `chaus-app` re-applies after adding the application commands.
        if let Err(error) = silvestris::command::apply_bindings(
            editor.menu.commands(),
            &[crate::editor_keys::KEY_CONTEXT],
            cx,
        ) {
            eprintln!("chaus: could not bind document commands: {error}");
        }
        editor.highlight.reset_language(language);
        editor.schedule_highlight(None, cx);
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

    /// The file this window owns, when the document came from disk. Open uses
    /// its directory as the starting location; an untitled document has none.
    #[must_use]
    pub fn document_path(&self) -> Option<&Path> {
        self.file.path()
    }

    /// Replace the selection and caret without changing the document.
    ///
    /// Keeps anchor/head direction, so callers can restore a reversed
    /// selection or select a range from search and navigation code.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = selection.clamp(self.document.len_chars());
        self.viewport.set_preferred_column(None);
        cx.notify();
    }

    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Effective document font size for this window, including zoom.
    #[must_use]
    pub fn editor_font_size(&self) -> f32 {
        self.viewport.effective_font_size(self.settings.font_size)
    }

    /// Family used for document rows. An explicit user family wins; otherwise
    /// the active theme's monospace face follows family changes. `None` is
    /// what keeps Aqua and Geist independent of any stored family name.
    #[must_use]
    pub fn editor_font_family(&self, theme: &silvestris::core::tokens::Theme) -> String {
        self.settings
            .font_family
            .clone()
            .unwrap_or_else(|| theme.type_.mono.to_string())
    }

    /// Whether a background disk conflict check is currently running.
    ///
    /// The watcher skips a tick while this is set, so a slow poll of a large
    /// file is not stacked behind the next one.
    #[must_use]
    pub fn is_checking_disk(&self) -> bool {
        self.file.check_in_flight()
    }

    /// Simulate the epoch change a completed save or reload performs while a
    /// background disk check is still in flight.
    #[doc(hidden)]
    pub fn advance_disk_epoch_for_test(&mut self) {
        self.file.bump_epoch();
    }

    /// Committed text-area geometry, for pointer and scrolling tests.
    #[doc(hidden)]
    #[must_use]
    pub fn geometry_for_test(&self) -> Option<Geometry> {
        self.viewport.geometry()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn scroll_x_for_test(&self) -> Pixels {
        self.viewport.scroll_x()
    }

    #[doc(hidden)]
    pub fn set_scroll_x_for_test(&mut self, x: Pixels) {
        self.viewport.set_scroll_x(x);
    }

    #[doc(hidden)]
    pub fn clamp_scroll_for_test(&mut self) {
        self.clamp_scroll();
    }

    #[doc(hidden)]
    pub fn offset_for_point_for_test(
        &mut self,
        position: gpui::Point<Pixels>,
    ) -> Option<CharOffset> {
        self.offset_for_point(position)
    }

    #[doc(hidden)]
    pub fn row_span_for_test(
        &mut self,
        row: usize,
    ) -> (LineIndex, Range<chaus_layout::VisualColumn>) {
        self.row_span(row)
    }

    #[doc(hidden)]
    pub fn offset_at_for_test(&mut self, line: LineIndex, column: usize) -> CharOffset {
        self.offset_at(line, chaus_layout::VisualColumn::new(column))
    }

    #[doc(hidden)]
    #[must_use]
    pub fn scroll_row_for_test(&self) -> f32 {
        self.viewport.scroll_row()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn selecting_for_test(&self) -> bool {
        self.pointer.selecting()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn drag_autoscroll_active_for_test(&self) -> bool {
        self.pointer.autoscroll_active()
    }

    /// This window's command registry. Application-level commands (New, Open,
    /// Preferences) are registered on it by the app so the titlebar menu uses
    /// one tree.
    #[must_use]
    pub fn commands(&self) -> &Entity<silvestris::command::CommandRegistry> {
        self.menu.commands()
    }

    /// Apply a new settings snapshot without recreating this editor.
    ///
    /// Only settings that change glyph advances, tab expansion or the break
    /// policy force a background re-measure. Cheap preferences (indent style,
    /// the default Vim mode) and pure geometry changes leave the committed row
    /// index alone. No change synchronously rebuilds the whole document.
    pub fn set_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let effect = settings_effect(&self.settings, &settings);
        self.settings = settings;

        if effect.remeasure {
            // Publish the new policy and width only with the measured rows in
            // the reflow completion. Committing either field here would pair a
            // new policy with an old row index for a frame. The generation check
            // drops a result that is stale when it lands.
            let columns = if self.settings.wrap == WrapMode::None {
                None
            } else {
                self.viewport.committed_columns()
            };
            self.viewport
                .set_request(columns, Self::wrap_policy(self.settings.wrap));
            if !self.viewport.is_reflowing() {
                self.spawn_reflow(cx);
            }
        }
        if effect.geometry {
            self.clamp_scroll();
        }
        cx.notify();
    }

    /// Install a deterministic stand-in for a background load.
    pub fn set_load_hook_for_test(&mut self, hook: crate::editor_file::LoadHook) {
        self.file.set_load_hook(hook);
    }

    /// Use this formatter instead of detecting one from the file extension.
    pub fn set_formatter_for_test(&mut self, formatter: Formatter) {
        self.format.set_formatter(formatter);
    }

    /// Current soft-wrap mode for this window.
    #[must_use]
    pub fn wrap_mode(&self) -> WrapMode {
        self.settings.wrap
    }

    /// Change wrapping without touching the document or its undo history.
    pub fn set_wrap_mode(&mut self, mode: WrapMode, cx: &mut Context<Self>) {
        if self.settings.wrap == mode {
            return;
        }
        let mut settings = self.settings.clone();
        settings.wrap = mode;
        self.set_settings(settings, cx);
        self.clamp_scroll();
        self.scroll_to_caret();
        cx.notify();
    }

    #[must_use]
    pub fn vim_enabled(&self) -> bool {
        self.vim.enabled()
    }

    /// The document name for chrome and the OS window. Dirty state belongs to
    /// the window frame's edited marker, not to the title string.
    #[must_use]
    pub fn title(&self) -> String {
        self.file
            .path()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            })
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status.set(message);
    }

    /// Show a routine confirmation that disappears unless another message
    /// replaces it first. The epoch prevents an old timer from clearing a newer
    /// message, and persistent warnings set through [`Self::set_status`] bump
    /// the epoch too.
    pub(crate) fn flash_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.set_status(message);
        let epoch = self.status.epoch();
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(TRANSIENT_STATUS).await;
            editor
                .update(cx, |editor, cx| {
                    if editor.status.clear_if_epoch(epoch) {
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    /// Current corner message, if one is visible.
    #[must_use]
    pub fn status_text(&self) -> Option<&str> {
        self.status.text()
    }

    /// The file this window's document is associated with, if any.
    #[must_use]
    pub fn file_state(&self) -> Option<&crate::editor_file::FileState> {
        self.file.state()
    }

    /// Text of the open Vim command line, without its prefix.
    #[must_use]
    pub fn command_line_text(&self) -> Option<String> {
        self.vim
            .command_line()
            .map(|view| format!("{}{}", view.before_caret, view.after_caret))
    }

    /// Show a transient message in the window corner.
    pub fn show_message(&mut self, message: impl Into<String>) {
        self.set_status(message);
    }

    /// Show a confirmation that disappears after a few seconds.
    pub fn show_transient_message(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.flash_status(message, cx);
    }

    // ---- line/row caches --------------------------------------------------

    pub(crate) fn rebuild_rows(&mut self) {
        self.viewport
            .rebuild_rows(&self.document, self.settings.tab_stop);
    }

    /// Bounded Ropey source for one hard line, excluding its hard break.
    pub(crate) fn line_source(
        document: &Document,
        line: LineIndex,
    ) -> chaus_layout::LineSource<'_> {
        ViewportState::line_source(document, line)
    }

    /// Update wrap metadata for only the touched hard lines.
    pub(crate) fn refresh_rows(&mut self, affected: LineRange) {
        self.viewport
            .refresh_rows(&self.document, affected, self.settings.tab_stop);
    }

    pub(crate) fn total_rows(&mut self) -> usize {
        self.viewport.total_rows()
    }

    /// Total visual rows at the current wrap width, for diagnostics and tests.
    ///
    /// A caller editing a large document can compare this against a fresh
    /// rebuild to prove the incremental row cache stayed consistent.
    #[must_use]
    pub fn visual_row_count(&mut self) -> usize {
        self.total_rows()
    }

    /// First visual row of `line`, for diagnostics and tests.
    #[must_use]
    pub fn visual_row_start(&mut self, line: LineIndex) -> usize {
        self.viewport.row_prefix(line.0)
    }

    /// How many document rows have been shaped for painting so far.
    #[must_use]
    pub fn shaped_row_calls(&self) -> u64 {
        self.viewport.shaped_row_calls()
    }

    /// Bounded-work counters for the Ropey-backed layout path.
    #[must_use]
    pub fn layout_work_counters(&self) -> LayoutWorkCounters {
        self.viewport.layout_work_counters()
    }

    /// Break policy implied by the current wrap mode.
    #[must_use]
    pub(crate) fn wrap_policy(mode: WrapMode) -> chaus_layout::WrapPolicy {
        ViewportState::wrap_policy(mode)
    }

    /// Request a new wrap width. The committed row index keeps the previous
    /// width until a background job measures the new one and its generation is
    /// still current, so every frame derives rows and paint from one layout.
    pub(crate) fn set_wrap_layout(
        &mut self,
        columns: Option<usize>,
        policy: chaus_layout::WrapPolicy,
        cx: &mut Context<Self>,
    ) {
        if self.viewport.is_wrap_locked() {
            return;
        }
        if self.viewport.request_wrap(columns, policy) {
            self.spawn_reflow(cx);
        }
    }

    /// Freeze the width derived from paint geometry, for deterministic tests.
    pub fn lock_wrap_width_for_test(&mut self) {
        self.viewport.lock_wrap_width();
    }

    fn spawn_reflow(&mut self, cx: &mut Context<Self>) {
        let (generation, columns, policy) = self.viewport.begin_reflow();
        let tab_stop = self.settings.tab_stop;
        let revision = self.document.revision();
        let snapshot = self.document.snapshot();
        let task = cx.background_executor().spawn(async move {
            wait_for_reflow_gate();
            ViewportState::measure_rows(&snapshot, columns, tab_stop, policy)
        });
        cx.spawn(async move |editor, cx| {
            let counts = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_reflow(generation, revision, counts, cx);
                })
                .ok();
        })
        .detach();
    }

    /// Publish a finished measurement only when its generation and document
    /// revision still match. A superseded or stale job is discarded and the
    /// latest width is measured instead; no partial layout is ever published.
    pub(crate) fn finish_reflow(
        &mut self,
        generation: u64,
        revision: chaus_core::position::DocumentRevision,
        counts: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        match self
            .viewport
            .finish_reflow(generation, revision, self.document.revision(), counts)
        {
            ReflowOutcome::Stale => {}
            ReflowOutcome::Retry => self.spawn_reflow(cx),
            ReflowOutcome::Published => {
                self.clamp_scroll();
                cx.notify();
            }
        }
    }

    #[must_use]
    pub fn reflow_generation(&self) -> u64 {
        self.viewport.reflow_generation()
    }

    #[must_use]
    pub fn reflow_publications(&self) -> u64 {
        self.viewport.reflow_publications()
    }

    #[must_use]
    pub fn is_reflowing(&self) -> bool {
        self.viewport.is_reflowing()
    }

    #[must_use]
    pub fn committed_wrap_columns(&self) -> Option<usize> {
        self.viewport.committed_columns()
    }

    /// Programmatic wrap width, also used by deterministic reflow tests.
    pub fn set_wrap_columns(&mut self, columns: Option<usize>, cx: &mut Context<Self>) {
        let policy = self.viewport.policy();
        if self.viewport.request_wrap(columns, policy) {
            self.spawn_reflow(cx);
        }
    }

    /// Force a synchronous measurement for deterministic layout oracles.
    pub fn rebuild_rows_for_test(&mut self) {
        self.rebuild_rows();
    }

    /// Visual rows of one hard line at the current wrap width, for
    /// diagnostics and tests.
    #[must_use]
    pub fn visual_segments(&self, line: LineIndex) -> Vec<chaus_layout::Segment> {
        self.segments_for(line)
    }

    pub(crate) fn checkpoints_for(&self, line: LineIndex) -> chaus_layout::WrapCheckpoints {
        self.viewport.checkpoints_for(line)
    }

    pub(crate) fn column_checkpoints_for(
        &self,
        line: LineIndex,
    ) -> chaus_layout::ColumnCheckpoints {
        self.viewport.column_checkpoints_for(line)
    }

    pub(crate) fn segments_for(&self, line: LineIndex) -> Vec<chaus_layout::Segment> {
        self.viewport
            .segments_for(&self.document, self.settings.tab_stop, line)
    }

    pub(crate) fn local_row_of(&self, offset: CharOffset, line: LineIndex) -> usize {
        self.viewport
            .local_row_of(&self.document, self.settings.tab_stop, offset, line)
    }

    pub(crate) fn row_of_offset(&mut self, offset: CharOffset) -> (LineIndex, usize) {
        self.viewport
            .row_of_offset(&self.document, self.settings.tab_stop, offset)
    }

    /// Resolve a document-wide visual row to its hard line and visual columns.
    pub(crate) fn row_span(
        &mut self,
        row: usize,
    ) -> (LineIndex, Range<chaus_layout::VisualColumn>) {
        self.viewport
            .row_span(&self.document, self.settings.tab_stop, row)
    }

    pub(crate) fn caret_column(&self, offset: CharOffset) -> usize {
        self.viewport
            .caret_column(&self.document, self.settings.tab_stop, offset)
    }

    pub(crate) fn offset_at(
        &mut self,
        line: LineIndex,
        column: chaus_layout::VisualColumn,
    ) -> CharOffset {
        self.viewport
            .offset_at(&self.document, self.settings.tab_stop, line, column)
    }

    pub(crate) fn clamp_scroll(&mut self) {
        self.viewport
            .clamp_scroll(&self.document, self.settings.tab_stop);
    }
}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Editor {
    /// Key context that owns input right now.
    ///
    /// Every modal sheet removes `KEY_CONTEXT` from the dispatch path; this is
    /// the one place that decides it. Recovery's asynchronous arrival is still
    /// handled by its separate interceptor, not by a special case here.
    #[must_use]
    pub(crate) fn key_context(&self) -> &'static str {
        if self.modal_sheet_open() {
            MODAL_KEY_CONTEXT
        } else if self.vim.enabled() && !self.vim.mode().accepts_text() {
            // Normal/Visual/Command modes own the key context; Insert mode and
            // the command line keep text input on the normal editor path.
            VIM_OWNED_KEY_CONTEXT
        } else {
            KEY_CONTEXT
        }
    }
}

/// Bind editor actions to `Editor` methods in one place.
///
/// This adapter is the only place that translates GPUI actions into editor
/// commands. Menus, key bindings and tests all end up invoking the same methods
/// from here; no action has its own module or macro.
fn bind_editor_actions(
    element: gpui::Stateful<Div>,
    cx: &mut Context<Editor>,
) -> gpui::Stateful<Div> {
    element
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
        .on_action(
            cx.listener(|editor, action: &TrimTrailingWhitespace, window, cx| {
                editor.trim_trailing_whitespace_action(action, window, cx);
            }),
        )
        .on_action(
            cx.listener(|editor, action: &ConvertIndentationToSpaces, window, cx| {
                editor.convert_indentation_to_spaces_action(action, window, cx);
            }),
        )
        .on_action(
            cx.listener(|editor, action: &ConvertIndentationToTabs, window, cx| {
                editor.convert_indentation_to_tabs_action(action, window, cx);
            }),
        )
        .on_action(cx.listener(|editor, action: &FormatDocument, window, cx| {
            editor.format_document_action(action, window, cx);
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
        .on_action(cx.listener(|editor, action: &ZoomIn, _, cx| {
            editor.zoom_in_action(action, cx);
        }))
        .on_action(cx.listener(|editor, action: &ZoomOut, _, cx| {
            editor.zoom_out_action(action, cx);
        }))
        .on_action(cx.listener(|editor, action: &ZoomReset, _, cx| {
            editor.zoom_reset_action(action, cx);
        }))
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_blink(cx);
        self.ensure_menu(window, cx);
        if let Some((kind, position)) = self.menu.take_request() {
            self.open_menu(kind, position, window, cx);
        }
        self.sync_command_state(cx);
        let theme = cx.theme().clone();
        let title = self.title();
        window.set_window_title(&title);
        let background = silvestris::convert::hsla(theme.colors.content);
        let status = self.status.text().map(str::to_owned);
        let disk_banner = self
            .file
            .change()
            .is_some()
            .then(|| self.render_disk_banner(cx));
        let find_bar = self
            .find_bar
            .is_some()
            .then(|| self.render_find_bar(window, cx));
        let close_prompt = self.close_prompt.then(|| self.render_close_prompt(cx));
        let conflict_prompt = self
            .conflict_prompt
            .is_some()
            .then(|| self.render_conflict_prompt(cx));
        let recovery_prompt = (!self.recovery.is_empty()).then(|| self.render_recovery_prompt(cx));
        // Repair focus on the sheet's first frame when the background scan
        // landed after this window focused the document. The app-level key
        // interceptor is what actually keeps keys out of the document; this
        // only gives the buttons a real focus ring and mouse-focus parity.
        if !self.recovery.is_empty()
            && (self.focus.is_focused(window) || window.focused(cx).is_none())
        {
            window.focus(&self.recovery.recover_focus(), cx);
        }
        let command_line = self.render_command_line(window, &theme);
        let vim_label = if command_line.is_some() {
            None
        } else {
            self.vim_mode_label()
        };
        // Key bindings are dispatched before element key handlers. Any modal
        // sheet must therefore remove the editor key context at the dispatch
        // boundary; stopping propagation inside the sheet is too late.
        // `modal_sheet_open()` is the single definition of "the document must
        // not receive editing input". Recovery keeps its interceptor because
        // it can arrive after focus already exists; close/conflict sheets use
        // their own focused-button handlers once this context is removed.
        let key_context = self.key_context();
        let content = bind_editor_actions(
            div()
                .id("chaus-editor")
                .track_focus(&self.focus)
                // The focus handle must be tracked by an element in the rendered
                // tree, otherwise GPUI never adds `ChausEditor` to the key-dispatch
                // path and every action binding silently does nothing.
                .key_context(key_context)
                .size_full()
                .relative()
                .bg(background)
                .text_color(silvestris::convert::hsla(theme.colors.text))
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
                })),
            cx,
        )
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
        .when_some(command_line, |element, line| element.child(line))
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
        // No toolbar: the window is deliberately just the titlebar and the
        // document. `WindowFrame` owns left-button drag, double-click zoom and
        // the traffic lights. A window-level bubble right-click listener
        // replaces only the frame's native window menu with Chaus's command
        // menu, so the two hit targets never compete.
        let frame = silvestris::chrome::WindowFrame::new(title)
            .role(silvestris::window_role::WindowRole::Document)
            .edited(self.document.is_dirty())
            .child(content);
        let mut root = div().relative().size_full().child(frame);
        if let Some(popup) = self.menu.popup() {
            root = root.child(gpui::deferred(popup).priority(1));
        }
        // The command scope must use the same context as the editor root:
        // leaving it as `KEY_CONTEXT` would keep every editing and command
        // binding alive behind a modal sheet, defeating the modal context.
        silvestris::command::CommandScope::new(
            "chaus-command-scope",
            key_context,
            self.menu.commands().clone(),
        )
        .child(root)
    }
}

/// Which expensive work a settings change actually requires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SettingsEffect {
    /// Measurement inputs changed (wrap, width, tabs, shaping or theme).
    remeasure: bool,
    /// Only pixel geometry changed; refresh scroll without re-wrapping.
    geometry: bool,
}

fn settings_effect(previous: &Settings, next: &Settings) -> SettingsEffect {
    let wrap = previous.wrap != next.wrap;
    let remeasure = wrap
        || previous.tab_stop != next.tab_stop
        || (previous.font_size - next.font_size).abs() > f32::EPSILON
        || previous.font_family != next.font_family
        || previous.theme != next.theme;
    let geometry = (previous.line_height - next.line_height).abs() > f32::EPSILON
        || (previous.font_size - next.font_size).abs() > f32::EPSILON;
    SettingsEffect {
        remeasure,
        geometry,
    }
}
