//! Editor view for chaus.
//!
//! The document model lives in `chaus-core`; this crate turns it into a GPUI
//! entity: cell geometry, painting, pointer handling, the platform text-input
//! bridge and the per-window file session.

mod editor;
mod editor_blink;
mod editor_close;
mod editor_command_line;
mod editor_commands;
mod editor_completion;
mod editor_conflict;
mod editor_cursor;
mod editor_edit;
mod editor_element;
mod editor_ex;
mod editor_file;
mod editor_find;
mod editor_format;
mod editor_highlight;
mod editor_indent;
mod editor_input;
mod editor_keys;
mod editor_paint;
mod editor_pointer;
mod editor_recovery;
mod editor_viewport;
mod editor_vim;
mod editor_watch;
mod editor_wqa;
mod editor_zoom;
mod file_dialogs;
mod file_operation;
mod geometry;
mod paint_cache;
mod paint_row;
mod row_index;

pub use editor::{set_reflow_gate_for_test, Editor};
pub use editor_file::{set_save_delay_for_test, set_save_gate_for_test, FileState, LoadHook};
pub use editor_highlight::HighlightStatistics;
pub use editor_keys::{bind_keys, KEY_CONTEXT};
pub use file_dialogs::{FileDialogs, MemoryFrameStore};
pub use geometry::Geometry;
