//! Text engine for chaus: positions, Ropey document, transactions, history,
//! EOL metadata, settings parsing and Vim state.
//!
//! This crate has no GPUI dependency. The view layer owns pixels, focus and
//! platform input; it mutates text only through [`document::Document::apply`].

pub mod diff;
pub mod document;
pub mod edit;
pub mod eol;
pub mod position;
pub mod settings;
pub mod snapshot;

pub(crate) mod history;
pub(crate) mod utf16_index;

pub use document::Document;
pub use edit::{
    AppliedEdit, EditCause, EditError, EditResult, EditTransaction, LineRange, TextEdit, TextPoint,
};
pub use snapshot::DocumentSnapshot;
