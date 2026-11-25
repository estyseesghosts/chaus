//! The `Editor` entity: document, caret, scroll and per-window settings.

use std::collections::VecDeque;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use chaus_core::document::{Document, LineRange};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use chaus_core::settings::{Settings, WrapMode};
use chaus_highlight::{LanguageId, Span};
use chaus_io::recovery::RecoveryRecord;
use chaus_io::LoadedFile;
use chaus_vim::Engine;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, App, Context, CursorStyle, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, Render, ScrollEvent, Styled as _, Window,
};
use silvestris::ActiveTheme;

use crate::editor_close::ClosePromptFocus;
use crate::editor_commands::MenuKind;
use crate::editor_conflict::ConflictPromptFocus;
use crate::editor_element::EditorElement;
use crate::editor_file::FileState;
use crate::editor_file::LoadHook;
use crate::editor_find::FindBar;
use crate::editor_keys::{
    Backspace, ConvertIndentationToSpaces, ConvertIndentationToTabs, Copy, Cut, Delete,
    DeleteWordLeft, DeleteWordRight, DocumentEnd, DocumentStart, Down, End, Escape, Find, FindNext,
    FindPrevious, FormatDocument, Home, InsertNewline, InsertTab, Left, Outdent, Paste, Redo,
    Replace, Right, Save, SaveAs, SelectAll, SelectDocumentEnd, SelectDocumentStart, SelectDown,
    SelectEnd, SelectHome, SelectLeft, SelectRight, SelectUp, SelectWordLeft, SelectWordRight,
    ToggleVim, TrimTrailingWhitespace, Undo, Up, WordLeft, WordRight, ZoomIn, ZoomOut, ZoomReset,
    KEY_CONTEXT, VIM_OWNED_KEY_CONTEXT,
};
use crate::editor_recovery::{new_recovery_id, RecoveryPromptFocus};
use crate::file_operation::{DocumentGeneration, FileOperation};
use crate::geometry::Geometry;

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

/// Work performed by the layout primitives, for long-line diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutWorkCounters {
    /// Whole hard lines materialized as `String` by paint/layout. Must stay
    /// zero while painting a giant single line.
    pub hard_lines_materialized: u64,
    /// Unicode scalars read from Ropey windows.
    pub source_chars_read: u64,
    /// Visual rows visited by the shared walker.
    pub rows_walked: u64,
    /// Rows skipped by starting at a checkpoint.
    pub rows_skipped_via_checkpoint: u64,
    /// UTF-8 bytes built into display strings for visible rows.
    pub display_bytes_built: u64,
}

/// One editor window.
pub struct Editor {
    pub(crate) document: Document,
    pub(crate) file: Option<FileState>,
    pub(crate) selection: Selection,
    /// Preferred horizontal offset within the current visual row, in display
    /// columns. Cleared by horizontal movement and edits that relocate the
    /// caret.
    pub(crate) preferred_column: Option<usize>,
    pub(crate) scroll_row: f32,
    pub(crate) scroll_x: Pixels,
    pub(crate) focus: FocusHandle,
    pub(crate) marked: Option<CharRange>,
    pub(crate) preedit_start: Option<CharOffset>,
    pub(crate) preedit_active: bool,
    pub(crate) selecting: bool,
    pub(crate) geometry: Option<Geometry>,
    /// Per-line visual-row counts with an `O(log lines)` prefix index.
    pub(crate) rows: crate::row_index::RowIndex,
    /// Numeric wrap checkpoints per hard line, rebuilt with the row counts.
    pub(crate) wrap_checkpoints: Vec<chaus_layout::WrapCheckpoints>,
    /// Bounded-work counters for the long-line layout path.
    pub(crate) layout_work: LayoutWorkCounters,
    /// Shaped visible rows reused across text-preserving repaints.
    pub(crate) shaped_rows: crate::paint_cache::ShapedRowCache,
    /// Number of `shape_line` calls made for document rows, for diagnostics.
    pub(crate) shaped_row_calls: u64,
    pub(crate) rows_columns: Option<usize>,
    /// Break policy used by the row cache. Kept next to the width so a change
    /// between word and character wrapping invalidates the rows even when the
    /// column count did not change.
    pub(crate) rows_policy: chaus_layout::WrapPolicy,
    /// Tests freeze the paint-driven width so their explicit reflow requests
    /// cannot be overridden by the test window's own geometry.
    pub(crate) wrap_width_locked: bool,
    /// Latest requested reflow. Publication only happens when the finished
    /// generation still matches this one and the document revision is intact.
    pub(crate) reflow_generation: u64,
    pub(crate) reflow_in_flight: Option<u64>,
    pub(crate) reflow_columns: Option<usize>,
    pub(crate) reflow_policy: chaus_layout::WrapPolicy,
    pub(crate) reflow_publications: u64,
    pub(crate) settings: Settings,
    /// Per-window zoom override in logical pixels. `None` follows
    /// `settings.font_size`; this never writes back to the rc file.
    pub(crate) zoom: Option<f32>,
    pub(crate) status: Option<String>,
    pub(crate) status_epoch: u64,
    pub(crate) find_bar: Option<FindBar>,
    pub(crate) close_prompt: bool,
    pub(crate) close_focus: ClosePromptFocus,
    pub(crate) conflict_prompt: Option<chaus_io::SaveConflict>,
    pub(crate) conflict_focus: ConflictPromptFocus,
    pub(crate) recovery_queue: VecDeque<RecoveryRecord>,
    pub(crate) recovery_focus: RecoveryPromptFocus,
    pub(crate) recovery_key: u64,
    pub(crate) recovery_epoch: u64,
    /// Original path/bom retained by a recovered document until it is saved.
    /// The document itself stays untitled so recovery never overwrites the
    /// original file by accident.
    pub(crate) recovery_original_path: Option<PathBuf>,
    pub(crate) recovery_had_utf8_bom: bool,
    pub(crate) disk_change: Option<chaus_io::SaveConflict>,
    pub(crate) disk_epoch: u64,
    /// Set while a watcher conflict check is running, so a slow poll of a large
    /// file is not stacked behind a faster one.
    pub(crate) disk_check_in_flight: bool,
    pub(crate) force_next_save: bool,
    pub(crate) vim: bool,
    pub(crate) vim_engine: Engine,
    pub(crate) caret_visible: bool,
    pub(crate) blink_epoch: u64,
    pub(crate) blink_key: Option<(Selection, bool)>,
    pub(crate) mapping_epoch: u64,
    /// Test seam for a deterministic formatter invocation.
    pub(crate) formatter_override: Option<chaus_io::format::Formatter>,
    /// At most one formatter job owns a document at a time.
    pub(crate) format_in_flight: bool,
    pub(crate) format_generation: u64,
    /// One authoritative owner for picker, open and save work.
    pub(crate) file_operation: FileOperation,
    pub(crate) file_request_next: u64,
    /// `:wqa` participation: set on every window until the coordinator either
    /// closes them or aborts. A failure leaves all windows open.
    pub(crate) wqa_active: bool,
    pub(crate) wqa_failure: Option<String>,
    /// Test seam for holding a load in flight. Production leaves this `None`.
    pub(crate) load_hook: Option<LoadHook>,
    pub(crate) document_generation: DocumentGeneration,
    /// Set when a key-down was consumed by the modal engine. Windows delivers
    /// the printable character in a separate WM_CHAR message afterwards, so
    /// the next text-input callback must be ignored to avoid inserting the
    /// command itself (`j`, `i`, `:`).
    pub(crate) suppress_text_input: bool,
    /// Per-window command registry used by the titlebar context menu.
    pub(crate) commands: Entity<silvestris::command::CommandRegistry>,
    /// Popup menu overlay, created on the first render when a window exists.
    pub(crate) menu: Option<Entity<silvestris::menu::PopupMenu>>,
    /// Right-click in the title band, captured at window level and opened on
    /// the next render.
    pub(crate) pending_menu: Option<(MenuKind, gpui::Point<Pixels>)>,
    /// Detected or explicitly chosen syntax language.
    pub(crate) language: LanguageId,
    /// True when the person chose a language instead of extension detection.
    pub(crate) language_override: bool,
    /// Spans for `highlights_revision`; never painted for another revision.
    pub(crate) highlights: std::sync::Arc<Vec<Span>>,
    pub(crate) highlights_revision: Option<chaus_core::position::DocumentRevision>,
    pub(crate) highlight_tree: Option<chaus_highlight::Tree>,
    /// Revision the accepted tree was parsed from. Incremental reuse is only
    /// valid when this is exactly the revision before the pending edit.
    pub(crate) highlight_tree_revision: Option<chaus_core::position::DocumentRevision>,
    pub(crate) highlight_generation: u64,
    pub(crate) highlight_applied_generation: u64,
    pub(crate) highlight_worker_running: bool,
    pub(crate) highlight_pending_edit: Option<crate::editor_highlight::PendingHighlightEdit>,
    pub(crate) highlight_statistics: crate::editor_highlight::HighlightStatistics,
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
        let language = file
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
        let recovery_focus = RecoveryPromptFocus::new(cx);
        let commands = Self::create_command_registry(cx);
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
            rows: crate::row_index::RowIndex::default(),
            wrap_checkpoints: Vec::new(),
            layout_work: LayoutWorkCounters::default(),
            shaped_rows: crate::paint_cache::ShapedRowCache::default(),
            shaped_row_calls: 0,
            rows_columns: None,
            rows_policy: chaus_layout::WrapPolicy::Word,
            wrap_width_locked: false,
            reflow_generation: 0,
            reflow_in_flight: None,
            reflow_columns: None,
            reflow_policy: chaus_layout::WrapPolicy::Word,
            reflow_publications: 0,
            settings,
            zoom: None,
            status: None,
            status_epoch: 0,
            find_bar: None,
            close_prompt: false,
            close_focus,
            conflict_prompt: None,
            conflict_focus,
            recovery_queue: VecDeque::new(),
            recovery_focus,
            recovery_key: new_recovery_id(),
            recovery_epoch: 0,
            recovery_original_path: None,
            recovery_had_utf8_bom: false,
            disk_change: None,
            disk_epoch: 0,
            disk_check_in_flight: false,
            force_next_save: false,
            vim,
            vim_engine: Engine::new(mappings),
            caret_visible: true,
            blink_epoch: 0,
            blink_key: None,
            mapping_epoch: 0,
            formatter_override: None,
            format_in_flight: false,
            format_generation: 0,
            file_operation: FileOperation::Idle,
            file_request_next: 0,
            wqa_active: false,
            wqa_failure: None,
            load_hook: None,
            document_generation: DocumentGeneration::default(),
            suppress_text_input: false,
            commands,
            menu: None,
            pending_menu: None,
            language,
            language_override: false,
            highlights: std::sync::Arc::new(Vec::new()),
            highlights_revision: None,
            highlight_tree: None,
            highlight_tree_revision: None,
            highlight_generation: 0,
            highlight_applied_generation: 0,
            highlight_worker_running: false,
            highlight_pending_edit: None,
            highlight_statistics: crate::editor_highlight::HighlightStatistics::default(),
        };
        let weak = cx.weak_entity();
        editor.register_editor_commands(weak, cx);
        // Command chords belong to the registry, not to `bind_keys`. Applying
        // them here keeps a standalone editor (tests, embedding) consistent
        // while `chaus-app` re-applies after adding the application commands.
        if let Err(error) = silvestris::command::apply_bindings(
            &editor.commands,
            &[crate::editor_keys::KEY_CONTEXT],
            cx,
        ) {
            eprintln!("chaus: could not bind document commands: {error}");
        }
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
        self.file
            .as_ref()
            .map(|file| file.meta.user_requested_path.as_path())
    }

    /// Replace the selection and caret without changing the document.
    ///
    /// Keeps anchor/head direction, so callers can restore a reversed
    /// selection or select a range from search and navigation code.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = selection.clamp(self.document.len_chars());
        self.preferred_column = None;
        cx.notify();
    }

    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Effective document font size for this window, including zoom.
    #[must_use]
    pub fn editor_font_size(&self) -> f32 {
        self.zoom.unwrap_or(self.settings.font_size)
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
        self.disk_check_in_flight
    }

    /// This window's command registry. Application-level commands (New, Open,
    /// Preferences) are registered on it by the app so the titlebar menu uses
    /// one tree.
    #[must_use]
    pub fn commands(&self) -> &Entity<silvestris::command::CommandRegistry> {
        &self.commands
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
            // `finish_reflow`. Committing either field here would pair a new
            // policy with an old row index for a frame. The generation check
            // drops a result that is stale when it lands.
            let columns = if self.settings.wrap == WrapMode::None {
                None
            } else {
                self.rows_columns
            };
            self.reflow_columns = columns;
            self.reflow_policy = Self::wrap_policy(self.settings.wrap);
            self.reflow_generation = self.reflow_generation.wrapping_add(1);
            if self.reflow_in_flight.is_none() {
                self.spawn_reflow(cx);
            }
        }
        if effect.geometry {
            self.clamp_scroll();
        }
        cx.notify();
    }

    /// Install a deterministic stand-in for a background load.
    pub fn set_load_hook_for_test(&mut self, hook: LoadHook) {
        self.load_hook = Some(hook);
    }

    /// Use this formatter instead of detecting one from the file extension.
    pub fn set_formatter_for_test(&mut self, formatter: chaus_io::format::Formatter) {
        self.formatter_override = Some(formatter);
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
        self.vim
    }

    /// The document name for chrome and the OS window. Dirty state belongs to
    /// the window frame's edited marker, not to the title string.
    #[must_use]
    pub fn title(&self) -> String {
        self.file
            .as_ref()
            .map(|file| {
                file.meta
                    .user_requested_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.meta.user_requested_path.display().to_string())
            })
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub(crate) fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
        self.status_epoch = self.status_epoch.wrapping_add(1);
    }

    /// Show a routine confirmation that disappears unless another message
    /// replaces it first. The epoch prevents an old timer from clearing a newer
    /// message, and persistent warnings set through [`Self::set_status`] bump
    /// the epoch too.
    pub(crate) fn flash_status(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.set_status(message);
        let epoch = self.status_epoch;
        cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(TRANSIENT_STATUS).await;
            editor
                .update(cx, |editor, cx| {
                    if editor.status_epoch == epoch && editor.status.is_some() {
                        editor.status = None;
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
        self.status.as_deref()
    }

    /// The file this window's document is associated with, if any.
    #[must_use]
    pub fn file_state(&self) -> Option<&crate::editor_file::FileState> {
        self.file.as_ref()
    }

    /// Text of the open Vim command line, without its prefix.
    #[must_use]
    pub fn command_line_text(&self) -> Option<String> {
        self.vim_engine
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
        let line_count = self.document.len_lines();
        self.wrap_checkpoints
            .resize_with(line_count, chaus_layout::WrapCheckpoints::default);
        let mut counts = Vec::with_capacity(line_count);
        for line in 0..line_count {
            counts.push(self.measure_line(line));
        }
        self.rows.rebuild(counts);
    }

    /// Bounded Ropey source for one hard line, excluding its hard break.
    pub(crate) fn line_source(
        document: &chaus_core::document::Document,
        line: LineIndex,
    ) -> chaus_layout::LineSource<'_> {
        let start = document.line_start(line).0;
        let end = document.line_content_end(line).0;
        chaus_layout::LineSource::from_rope(document.rope().slice(start..end))
    }

    /// Measure one hard line and store its numeric checkpoints instead of a
    /// cached copy of the line's text.
    fn measure_line(&mut self, line: usize) -> usize {
        let (count, checkpoints) = self.measure_line_data(LineIndex(line));
        if line >= self.wrap_checkpoints.len() {
            self.wrap_checkpoints
                .resize_with(line + 1, chaus_layout::WrapCheckpoints::default);
        }
        self.wrap_checkpoints[line] = checkpoints;
        count
    }

    fn measure_line_data(&self, line: LineIndex) -> (usize, chaus_layout::WrapCheckpoints) {
        let source = Self::line_source(&self.document, line);
        let checkpoints = chaus_layout::build_checkpoints(
            &source,
            self.rows_columns,
            self.settings.tab_stop,
            self.rows_policy,
            128,
        );
        let count = chaus_layout::row_count_source(
            &source,
            self.rows_columns,
            self.settings.tab_stop,
            self.rows_policy,
            &checkpoints,
        );
        (count, checkpoints)
    }

    /// Update wrap metadata for only the touched hard lines.
    ///
    /// A point edit updates one Fenwick leaf in `O(log lines)`. When the edit
    /// inserts or removes hard lines, only the affected range is re-wrapped and
    /// spliced into the measured counts for the rest; the tree is rebuilt from
    /// integers, never from Unicode layout work for untouched lines.
    pub(crate) fn refresh_rows(&mut self, affected: LineRange) {
        let line_count = self.document.len_lines();
        let start = affected.start.0.min(line_count);
        let end = affected.end.0.min(line_count);
        if start >= end {
            if self.rows.len() != line_count {
                self.rebuild_rows();
            }
            return;
        }
        if self.rows.len() != line_count {
            let delta = line_count as i64 - self.rows.len() as i64;
            let old_end = (end as i64 - delta).max(start as i64) as usize;
            let mut counts = Vec::with_capacity(end - start);
            let mut checkpoints = Vec::with_capacity(end - start);
            for line in start..end {
                let (count, index) = self.measure_line_data(LineIndex(line));
                counts.push(count);
                checkpoints.push(index);
            }
            self.rows.splice(start, old_end, counts);
            let old_checkpoints_end = old_end.min(self.wrap_checkpoints.len());
            self.wrap_checkpoints.splice(
                start.min(old_checkpoints_end)..old_checkpoints_end,
                checkpoints,
            );
            self.wrap_checkpoints
                .resize_with(line_count, chaus_layout::WrapCheckpoints::default);
            return;
        }
        for line in start..end {
            let rows = self.measure_line(line);
            self.rows.set(LineIndex(line), rows);
        }
    }

    pub(crate) fn total_rows(&mut self) -> usize {
        self.rows.total().max(1)
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
    ///
    /// Comparing this against the summed `visual_segments` of the preceding
    /// lines is the invariant the incremental prefix must preserve.
    #[must_use]
    pub fn visual_row_start(&mut self, line: LineIndex) -> usize {
        self.rows.prefix(line.0)
    }

    /// How many document rows have been shaped for painting so far.
    ///
    /// Diagnostics only: a repaint that changes no text, font, theme or wrap
    /// width should leave this unchanged after the first frame.
    #[must_use]
    pub fn shaped_row_calls(&self) -> u64 {
        self.shaped_row_calls
    }

    /// Bounded-work counters for the Ropey-backed layout path. A giant hard
    /// line must never materialize the whole line through these paths.
    #[must_use]
    pub fn layout_work_counters(&self) -> LayoutWorkCounters {
        self.layout_work
    }

    /// Break policy implied by the current wrap mode.
    #[must_use]
    pub(crate) fn wrap_policy(mode: WrapMode) -> chaus_layout::WrapPolicy {
        match mode {
            WrapMode::Word => chaus_layout::WrapPolicy::Word,
            _ => chaus_layout::WrapPolicy::Character,
        }
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
        if self.wrap_width_locked {
            return;
        }
        self.request_wrap_layout(columns, policy, cx);
    }

    /// Freeze the width derived from paint geometry, for deterministic tests.
    pub fn lock_wrap_width_for_test(&mut self) {
        self.wrap_width_locked = true;
    }

    fn request_wrap_layout(
        &mut self,
        columns: Option<usize>,
        policy: chaus_layout::WrapPolicy,
        cx: &mut Context<Self>,
    ) {
        if self.rows_columns == columns && self.rows_policy == policy {
            if self.reflow_columns != columns || self.reflow_policy != policy {
                self.reflow_columns = columns;
                self.reflow_policy = policy;
                self.reflow_generation = self.reflow_generation.wrapping_add(1);
            }
            return;
        }
        if self.reflow_columns == columns && self.reflow_policy == policy {
            return;
        }
        self.reflow_columns = columns;
        self.reflow_policy = policy;
        self.reflow_generation = self.reflow_generation.wrapping_add(1);
        if self.reflow_in_flight.is_none() {
            self.spawn_reflow(cx);
        }
    }

    /// Programmatic wrap width, also used by deterministic reflow tests.
    pub fn set_wrap_columns(&mut self, columns: Option<usize>, cx: &mut Context<Self>) {
        let policy = self.rows_policy;
        self.request_wrap_layout(columns, policy, cx);
    }

    fn spawn_reflow(&mut self, cx: &mut Context<Self>) {
        let generation = self.reflow_generation;
        let columns = self.reflow_columns;
        let policy = self.reflow_policy;
        let tab_stop = self.settings.tab_stop;
        let revision = self.document.revision();
        let snapshot = self.document.snapshot();
        self.reflow_in_flight = Some(generation);
        let task = cx.background_executor().spawn(async move {
            wait_for_reflow_gate();
            measure_rows(&snapshot, columns, tab_stop, policy)
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
        if self.reflow_in_flight != Some(generation) {
            return;
        }
        self.reflow_in_flight = None;
        if generation == self.reflow_generation && revision == self.document.revision() {
            self.rows_columns = self.reflow_columns;
            self.rows_policy = self.reflow_policy;
            self.rows.rebuild(counts);
            self.reflow_publications = self.reflow_publications.wrapping_add(1);
            self.clamp_scroll();
            cx.notify();
            return;
        }
        self.spawn_reflow(cx);
    }

    #[must_use]
    pub fn reflow_generation(&self) -> u64 {
        self.reflow_generation
    }

    #[must_use]
    pub fn reflow_publications(&self) -> u64 {
        self.reflow_publications
    }

    #[must_use]
    pub fn is_reflowing(&self) -> bool {
        self.reflow_in_flight.is_some()
    }

    #[must_use]
    pub fn committed_wrap_columns(&self) -> Option<usize> {
        self.rows_columns
    }

    /// Force a synchronous measurement for deterministic layout oracles.
    pub fn rebuild_rows_for_test(&mut self) {
        self.rebuild_rows();
    }

    /// Visual rows of one hard line at the current wrap width, for
    /// diagnostics and tests. These are the exact rows used for painting,
    /// caret placement and hit testing.
    #[must_use]
    pub fn visual_segments(&self, line: LineIndex) -> Vec<chaus_layout::Segment> {
        self.segments_for(line)
    }

    pub(crate) fn checkpoints_for(&self, line: LineIndex) -> chaus_layout::WrapCheckpoints {
        self.wrap_checkpoints
            .get(line.0)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn segments_for(&self, line: LineIndex) -> Vec<chaus_layout::Segment> {
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        chaus_layout::segments_window_source(
            &source,
            self.rows_columns,
            self.settings.tab_stop,
            self.rows_policy,
            chaus_layout::WrapRowIndex::new(0),
            usize::MAX,
            &checkpoints,
        )
        .0
    }

    pub(crate) fn local_row_of(&self, offset: CharOffset, line: LineIndex) -> usize {
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let local_char = offset.0.saturating_sub(self.document.line_start(line).0);
        chaus_layout::row_of_char_source(
            &source,
            self.rows_columns,
            self.settings.tab_stop,
            self.rows_policy,
            chaus_layout::LineCharOffset::new(local_char),
            &checkpoints,
        )
        .row
        .get()
    }

    pub(crate) fn row_of_offset(&mut self, offset: CharOffset) -> (LineIndex, usize) {
        let line = self.document.line_of(offset);
        let row_in_line = self.local_row_of(offset, line);
        (line, self.rows.prefix(line.0) + row_in_line)
    }

    /// Resolve a document-wide visual row to its hard line and the visual
    /// columns covered by that row.
    pub(crate) fn row_span(&mut self, row: usize) -> (LineIndex, Range<usize>) {
        let line = self.line_at_row(row);
        let local_row = row.saturating_sub(self.rows.prefix(line.0));
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let span = chaus_layout::segment_at_source(
            &source,
            self.rows_columns,
            self.settings.tab_stop,
            self.rows_policy,
            chaus_layout::WrapRowIndex::new(local_row),
            &checkpoints,
        )
        .map_or(0..0, |segment| {
            segment.start_column.get()..segment.end_column().get()
        });
        (line, span)
    }

    pub(crate) fn caret_column(&self, offset: CharOffset) -> usize {
        let line = self.document.line_of(offset);
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let local = offset.0.saturating_sub(self.document.line_start(line).0);
        chaus_layout::column_at_source(
            &source,
            chaus_layout::LineCharOffset::new(local),
            self.settings.tab_stop,
            &checkpoints,
        )
        .get()
    }

    pub(crate) fn offset_at(&mut self, line: LineIndex, column: usize) -> CharOffset {
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let local = chaus_layout::char_at_column_source(
            &source,
            chaus_layout::VisualColumn::new(column),
            self.settings.tab_stop,
            &checkpoints,
        );
        CharOffset(self.document.line_start(line).0 + local.get())
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
        self.ensure_menu(window, cx);
        if let Some((kind, position)) = self.pending_menu.take() {
            self.open_menu(kind, position, window, cx);
        }
        self.sync_command_state(cx);
        let theme = cx.theme().clone();
        let title = self.title();
        window.set_window_title(&title);
        let background = silvestris::convert::hsla(theme.colors.content);
        let status = self.status.clone();
        let disk_banner = self
            .disk_change
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
        let recovery_prompt =
            (!self.recovery_queue.is_empty()).then(|| self.render_recovery_prompt(cx));
        let command_line = self.render_command_line(window, &theme);
        let vim_label = if command_line.is_some() {
            None
        } else {
            self.vim_mode_label()
        };
        let vim_owns_keys = self.vim && !self.vim_engine.mode().accepts_text();
        let key_context = if vim_owns_keys {
            VIM_OWNED_KEY_CONTEXT
        } else {
            KEY_CONTEXT
        };
        let content = div()
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
        if let Some(menu) = self.menu.clone() {
            root = root.child(gpui::deferred(menu).priority(1));
        }
        silvestris::command::CommandScope::new(
            "chaus-command-scope",
            KEY_CONTEXT,
            self.commands.clone(),
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

fn measure_rows(
    snapshot: &chaus_core::document::DocumentSnapshot,
    columns: Option<usize>,
    tab_stop: usize,
    policy: chaus_layout::WrapPolicy,
) -> Vec<usize> {
    let checkpoints = chaus_layout::WrapCheckpoints::default();
    (0..snapshot.line_count())
        .map(|line| {
            let slice = snapshot.line_slice(line);
            let source = chaus_layout::LineSource::from_rope(slice);
            chaus_layout::row_count_source(&source, columns, tab_stop, policy, &checkpoints)
        })
        .collect()
}
