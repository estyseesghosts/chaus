//! Editor view for chaus.
//!
//! The document model lives in `chaus-core`; this crate turns it into a GPUI
//! entity: cell geometry, painting, pointer handling, the platform text-input
//! bridge and the per-window file session.

mod editor;
mod editor_blink;
mod editor_close;
mod editor_conflict;
mod editor_cursor;
mod editor_edit;
mod editor_element;
mod editor_file;
mod editor_find;
mod editor_input;
mod editor_keys;
mod editor_paint;
mod editor_pointer;
mod editor_vim;
mod geometry;
mod paint_row;

pub use editor::Editor;
pub use editor_file::FileState;
pub use editor_keys::{bind_keys, KEY_CONTEXT};
pub use geometry::Geometry;
