//! Text engine for chaus: positions, Ropey document, transactions, history,
//! EOL metadata, settings parsing and Vim state.
//!
//! This crate has no GPUI dependency. The view layer owns pixels, focus and
//! platform input; it mutates text only through [`document::Document::apply`].

pub mod document;
pub mod eol;
pub mod position;
pub mod settings;
