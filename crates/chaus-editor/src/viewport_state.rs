//! Visual layout and scroll state for one editor window.
//!
//! `ViewportState` owns the preferred row-local column, vertical/horizontal
//! scroll, committed geometry, the incremental row index with wrap and column
//! checkpoints, shaped-row cache and layout counters, the committed wrap
//! configuration, reflow request/in-flight/generation/publication state, and
//! the per-window font zoom. It does not own the document, selection, settings
//! or paint work: callers pass the document, tab stop and explicit values, and
//! `Editor` coordinates spawning and notification.

use std::ops::Range;

use chaus_core::document::{Document, DocumentSnapshot, LineRange};
use chaus_core::position::LineIndex;
use chaus_core::settings::WrapMode;
use chaus_layout::{ColumnCheckpoints, Segment, VisualColumn, WrapCheckpoints, WrapPolicy};
use gpui::{Pixels, ShapedLine};

use crate::geometry::Geometry;
use crate::paint_cache::{PaintKey, ShapedRowCache};
use crate::row_index::RowIndex;

/// Work performed by the layout primitives, for long-line diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutWorkCounters {
    /// Whole hard lines materialized as `String` by paint/layout. Must stay
    /// zero while painting a giant single line.
    pub hard_lines_materialized: u64,
    /// Unicode scalars read from Ropey windows on every path.
    pub source_chars_read: u64,
    /// Unicode scalars read on the unwrapped horizontal path.
    pub unwrapped_chars_read: u64,
    /// Visual rows visited by the shared walker.
    pub rows_walked: u64,
    /// Rows skipped by starting at a checkpoint.
    pub rows_skipped_via_checkpoint: u64,
    /// UTF-8 bytes built into display strings for visible rows.
    pub display_bytes_built: u64,
}

/// How a finished reflow was handled.
pub(crate) enum ReflowOutcome {
    /// A different job owns the in-flight slot; do nothing.
    Stale,
    /// The measurement was superseded; measure the newest request instead.
    Retry,
    /// The measurement was published; the caller should clamp scroll and notify.
    Published,
}

/// Committed and requested layout, scroll position and zoom.
pub(crate) struct ViewportState {
    preferred_column: Option<chaus_layout::RowColumn>,
    scroll_row: f32,
    scroll_x: Pixels,
    geometry: Option<Geometry>,
    rows: RowIndex,
    wrap_checkpoints: Vec<WrapCheckpoints>,
    column_checkpoints: Vec<ColumnCheckpoints>,
    layout_work: LayoutWorkCounters,
    shaped_rows: ShapedRowCache,
    shaped_row_calls: u64,
    rows_columns: Option<usize>,
    rows_policy: WrapPolicy,
    wrap_width_locked: bool,
    reflow_generation: u64,
    reflow_in_flight: Option<u64>,
    reflow_columns: Option<usize>,
    reflow_policy: WrapPolicy,
    reflow_publications: u64,
    zoom: Option<f32>,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewportState {
    pub(crate) fn new() -> Self {
        Self {
            preferred_column: None,
            scroll_row: 0.0,
            scroll_x: gpui::px(0.),
            geometry: None,
            rows: RowIndex::default(),
            wrap_checkpoints: Vec::new(),
            column_checkpoints: Vec::new(),
            layout_work: LayoutWorkCounters::default(),
            shaped_rows: ShapedRowCache::default(),
            shaped_row_calls: 0,
            rows_columns: None,
            rows_policy: WrapPolicy::Word,
            wrap_width_locked: false,
            reflow_generation: 0,
            reflow_in_flight: None,
            reflow_columns: None,
            reflow_policy: WrapPolicy::Word,
            reflow_publications: 0,
            zoom: None,
        }
    }

    // ---- simple accessors ---------------------------------------------------

    pub(crate) fn preferred_column(&self) -> Option<chaus_layout::RowColumn> {
        self.preferred_column
    }

    pub(crate) fn set_preferred_column(&mut self, column: Option<chaus_layout::RowColumn>) {
        self.preferred_column = column;
    }

    pub(crate) fn scroll_row(&self) -> f32 {
        self.scroll_row
    }

    pub(crate) fn set_scroll_row(&mut self, row: f32) {
        self.scroll_row = row;
    }

    pub(crate) fn advance_scroll_row(&mut self, delta: f32) {
        self.scroll_row += delta;
    }

    pub(crate) fn scroll_x(&self) -> Pixels {
        self.scroll_x
    }

    pub(crate) fn set_scroll_x(&mut self, x: Pixels) {
        self.scroll_x = x;
    }

    pub(crate) fn advance_scroll_x(&mut self, delta: Pixels) {
        self.scroll_x -= delta;
    }

    pub(crate) fn geometry(&self) -> Option<Geometry> {
        self.geometry
    }

    pub(crate) fn set_geometry(&mut self, geometry: Geometry) {
        self.geometry = Some(geometry);
    }

    pub(crate) fn viewport_rows(&self) -> usize {
        self.geometry
            .map(|geometry| geometry.visible_rows())
            .unwrap_or(1)
            .max(1)
    }

    pub(crate) fn visible_row_count(&self) -> Option<usize> {
        self.geometry.map(|geometry| geometry.visible_rows())
    }

    pub(crate) fn set_zoom(&mut self, zoom: Option<f32>) {
        self.zoom = zoom;
    }

    pub(crate) fn effective_font_size(&self, default: f32) -> f32 {
        self.zoom.unwrap_or(default)
    }

    /// Clear scroll and preferred column when the document is replaced.
    pub(crate) fn reset_scroll(&mut self) {
        self.preferred_column = None;
        self.scroll_row = 0.0;
        self.scroll_x = gpui::px(0.);
    }

    pub(crate) fn layout_work_counters(&self) -> LayoutWorkCounters {
        self.layout_work
    }

    pub(crate) fn record_layout_work(
        &mut self,
        chars: usize,
        unwrapped: usize,
        walked: usize,
        skipped: usize,
        display_bytes: usize,
    ) {
        self.layout_work.source_chars_read += chars as u64;
        self.layout_work.unwrapped_chars_read += unwrapped as u64;
        self.layout_work.rows_walked += walked as u64;
        self.layout_work.rows_skipped_via_checkpoint += skipped as u64;
        self.layout_work.display_bytes_built += display_bytes as u64;
    }

    pub(crate) fn shaped_row_calls(&self) -> u64 {
        self.shaped_row_calls
    }

    pub(crate) fn bump_shaped_row_calls(&mut self) {
        self.shaped_row_calls += 1;
    }

    pub(crate) fn shaped_row(
        &mut self,
        key: &PaintKey,
        line: usize,
        segment: usize,
    ) -> Option<ShapedLine> {
        self.shaped_rows.get(key, line, segment)
    }

    pub(crate) fn cache_shaped_row(
        &mut self,
        key: &PaintKey,
        line: usize,
        segment: usize,
        shaped: ShapedLine,
    ) {
        self.shaped_rows.insert(key, line, segment, shaped);
    }

    // ---- row cache ----------------------------------------------------------

    /// Bounded Ropey source for one hard line, excluding its hard break.
    pub(crate) fn line_source(
        document: &Document,
        line: LineIndex,
    ) -> chaus_layout::LineSource<'_> {
        let start = document.line_start(line).0;
        let end = document.line_content_end(line).0;
        chaus_layout::LineSource::from_rope(document.rope().slice(start..end))
    }

    pub(crate) fn rebuild_rows(&mut self, document: &Document, tab_stop: usize) {
        let line_count = document.len_lines();
        self.wrap_checkpoints
            .resize_with(line_count, WrapCheckpoints::default);
        self.column_checkpoints
            .resize_with(line_count, ColumnCheckpoints::default);
        let mut counts = Vec::with_capacity(line_count);
        for line in 0..line_count {
            counts.push(self.measure_line(document, tab_stop, line));
        }
        self.rows.rebuild(counts);
    }

    fn measure_line(&mut self, document: &Document, tab_stop: usize, line: usize) -> usize {
        let (count, checkpoints, columns) =
            self.measure_line_data(document, tab_stop, LineIndex(line));
        if line >= self.wrap_checkpoints.len() {
            self.wrap_checkpoints
                .resize_with(line + 1, WrapCheckpoints::default);
        }
        if line >= self.column_checkpoints.len() {
            self.column_checkpoints
                .resize_with(line + 1, ColumnCheckpoints::default);
        }
        self.wrap_checkpoints[line] = checkpoints;
        self.column_checkpoints[line] = columns;
        count
    }

    fn measure_line_data(
        &self,
        document: &Document,
        tab_stop: usize,
        line: LineIndex,
    ) -> (usize, WrapCheckpoints, ColumnCheckpoints) {
        let source = Self::line_source(document, line);
        let checkpoints = chaus_layout::build_checkpoints(
            &source,
            self.rows_columns,
            tab_stop,
            self.rows_policy,
            128,
        );
        let columns = chaus_layout::build_column_checkpoints(
            &source,
            tab_stop,
            chaus_layout::COLUMN_CHECKPOINT_STRIDE,
        );
        let count = chaus_layout::row_count_source(
            &source,
            self.rows_columns,
            tab_stop,
            self.rows_policy,
            &checkpoints,
        );
        (count, checkpoints, columns)
    }

    /// Re-measure only the touched hard lines. A point edit updates one Fenwick
    /// leaf in `O(log lines)`; inserting hard lines splices only the affected
    /// range and rebuilds the tree from integers.
    pub(crate) fn refresh_rows(
        &mut self,
        document: &Document,
        affected: LineRange,
        tab_stop: usize,
    ) {
        let line_count = document.len_lines();
        let start = affected.start.0.min(line_count);
        let end = affected.end.0.min(line_count);
        if start >= end {
            if self.rows.len() != line_count {
                self.rebuild_rows(document, tab_stop);
            }
            return;
        }
        if self.rows.len() != line_count {
            let delta = line_count as i64 - self.rows.len() as i64;
            let old_end = (end as i64 - delta).max(start as i64) as usize;
            let mut counts = Vec::with_capacity(end - start);
            let mut checkpoints = Vec::with_capacity(end - start);
            let mut columns = Vec::with_capacity(end - start);
            for line in start..end {
                let (count, index, column_index) =
                    self.measure_line_data(document, tab_stop, LineIndex(line));
                counts.push(count);
                checkpoints.push(index);
                columns.push(column_index);
            }
            self.rows.splice(start, old_end, counts);
            let old_checkpoints_end = old_end.min(self.wrap_checkpoints.len());
            self.wrap_checkpoints.splice(
                start.min(old_checkpoints_end)..old_checkpoints_end,
                checkpoints,
            );
            let old_columns_end = old_end.min(self.column_checkpoints.len());
            self.column_checkpoints
                .splice(start.min(old_columns_end)..old_columns_end, columns);
            self.wrap_checkpoints
                .resize_with(line_count, WrapCheckpoints::default);
            self.column_checkpoints
                .resize_with(line_count, ColumnCheckpoints::default);
            return;
        }
        for line in start..end {
            let rows = self.measure_line(document, tab_stop, line);
            self.rows.set(LineIndex(line), rows);
        }
    }

    pub(crate) fn total_rows(&self) -> usize {
        self.rows.total().max(1)
    }

    pub(crate) fn row_prefix(&self, line: usize) -> usize {
        self.rows.prefix(line)
    }

    pub(crate) fn line_at_row(&self, row: usize) -> LineIndex {
        self.rows.line_at_row(row)
    }

    pub(crate) fn checkpoints_for(&self, line: LineIndex) -> WrapCheckpoints {
        self.wrap_checkpoints
            .get(line.0)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn column_checkpoints_for(&self, line: LineIndex) -> ColumnCheckpoints {
        self.column_checkpoints
            .get(line.0)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn committed_columns(&self) -> Option<usize> {
        self.rows_columns
    }

    pub(crate) fn policy(&self) -> WrapPolicy {
        self.rows_policy
    }

    pub(crate) fn segments_for(
        &self,
        document: &Document,
        tab_stop: usize,
        line: LineIndex,
    ) -> Vec<Segment> {
        let source = Self::line_source(document, line);
        let checkpoints = self.checkpoints_for(line);
        chaus_layout::segments_window_source(
            &source,
            self.rows_columns,
            tab_stop,
            self.rows_policy,
            chaus_layout::WrapRowIndex::new(0),
            usize::MAX,
            &checkpoints,
        )
        .0
    }

    pub(crate) fn local_row_of(
        &self,
        document: &Document,
        tab_stop: usize,
        offset: chaus_core::position::CharOffset,
        line: LineIndex,
    ) -> usize {
        let source = Self::line_source(document, line);
        let checkpoints = self.checkpoints_for(line);
        let local_char = offset.0.saturating_sub(document.line_start(line).0);
        chaus_layout::row_of_char_source(
            &source,
            self.rows_columns,
            tab_stop,
            self.rows_policy,
            chaus_layout::LineCharOffset::new(local_char),
            &checkpoints,
        )
        .row
        .get()
    }

    pub(crate) fn row_of_offset(
        &self,
        document: &Document,
        tab_stop: usize,
        offset: chaus_core::position::CharOffset,
    ) -> (LineIndex, usize) {
        let line = document.line_of(offset);
        let row_in_line = self.local_row_of(document, tab_stop, offset, line);
        (line, self.rows.prefix(line.0) + row_in_line)
    }

    /// Resolve a document-wide visual row to its hard line and visual columns.
    pub(crate) fn row_span(
        &self,
        document: &Document,
        tab_stop: usize,
        row: usize,
    ) -> (LineIndex, Range<VisualColumn>) {
        let line = self.rows.line_at_row(row);
        let local_row = row.saturating_sub(self.rows.prefix(line.0));
        let source = Self::line_source(document, line);
        let checkpoints = self.checkpoints_for(line);
        let span = if self.rows_columns.is_none() {
            // The one unwrapped row spans the whole hard line. Its end column
            // comes from the horizontal index, never from a whole-line segment.
            let columns = self.column_checkpoints_for(line);
            let end = chaus_layout::line_columns(&source, tab_stop, &columns);
            VisualColumn::new(0)..end
        } else {
            chaus_layout::segment_at_source(
                &source,
                self.rows_columns,
                tab_stop,
                self.rows_policy,
                chaus_layout::WrapRowIndex::new(local_row),
                &checkpoints,
            )
            .map_or(VisualColumn::new(0)..VisualColumn::new(0), |segment| {
                segment.start_column..segment.end_column()
            })
        };
        (line, span)
    }

    pub(crate) fn caret_column(
        &self,
        document: &Document,
        tab_stop: usize,
        offset: chaus_core::position::CharOffset,
    ) -> usize {
        let line = document.line_of(offset);
        let source = Self::line_source(document, line);
        let checkpoints = self.column_checkpoints_for(line);
        let local = offset.0.saturating_sub(document.line_start(line).0);
        chaus_layout::column_at_source(
            &source,
            chaus_layout::LineCharOffset::new(local),
            tab_stop,
            &checkpoints,
        )
        .get()
    }

    pub(crate) fn offset_at(
        &self,
        document: &Document,
        tab_stop: usize,
        line: LineIndex,
        column: VisualColumn,
    ) -> chaus_core::position::CharOffset {
        let source = Self::line_source(document, line);
        let checkpoints = self.column_checkpoints_for(line);
        let local = chaus_layout::char_at_column_source(&source, column, tab_stop, &checkpoints);
        chaus_core::position::CharOffset(document.line_start(line).0 + local.get())
    }

    // ---- scroll -------------------------------------------------------------

    pub(crate) fn clamp_scroll(&mut self, document: &Document, tab_stop: usize) {
        let total = self.total_rows();
        let visible = self
            .geometry
            .map(|geometry| geometry.visible_rows())
            .unwrap_or(1);
        let max_row = total.saturating_sub(visible);
        self.scroll_row = self.scroll_row.clamp(0.0, max_row as f32);
        self.scroll_x = self.scroll_x.max(gpui::px(0.));
        // Wrapped rows always fit the viewport; horizontal scrolling is not
        // meaningful and must not be left over from a previous no-wrap mode.
        if self.rows_columns.is_some() {
            self.scroll_x = gpui::px(0.);
            return;
        }
        // Right-hand bound: the widest of the visible rows. Without it wheel
        // deltas can move every glyph off-screen and leave the caret pulling
        // the view back at an arbitrary later moment.
        let Some(geometry) = self.geometry else {
            self.scroll_x = gpui::px(0.);
            return;
        };
        let first = (self.scroll_row.max(0.0) as usize).min(total.saturating_sub(1));
        let last = (first + visible + 1).min(total);
        let mut widest = 0;
        for row in first..last {
            let (_, span) = self.row_span(document, tab_stop, row);
            widest = widest.max(span.end.get());
        }
        let viewport = f32::from(geometry.bounds.size.width) - 2.0 * f32::from(geometry.padding());
        let content = widest as f32 * f32::from(geometry.char_width);
        let max_x = (content - viewport).max(0.0);
        self.scroll_x = self.scroll_x.min(gpui::px(max_x));
    }

    pub(crate) fn scroll_to_caret(
        &mut self,
        document: &Document,
        tab_stop: usize,
        head: chaus_core::position::CharOffset,
    ) {
        let Some(geometry) = self.geometry else {
            return;
        };
        let (_, row) = self.row_of_offset(document, tab_stop, head);
        let visible = geometry.visible_rows();
        let first = self.scroll_row.floor().max(0.0) as usize;
        if row < first {
            self.scroll_row = row as f32;
        } else if row >= first + visible {
            self.scroll_row = (row + 1).saturating_sub(visible) as f32;
        }
        self.clamp_scroll(document, tab_stop);

        if self.rows_columns.is_none() {
            let column = self.caret_column(document, tab_stop, head);
            let caret_x = gpui::px(column as f32 * f32::from(geometry.char_width));
            let width = geometry.bounds.size.width - geometry.padding() * 2.0;
            if caret_x - self.scroll_x > width {
                self.scroll_x = caret_x - width;
            }
            if caret_x < self.scroll_x {
                self.scroll_x = caret_x;
            }
            self.scroll_x = self.scroll_x.max(gpui::px(0.));
        } else {
            self.scroll_x = gpui::px(0.);
        }
    }

    // ---- wrap configuration and reflow -------------------------------------

    /// Break policy implied by a wrap mode.
    pub(crate) fn wrap_policy(mode: WrapMode) -> WrapPolicy {
        match mode {
            WrapMode::Word => WrapPolicy::Word,
            _ => WrapPolicy::Character,
        }
    }

    pub(crate) fn is_wrap_locked(&self) -> bool {
        self.wrap_width_locked
    }

    pub(crate) fn lock_wrap_width(&mut self) {
        self.wrap_width_locked = true;
    }

    pub(crate) fn reflow_generation(&self) -> u64 {
        self.reflow_generation
    }

    pub(crate) fn reflow_publications(&self) -> u64 {
        self.reflow_publications
    }

    pub(crate) fn is_reflowing(&self) -> bool {
        self.reflow_in_flight.is_some()
    }

    /// Record a new wrap request. Returns `true` when a background measurement
    /// must be started (no job is in flight for the previous request).
    pub(crate) fn request_wrap(&mut self, columns: Option<usize>, policy: WrapPolicy) -> bool {
        if self.rows_columns == columns && self.rows_policy == policy {
            if self.reflow_columns != columns || self.reflow_policy != policy {
                self.set_request(columns, policy);
            }
            return false;
        }
        if self.reflow_columns == columns && self.reflow_policy == policy {
            return false;
        }
        self.set_request(columns, policy);
        self.reflow_in_flight.is_none()
    }

    /// Record a width change without starting a measurement, used when the
    /// settings snapshot changes but a reflow is already in flight.
    pub(crate) fn set_request(&mut self, columns: Option<usize>, policy: WrapPolicy) {
        self.reflow_columns = columns;
        self.reflow_policy = policy;
        self.reflow_generation = self.reflow_generation.wrapping_add(1);
    }

    /// Claim the in-flight slot for the current request.
    pub(crate) fn begin_reflow(&mut self) -> (u64, Option<usize>, WrapPolicy) {
        let generation = self.reflow_generation;
        self.reflow_in_flight = Some(generation);
        (generation, self.reflow_columns, self.reflow_policy)
    }

    /// Publish a finished measurement only when its generation and document
    /// revision still match. A superseded or stale job is discarded and the
    /// latest width is measured instead; no partial layout is ever published.
    pub(crate) fn finish_reflow(
        &mut self,
        generation: u64,
        revision: chaus_core::position::DocumentRevision,
        current_revision: chaus_core::position::DocumentRevision,
        counts: Vec<usize>,
    ) -> ReflowOutcome {
        if self.reflow_in_flight != Some(generation) {
            return ReflowOutcome::Stale;
        }
        self.reflow_in_flight = None;
        if generation == self.reflow_generation && revision == current_revision {
            self.rows_columns = self.reflow_columns;
            self.rows_policy = self.reflow_policy;
            self.rows.rebuild(counts);
            self.reflow_publications = self.reflow_publications.wrapping_add(1);
            ReflowOutcome::Published
        } else {
            ReflowOutcome::Retry
        }
    }

    pub(crate) fn measure_rows(
        snapshot: &DocumentSnapshot,
        columns: Option<usize>,
        tab_stop: usize,
        policy: WrapPolicy,
    ) -> Vec<usize> {
        let checkpoints = WrapCheckpoints::default();
        (0..snapshot.line_count())
            .map(|line| {
                let slice = snapshot.line_slice(line);
                let source = chaus_layout::LineSource::from_rope(slice);
                chaus_layout::row_count_source(&source, columns, tab_stop, policy, &checkpoints)
            })
            .collect()
    }
}
