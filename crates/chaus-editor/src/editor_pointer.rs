//! Mouse selection, hit testing and wheel scrolling.

use chaus_core::position::{CharOffset, Selection};
use gpui::{
    px, Context, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta,
    ScrollEvent, Window,
};

use crate::editor::Editor;

impl Editor {
    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let Some(offset) = self.offset_for_point(event.position) else {
            return;
        };
        match event.click_count {
            2 => {
                let range = self.word_bounds(offset);
                self.selection = Selection::new(range.start, range.end);
                self.selecting = false;
            }
            count if count >= 3 => {
                let line = self.document.line_of(offset);
                self.selection = Selection::new(
                    self.document.line_start(line),
                    self.document.line_content_end(line),
                );
                self.selecting = false;
            }
            _ => {
                self.selecting = true;
                if event.modifiers.shift {
                    self.selection = Selection::new(self.selection.anchor, offset);
                } else {
                    self.selection = Selection::caret(offset);
                }
            }
        }
        self.preferred_column = None;
        cx.notify();
    }

    pub(crate) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting || !event.dragging() {
            return;
        }
        let Some(geometry) = self.geometry else {
            return;
        };
        if event.position.y < geometry.bounds.top() + geometry.padding() {
            self.scroll_row = (self.scroll_row - 1.0).max(0.0);
        } else if event.position.y > geometry.bounds.bottom() - geometry.padding() {
            self.scroll_row += 1.0;
            self.clamp_scroll();
        }
        // Keep extending the selection when the pointer leaves the viewport;
        // clamp into the element so hit testing still yields the edge row.
        let bounds = geometry.bounds;
        let last_x = (bounds.right() - px(1.0)).max(bounds.left());
        let last_y = (bounds.bottom() - px(1.0)).max(bounds.top());
        let position = Point {
            x: event.position.x.clamp(bounds.left(), last_x),
            y: event.position.y.clamp(bounds.top(), last_y),
        };
        if let Some(offset) = self.offset_for_point(position) {
            self.selection = Selection::new(self.selection.anchor, offset);
            cx.notify();
        }
    }

    pub(crate) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        cx.notify();
    }

    pub(crate) fn on_scroll(
        &mut self,
        event: &ScrollEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_height = self
            .geometry
            .map(|geometry| f32::from(geometry.line_height))
            .unwrap_or(18.0);
        match event.delta {
            ScrollDelta::Lines(delta) => {
                self.scroll_row -= delta.y;
                self.scroll_x -= px(delta.x * 12.0);
            }
            ScrollDelta::Pixels(delta) => {
                self.scroll_row -= f32::from(delta.y) / line_height.max(1.0);
                self.scroll_x -= delta.x;
            }
        }
        if self.rows_columns.is_some() {
            self.scroll_x = px(0.0);
        }
        self.clamp_scroll();
        cx.notify();
    }

    pub(crate) fn offset_for_point(&mut self, position: Point<Pixels>) -> Option<CharOffset> {
        let geometry = self.geometry?;
        if !geometry.bounds.contains(&position) {
            return None;
        }
        let line_height = geometry.line_height;
        let row = (position.y - geometry.bounds.top() - geometry.padding()) / line_height
            + self.scroll_row;
        let target_row = (row.max(0.0) as usize).min(self.total_rows().saturating_sub(1));
        let (line, span) = self.row_span(target_row);
        let x = position.x - geometry.bounds.left() - geometry.padding() + self.scroll_x;
        let local_columns =
            (f32::from(x) / f32::from(geometry.char_width).max(1.0)).max(0.0) as usize;
        Some(self.offset_at(line, span.start + local_columns))
    }
}
