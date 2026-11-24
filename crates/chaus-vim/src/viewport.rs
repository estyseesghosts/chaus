//! Semantic viewport requests produced by Normal-mode keys.
//!
//! `chaus-vim` knows nothing about pixels, rows or an `Editor`; the editor
//! interprets these commands against its own geometry.

/// One viewport placement or scroll command.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportCommand {
    /// Put the caret's visual row in the middle of the viewport.
    Center,
    /// Put the caret's visual row at the top of the viewport.
    Top,
    /// Put the caret's visual row at the bottom of the viewport.
    Bottom,
    /// Move the caret and viewport down by half a page.
    HalfPageDown,
    /// Move the caret and viewport up by half a page.
    HalfPageUp,
}

/// A viewport command with the count the person typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportIntent {
    pub command: ViewportCommand,
    /// Number of half pages for `Ctrl-D`/`Ctrl-U`; one for `zz`/`zt`/`zb`.
    pub count: usize,
}

impl ViewportIntent {
    #[must_use]
    pub(crate) fn new(command: ViewportCommand, count: usize) -> Self {
        Self {
            command,
            count: count.max(1),
        }
    }
}
