//! A small, deterministic Vim-style modal layer.
//!
//! This is not a Vim emulator: it implements one Normal/Insert/Visual/Command
//! state machine over [`chaus_core::document::Document`]. Every text change goes
//! through [`chaus_core::document::Document::apply`], so modal edits share undo,
//! dirty tracking and EOL handling with ordinary typing.

mod engine;
mod engine_normal;
mod engine_visual;
mod key;
mod mapping;
mod mode;
mod motion;
mod operator;
mod register;

pub use engine::{Engine, Outcome};
pub use key::Key;
pub use mapping::{parse_mappings, Mapping};
pub use mode::Mode;
