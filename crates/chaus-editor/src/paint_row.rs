//! One shaped visual row queued for painting.

use gpui::{Pixels, Point, ShapedLine};

pub(crate) struct PaintRow {
    pub(crate) line: ShapedLine,
    pub(crate) origin: Point<Pixels>,
}
