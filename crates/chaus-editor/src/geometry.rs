//! Pixel geometry of the editor's text area.

use gpui::{point, px, Bounds, Pixels, Point};

/// Inset between the element bounds and the text area, in logical pixels.
pub(crate) const PADDING: f32 = 8.0;

/// Where the text area is on screen, in logical pixels.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub char_width: Pixels,
}

impl Geometry {
    pub(crate) fn text_origin(self) -> Point<Pixels> {
        point(
            self.bounds.left() + px(PADDING),
            self.bounds.top() + px(PADDING),
        )
    }

    pub(crate) fn visible_rows(self) -> usize {
        let height = (f32::from(self.bounds.size.height) - 2.0 * PADDING).max(0.0);
        (height / f32::from(self.line_height).max(1.0))
            .floor()
            .max(1.0) as usize
    }

    pub(crate) fn padding(self) -> Pixels {
        px(PADDING)
    }
}
