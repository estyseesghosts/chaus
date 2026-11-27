//! Row layout and paint preparation.

use std::borrow::Cow;

use chaus_core::position::{CharOffset, LineIndex};
use chaus_highlight::{Span, TokenKind};
use gpui::{
    fill, point, px, size, Bounds, Context, PaintQuad, Pixels, SharedString, TextRun, Window,
};
use silvestris::convert::hsla;
use silvestris::core::tokens::Theme;

use crate::editor::Editor;
use crate::geometry::PADDING;
use crate::paint_row::PaintRow;

pub(crate) struct PaintFrame {
    pub(crate) line_height: Pixels,
    pub(crate) char_width: Pixels,
    pub(crate) rows: Vec<PaintRow>,
    pub(crate) selection: Vec<PaintQuad>,
    pub(crate) caret: Option<PaintQuad>,
}

struct PaintMetrics {
    line_height: Pixels,
    char_width: Pixels,
    caret_visible: bool,
}

impl Editor {
    pub(crate) fn build_visible_rows(
        &mut self,
        window: &mut Window,
        bounds: Bounds<Pixels>,
        theme: &Theme,
        cx: &mut Context<Editor>,
    ) -> PaintFrame {
        let font_size = px(self.editor_font_size());
        let line_height = px(self.editor_font_size() * self.settings.line_height);
        let font_family = self.editor_font_family(theme);
        let font = gpui::font(font_family.clone());
        let sample = "MMMMMMMM";
        let text_run = TextRun {
            len: sample.len(),
            font,
            color: hsla(theme.colors.text),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let char_width = measure_advance(window, font_size, &text_run, sample);
        let caret_visible = self.blink.visible() && self.focus.is_focused(window);
        let usable_width =
            (f32::from(bounds.size.width) - 2.0 * PADDING).max(f32::from(char_width));
        let mode = self.settings.wrap;
        let columns = (mode != chaus_core::settings::WrapMode::None).then(|| {
            (usable_width / f32::from(char_width).max(1.0))
                .floor()
                .max(1.0) as usize
        });
        let policy = Editor::wrap_policy(mode);
        self.set_wrap_layout(columns, policy, cx);
        let paint_key = crate::paint_cache::PaintKey {
            revision: self.document.revision(),
            highlight: self.highlight_applied_generation(),
            columns,
            policy,
            tab_stop: self.settings.tab_stop,
            font_size_bits: f32::from(font_size).to_bits(),
            font_family,
            theme: theme.id,
            offset: 0,
        };

        let visible = (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize + 2;
        let last_row = self.total_rows().saturating_sub(1);
        let first_row = (self.viewport.scroll_row().floor().max(0.0) as usize).min(last_row);
        let first_line = self.line_at_row(first_row);
        let selection = self.selection.range();
        let selection_color = hsla(theme.colors.accent).opacity(0.35);
        let mut rows = Vec::new();
        let mut selection_quads = Vec::new();
        let mut row = first_row;
        let base_x = bounds.left() + px(PADDING) - self.viewport.scroll_x();
        let base_y = bounds.top() + px(PADDING);

        let mut paint_walked = 0usize;
        let mut paint_skipped = 0usize;
        let mut paint_chars = 0usize;
        let mut paint_unwrapped = 0usize;
        let mut paint_display_bytes = 0usize;
        for line_index in first_line.0..self.document.len_lines() {
            let line = LineIndex(line_index);
            let line_start = self.document.line_start(line);
            let source = Self::line_source(&self.document, line);
            let checkpoints = self.checkpoints_for(line);
            let column_checkpoints = self.column_checkpoints_for(line);
            // An unwrapped hard line has one row. Read only the visible source
            // window plus fixed overscan instead of shaping the whole line.
            let unwrapped = columns.is_none().then(|| {
                let first_column = (f32::from(self.viewport.scroll_x())
                    / f32::from(char_width).max(1.0))
                .floor()
                .max(0.0) as usize;
                chaus_layout::unwrapped_window(
                    &source,
                    chaus_layout::VisualColumn::new(first_column),
                    chaus_layout::VisualColumn::new(visible),
                    chaus_layout::VisualColumn::new(chaus_layout::OVERSCAN_COLUMNS),
                    self.settings.tab_stop,
                    &column_checkpoints,
                )
            });
            // Start at the first visible row of the first line and stop once
            // the viewport is covered. Checkpoints keep a deep row from walking
            // the thousands of rows above it.
            let skip = if line_index == first_line.0 {
                first_row.saturating_sub(self.viewport.row_prefix(line_index))
            } else {
                0
            };
            let (segments, stats) = if let Some(window) = &unwrapped {
                let mut stats = chaus_layout::WindowStats::default();
                stats.unwrapped_chars = window.source_chars;
                (vec![window.as_segment()], stats)
            } else {
                chaus_layout::segments_window_source(
                    &source,
                    columns,
                    self.settings.tab_stop,
                    policy,
                    chaus_layout::WrapRowIndex::new(skip),
                    visible.saturating_add(4),
                    &checkpoints,
                )
            };
            paint_chars += stats.window_chars + stats.unwrapped_chars;
            paint_walked += stats.rows_walked;
            paint_skipped += stats.rows_skipped_via_checkpoint;
            paint_unwrapped += stats.unwrapped_chars;
            for (offset, segment) in segments.into_iter().enumerate() {
                let segment_index = skip + offset;
                let y =
                    base_y + px((row as f32 - self.viewport.scroll_row()) * f32::from(line_height));
                row += 1;
                if y + line_height < bounds.top() {
                    continue;
                }
                if y > bounds.bottom() {
                    self.viewport.record_layout_work(
                        paint_chars,
                        paint_unwrapped,
                        paint_walked,
                        paint_skipped,
                        paint_display_bytes,
                    );
                    return self.finish_frame(
                        bounds,
                        PaintMetrics {
                            line_height,
                            char_width,
                            caret_visible,
                        },
                        rows,
                        selection_quads,
                        theme,
                    );
                }
                let window_start = unwrapped
                    .as_ref()
                    .map_or(segment.char_start.get(), |window| window.char_start.get());
                let window_end = unwrapped
                    .as_ref()
                    .map_or(segment.char_end.get(), |window| window.char_end.get());
                let segment_start = CharOffset(line_start.0 + window_start);
                let segment_end = CharOffset(line_start.0 + window_end);
                if !selection.is_empty()
                    && selection.start < segment_end.max(CharOffset(segment_start.0 + 1))
                    && selection.end > segment_start
                {
                    let start = selection.start.max(segment_start);
                    let end = selection.end.min(segment_end);
                    if let Some(window) = &unwrapped {
                        let absolute_start = chaus_layout::column_at_source(
                            &source,
                            chaus_layout::LineCharOffset::new(start.0.saturating_sub(line_start.0)),
                            self.settings.tab_stop,
                            &column_checkpoints,
                        );
                        let absolute_end = chaus_layout::column_at_source(
                            &source,
                            chaus_layout::LineCharOffset::new(end.0.saturating_sub(line_start.0)),
                            self.settings.tab_stop,
                            &column_checkpoints,
                        );
                        let start_column = absolute_start
                            .get()
                            .saturating_sub(window.start_column.get());
                        let end_column = absolute_end
                            .get()
                            .saturating_sub(window.start_column.get())
                            .min(window.display.chars().count());
                        if end_column > start_column {
                            let left = base_x + px(start_column as f32 * f32::from(char_width));
                            let right = base_x + px(end_column as f32 * f32::from(char_width));
                            selection_quads.push(fill(
                                Bounds::from_corners(point(left, y), point(right, y + line_height)),
                                selection_color,
                            ));
                        }
                    } else {
                        let absolute_start = chaus_layout::column_at_source(
                            &source,
                            chaus_layout::LineCharOffset::new(start.0.saturating_sub(line_start.0)),
                            self.settings.tab_stop,
                            &column_checkpoints,
                        )
                        .max(segment.start_column);
                        let absolute_end = if selection.end >= segment_end {
                            segment.end_column()
                        } else {
                            chaus_layout::column_at_source(
                                &source,
                                chaus_layout::LineCharOffset::new(
                                    end.0.saturating_sub(line_start.0),
                                ),
                                self.settings.tab_stop,
                                &column_checkpoints,
                            )
                        }
                        .max(segment.start_column)
                        .min(segment.end_column());
                        // Wrapped segments begin at the left edge of their row,
                        // so selection columns must be segment-local.
                        if let Some((start_column, end_column)) =
                            chaus_layout::local_selection_columns(
                                &segment,
                                absolute_start,
                                absolute_end,
                            )
                        {
                            let left =
                                base_x + px(start_column.get() as f32 * f32::from(char_width));
                            let right =
                                base_x + px(end_column.get() as f32 * f32::from(char_width));
                            selection_quads.push(fill(
                                Bounds::from_corners(point(left, y), point(right, y + line_height)),
                                selection_color,
                            ));
                        }
                    }
                }

                let window_key = crate::paint_cache::PaintKey {
                    offset: window_start,
                    ..paint_key.clone()
                };
                let shaped = match self
                    .viewport
                    .shaped_row(&window_key, line_index, segment_index)
                {
                    Some(shaped) => shaped,
                    None => {
                        let line_window: Cow<'_, str> = match &unwrapped {
                            Some(window) => Cow::Borrowed(window.text.as_ref()),
                            None => source.window(
                                chaus_layout::LineCharOffset::new(window_start),
                                chaus_layout::LineCharOffset::new(window_end),
                            ),
                        };
                        let run_start_column = unwrapped
                            .as_ref()
                            .map_or(segment.start_column, |window| window.start_column);
                        let mut local = segment.clone();
                        local.char_start = chaus_layout::LineCharOffset::new(0);
                        local.char_end =
                            chaus_layout::LineCharOffset::new(line_window.chars().count());
                        let runs = self.highlighted_runs(
                            &line_window,
                            line_start.0 + window_start,
                            &local,
                            &text_run,
                            theme,
                            run_start_column,
                        );
                        paint_display_bytes += segment.display.len();
                        let shaped = window.text_system().shape_line(
                            SharedString::new(segment.display),
                            font_size,
                            &runs,
                            None,
                        );
                        self.viewport.cache_shaped_row(
                            &window_key,
                            line_index,
                            segment_index,
                            shaped.clone(),
                        );
                        self.viewport.bump_shaped_row_calls();
                        shaped
                    }
                };
                rows.push(PaintRow {
                    line: shaped,
                    origin: point(base_x, y),
                });
            }
            if rows.len() > visible * 4 {
                break;
            }
        }
        self.viewport.record_layout_work(
            paint_chars,
            paint_unwrapped,
            paint_walked,
            paint_skipped,
            paint_display_bytes,
        );
        self.finish_frame(
            bounds,
            PaintMetrics {
                line_height,
                char_width,
                caret_visible,
            },
            rows,
            selection_quads,
            theme,
        )
    }

    /// Byte length and syntax kind of each painted run on one visual row.
    ///
    /// This is the diagnostic form of the runs passed to GPUI. Lengths index
    /// `Segment::display`, whose tab expansion and grapheme segmentation come
    /// from `chaus-layout`; their sum always equals its UTF-8 length.
    #[must_use]
    pub fn syntax_run_lengths(
        &self,
        line: LineIndex,
        segment_index: usize,
    ) -> Option<Vec<(usize, Option<TokenKind>)>> {
        let source = Self::line_source(&self.document, line);
        let segments = self.segments_for(line);
        let segment = segments.get(segment_index)?;
        let start = segment.char_start.get();
        let end = segment.char_end.get();
        let window = source.window(
            chaus_layout::LineCharOffset::new(start),
            chaus_layout::LineCharOffset::new(end),
        );
        let mut local = segment.clone();
        local.char_start = chaus_layout::LineCharOffset::new(0);
        local.char_end = chaus_layout::LineCharOffset::new(window.chars().count());
        let spans = self.highlight_spans();
        Some(syntax_run_lengths(
            spans,
            &window,
            self.document.line_start(line).0 + start,
            &local,
            self.settings.tab_stop,
            segment.start_column,
        ))
    }

    /// Split one visual row into GPUI runs by the current highlight spans.
    ///
    /// Runs are counted in bytes of `segment.display`, where a tab became
    /// `width` spaces. Spans are in document char offsets and are only read
    /// when they belong to the current document revision.
    fn highlighted_runs(
        &self,
        line: &str,
        line_start: usize,
        segment: &chaus_layout::Segment,
        base: &TextRun,
        theme: &Theme,
        start_column: chaus_layout::VisualColumn,
    ) -> Vec<TextRun> {
        let spans = self.highlight_spans();
        if spans.is_empty() || segment.display.is_empty() {
            return vec![TextRun {
                len: segment.display.len(),
                color: base.color,
                ..base.clone()
            }];
        }
        let pieces = syntax_run_lengths(
            spans,
            line,
            line_start,
            segment,
            self.settings.tab_stop,
            start_column,
        );
        let runs: Vec<TextRun> = pieces
            .into_iter()
            .map(|(len, kind)| run(len, kind, base, theme))
            .collect();
        debug_assert_eq!(
            runs.iter().map(|text_run| text_run.len).sum::<usize>(),
            segment.display.len(),
            "GPUI run lengths must cover the displayed row exactly"
        );
        runs
    }

    fn finish_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        metrics: PaintMetrics,
        rows: Vec<PaintRow>,
        selection: Vec<PaintQuad>,
        theme: &Theme,
    ) -> PaintFrame {
        let caret = if metrics.caret_visible && self.selection.is_empty() {
            self.caret_rect(bounds, metrics.line_height, metrics.char_width, theme)
        } else {
            None
        };
        PaintFrame {
            line_height: metrics.line_height,
            char_width: metrics.char_width,
            rows,
            selection,
            caret,
        }
    }

    fn caret_rect(
        &mut self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        char_width: Pixels,
        theme: &Theme,
    ) -> Option<PaintQuad> {
        let head = self.selection.head;
        let (line, row) = self.row_of_offset(head);
        let row_top = bounds.top()
            + px(PADDING)
            + px((row as f32 - self.viewport.scroll_row()) * f32::from(line_height));
        if row_top + line_height < bounds.top() || row_top > bounds.bottom() {
            return None;
        }
        let line_start = self.document.line_start(line);
        let source = Self::line_source(&self.document, line);
        let checkpoints = self.checkpoints_for(line);
        let column_checkpoints = self.column_checkpoints_for(line);
        let absolute_column = chaus_layout::column_at_source(
            &source,
            chaus_layout::LineCharOffset::new(head.0.saturating_sub(line_start.0)),
            self.settings.tab_stop,
            &column_checkpoints,
        );
        let local_row = self.local_row_of(head, line);
        let row_start_column = if self.viewport.committed_columns().is_none() {
            // No segment is materialized for a one-row unwrapped line.
            (f32::from(self.viewport.scroll_x()) / f32::from(char_width).max(1.0))
                .floor()
                .max(0.0) as usize
        } else {
            chaus_layout::segment_at_source(
                &source,
                self.viewport.committed_columns(),
                self.settings.tab_stop,
                self.viewport.policy(),
                chaus_layout::WrapRowIndex::new(local_row),
                &checkpoints,
            )?
            .start_column
            .get()
        };
        let x = bounds.left()
            + px(PADDING)
            + px(
                absolute_column.get().saturating_sub(row_start_column) as f32
                    * f32::from(char_width),
            );
        Some(fill(
            Bounds::new(point(x, row_top), size(px(1.5), line_height)),
            hsla(theme.colors.text),
        ))
    }
}

/// Resolve syntax spans into contiguous display-byte runs for one segment.
///
/// The display mapping comes from `chaus-layout`, so a run boundary is always
/// a grapheme boundary in the expanded string. A span that covers only part of
/// a grapheme colours the whole grapheme, chosen by the kind at its first
/// source scalar; joined emoji are never split.
fn syntax_run_lengths(
    spans: &[Span],
    line: &str,
    line_start: usize,
    segment: &chaus_layout::Segment,
    tab_stop: usize,
    start_column: chaus_layout::VisualColumn,
) -> Vec<(usize, Option<TokenKind>)> {
    let boundaries =
        chaus_layout::display_boundaries_from_column(line, segment, tab_stop, start_column);
    let segment_start = line_start + segment.char_start.get();
    let mut span_index = spans.partition_point(|span| span.range.end <= segment_start);
    let mut run_start = 0usize;
    let mut run_kind: Option<TokenKind> = None;
    let mut runs = Vec::new();
    let mut first = true;
    for pair in boundaries.windows(2) {
        let offset = line_start + pair[0].char_offset.get();
        let display_start = pair[0].display_byte.get();
        while span_index < spans.len() && spans[span_index].range.end <= offset {
            span_index += 1;
        }
        let kind = spans
            .get(span_index)
            .filter(|span| span.range.contains(&offset))
            .map(|span| span.kind);
        if first {
            run_kind = kind;
            first = false;
        } else if kind != run_kind {
            if display_start > run_start {
                runs.push((display_start - run_start, run_kind));
            }
            run_start = display_start;
            run_kind = kind;
        }
        debug_assert!(pair[1].display_byte <= segment.display.len());
    }
    if segment.display.len() > run_start {
        runs.push((segment.display.len() - run_start, run_kind));
    }
    if runs.is_empty() {
        runs.push((segment.display.len(), None));
    }
    runs
}

fn run(len: usize, kind: Option<TokenKind>, base: &TextRun, theme: &Theme) -> TextRun {
    TextRun {
        len,
        color: kind.map_or(base.color, |kind| token_color(kind, theme)),
        ..base.clone()
    }
}

/// Theme-derived syntax colours. Chaus keeps the small semantic token set and
/// borrows the active Silvestris palette, so Aqua and Geist both stay legible.
fn token_color(kind: TokenKind, theme: &Theme) -> gpui::Hsla {
    let colors = &theme.colors;
    hsla(match kind {
        TokenKind::Keyword => colors.accent,
        TokenKind::Type => colors.info,
        TokenKind::Function | TokenKind::Tag => colors.link,
        TokenKind::String => colors.success,
        TokenKind::Number => colors.warning,
        TokenKind::Comment => colors.text_secondary,
        TokenKind::Constant => colors.error,
        TokenKind::Attribute => colors.warning,
        TokenKind::Variable
        | TokenKind::Property
        | TokenKind::Operator
        | TokenKind::Punctuation => colors.text,
        _ => colors.text,
    })
}

/// Average advance of a monospace sample, in logical pixels.
///
/// The run length must cover the sample's UTF-8 bytes; GPUI drops zero-length
/// runs, which shapes an empty line and yields a zero advance.
fn measure_advance(window: &mut Window, font_size: Pixels, run: &TextRun, sample: &str) -> Pixels {
    let shaped =
        window
            .text_system()
            .shape_line(sample.into(), font_size, std::slice::from_ref(run), None);
    let count = sample.chars().count().max(1) as f32;
    px(f32::from(shaped.width()) / count).max(px(1.0))
}
