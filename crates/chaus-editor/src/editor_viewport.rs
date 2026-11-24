//! Interpreting the Vim engine's semantic viewport commands against the
//! editor's visual rows.
//!
//! `chaus-vim` never sees pixels, rows or window geometry. It only asks for
//! centering, top/bottom placement or a half-page scroll; this module turns
//! those into `scroll_row` and caret changes.

use chaus_core::position::Selection;
use chaus_vim::{Mode, ViewportCommand, ViewportIntent};
use gpui::Context;

use crate::editor::Editor;

impl Editor {
    pub(crate) fn apply_viewport_intent(&mut self, intent: ViewportIntent, cx: &mut Context<Self>) {
        match intent.command {
            ViewportCommand::Center => {
                let (_, row) = self.row_of_offset(self.selection.head);
                let visible = self.viewport_rows();
                self.scroll_row = row as f32 - (visible.saturating_sub(1) as f32 / 2.0);
                self.clamp_scroll();
            }
            ViewportCommand::Top => {
                let (_, row) = self.row_of_offset(self.selection.head);
                self.scroll_row = row as f32;
                self.clamp_scroll();
            }
            ViewportCommand::Bottom => {
                let (_, row) = self.row_of_offset(self.selection.head);
                let visible = self.viewport_rows();
                self.scroll_row = (row + 1).saturating_sub(visible) as f32;
                self.clamp_scroll();
            }
            ViewportCommand::HalfPageDown | ViewportCommand::HalfPageUp => {
                self.half_page(intent.command, intent.count);
            }
            // `ViewportCommand` is `#[non_exhaustive]`; future commands do
            // nothing until routed here.
            _ => {}
        }
        cx.notify();
    }

    fn half_page(&mut self, command: ViewportCommand, count: usize) {
        let visible = self.viewport_rows();
        let step = (visible / 2).max(1).saturating_mul(count.max(1));
        let down = command == ViewportCommand::HalfPageDown;
        let (_, row) = self.row_of_offset(self.selection.head);
        let total = self.total_rows();
        let target_row = if down {
            row.saturating_add(step).min(total.saturating_sub(1))
        } else {
            row.saturating_sub(step)
        };
        self.move_caret_to_row(row, target_row);
        self.scroll_row = if down {
            (self.scroll_row + step as f32).min(total.saturating_sub(visible) as f32)
        } else {
            (self.scroll_row - step as f32).max(0.0)
        };
        self.clamp_scroll();
        self.scroll_to_caret();
    }

    pub(crate) fn move_caret_to_row(&mut self, current_row: usize, target_row: usize) {
        if target_row == current_row {
            return;
        }
        let (_, current_span) = self.row_span(current_row);
        let x = self.preferred_column.unwrap_or_else(|| {
            self.caret_column(self.selection.head)
                .saturating_sub(current_span.start)
        });
        self.preferred_column = Some(x);
        let (line, span) = self.row_span(target_row);
        let offset = self.offset_at(line, span.start.saturating_add(x).min(span.end));
        let extend = matches!(self.vim_engine.mode(), Mode::VisualChar | Mode::VisualLine);
        self.selection = if extend {
            Selection::new(self.selection.anchor, offset)
        } else {
            Selection::caret(offset)
        };
    }

    fn viewport_rows(&self) -> usize {
        self.geometry
            .map(|geometry| geometry.visible_rows())
            .unwrap_or(1)
            .max(1)
    }

    /// First visual row currently painted at the top of the viewport.
    #[must_use]
    pub fn viewport_scroll_row(&self) -> f32 {
        self.scroll_row
    }

    /// Visual row of the Normal/Visual caret, for diagnostics and tests.
    #[must_use]
    pub fn caret_visual_row(&mut self) -> usize {
        self.row_of_offset(self.selection.head).1
    }

    /// Number of text rows that fit in the current geometry, if painted.
    #[must_use]
    pub fn visible_row_count(&self) -> Option<usize> {
        self.geometry.map(|geometry| geometry.visible_rows())
    }
}
