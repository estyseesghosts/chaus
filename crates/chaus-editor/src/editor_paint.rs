//! Row layout and paint preparation.

use chaus_core::position::{CharOffset, LineIndex};
use chaus_highlight::TokenKind;
use gpui::{fill, point, px, size, Bounds, PaintQuad, Pixels, SharedString, TextRun, Window};
use silvestris::convert::hsla;
use silvestris::core::tokens::Theme;
use unicode_width::UnicodeWidthChar as _;

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
    ) -> PaintFrame {
        let font_size = px(self.editor_font_size());
        let line_height = px(self.editor_font_size() * self.settings.line_height);
        let font_family = self
            .settings
            .font_family
            .clone()
            .unwrap_or_else(|| theme.type_.mono.to_string());
        let font = gpui::font(font_family);
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
        let caret_visible = self.caret_visible && self.focus.is_focused(window);
        let usable_width =
            (f32::from(bounds.size.width) - 2.0 * PADDING).max(f32::from(char_width));
        let mode = self.settings.wrap;
        let columns = (mode != chaus_core::settings::WrapMode::None).then(|| {
            (usable_width / f32::from(char_width).max(1.0))
                .floor()
                .max(1.0) as usize
        });
        let policy = Editor::wrap_policy(mode);
        self.set_wrap_layout(columns, policy);

        let visible = (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize + 2;
        let first_row = self.scroll_row.floor().max(0.0) as usize;
        let last_row = self.total_rows().saturating_sub(1);
        let first_line = self.line_at_row(first_row.min(last_row));
        let selection = self.selection.range();
        let selection_color = hsla(theme.colors.accent).opacity(0.35);
        let mut rows = Vec::new();
        let mut selection_quads = Vec::new();
        let mut row = self.prefix_rows.get(first_line.0).copied().unwrap_or(0);
        let base_x = bounds.left() + px(PADDING) - self.scroll_x;
        let base_y = bounds.top() + px(PADDING);

        for line_index in first_line.0..self.document.len_lines() {
            let line = LineIndex(line_index);
            let line_start = self.document.line_start(line);
            let line_text = self.document.line_text(line);
            for segment in
                chaus_layout::segments_with(&line_text, columns, self.settings.tab_stop, policy)
            {
                let y = base_y + px((row as f32 - self.scroll_row) * f32::from(line_height));
                row += 1;
                if y + line_height < bounds.top() {
                    continue;
                }
                if y > bounds.bottom() {
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
                let segment_start = CharOffset(line_start.0 + segment.char_start);
                let segment_end = CharOffset(line_start.0 + segment.char_end);
                if !selection.is_empty()
                    && selection.start < segment_end.max(CharOffset(segment_start.0 + 1))
                    && selection.end > segment_start
                {
                    let start = selection.start.max(segment_start);
                    let end = selection.end.min(segment_end);
                    let absolute_start = chaus_layout::column_at(
                        &line_text,
                        start.0.saturating_sub(line_start.0),
                        self.settings.tab_stop,
                    )
                    .max(segment.start_column);
                    let absolute_end = if selection.end >= segment_end {
                        segment.end_column()
                    } else {
                        chaus_layout::column_at(
                            &line_text,
                            end.0.saturating_sub(line_start.0),
                            self.settings.tab_stop,
                        )
                    }
                    .max(segment.start_column)
                    .min(segment.end_column());
                    // Segments begin at the left edge of their visual row, so
                    // selection columns must be segment-local. Using absolute
                    // logical-line columns shifts wrapped rows right.
                    if let Some((start_column, end_column)) = chaus_layout::local_selection_columns(
                        &segment,
                        absolute_start,
                        absolute_end,
                    ) {
                        let left = base_x + px(start_column as f32 * f32::from(char_width));
                        let right = base_x + px(end_column as f32 * f32::from(char_width));
                        selection_quads.push(fill(
                            Bounds::from_corners(point(left, y), point(right, y + line_height)),
                            selection_color,
                        ));
                    }
                }

                let runs =
                    self.highlighted_runs(&line_text, line_start.0, &segment, &text_run, theme);
                let shaped = window.text_system().shape_line(
                    SharedString::new(segment.display),
                    font_size,
                    &runs,
                    None,
                );
                rows.push(PaintRow {
                    line: shaped,
                    origin: point(base_x, y),
                });
            }
            if rows.len() > visible * 4 {
                break;
            }
        }
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
    ) -> Vec<TextRun> {
        let spans = self.highlight_spans();
        if spans.is_empty() || segment.display.is_empty() {
            return vec![TextRun {
                len: segment.display.len(),
                color: base.color,
                ..base.clone()
            }];
        }
        let segment_start = line_start + segment.char_start;
        let mut span_index = spans.partition_point(|span| span.range.end <= segment_start);
        let mut column = segment.start_column;
        let mut display_bytes = 0usize;
        let mut run_start = 0usize;
        let mut run_kind: Option<TokenKind> = None;
        let mut runs = Vec::new();
        for (index, character) in line
            .chars()
            .skip(segment.char_start)
            .take(segment.char_end.saturating_sub(segment.char_start))
            .enumerate()
        {
            let offset = segment_start + index;
            while span_index < spans.len() && spans[span_index].range.end <= offset {
                span_index += 1;
            }
            let kind = spans
                .get(span_index)
                .filter(|span| span.range.contains(&offset))
                .map(|span| span.kind);
            if run_kind.is_none() && index == 0 {
                run_kind = kind;
            } else if kind != run_kind {
                if display_bytes > run_start {
                    runs.push(run(display_bytes - run_start, run_kind, base, theme));
                }
                run_start = display_bytes;
                run_kind = kind;
            }
            let width = if character == '\t' {
                let stop = self.settings.tab_stop.max(1);
                stop - (column % stop)
            } else {
                character.width().unwrap_or(0)
            };
            display_bytes += if character == '\t' {
                width
            } else {
                character.len_utf8()
            };
            column += width;
        }
        if display_bytes > run_start {
            runs.push(run(display_bytes - run_start, run_kind, base, theme));
        }
        if runs.is_empty() {
            runs.push(TextRun {
                len: segment.display.len(),
                color: base.color,
                ..base.clone()
            });
        }
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
            + px((row as f32 - self.scroll_row) * f32::from(line_height));
        if row_top + line_height < bounds.top() || row_top > bounds.bottom() {
            return None;
        }
        let line_start = self.document.line_start(line);
        let line_text = self.document.line_text(line);
        let absolute_column = chaus_layout::column_at(
            &line_text,
            head.0.saturating_sub(line_start.0),
            self.settings.tab_stop,
        );
        let segments = self.segments_for(line);
        let local_row = self.local_row_of(head, line);
        let segment = segments.get(local_row).or_else(|| segments.last())?;
        let x = bounds.left() + px(PADDING) - self.scroll_x
            + px(
                absolute_column.saturating_sub(segment.start_column) as f32 * f32::from(char_width)
            );
        Some(fill(
            Bounds::new(point(x, row_top), size(px(1.5), line_height)),
            hsla(theme.colors.text),
        ))
    }
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
