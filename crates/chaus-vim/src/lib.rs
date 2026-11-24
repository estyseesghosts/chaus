//! A small, deterministic Vim-style modal layer.
//!
//! This is not a Vim emulator: it implements one Normal/Insert/Visual/Command
//! state machine over [`chaus_core::document::Document`]. Every text change goes
//! through [`chaus_core::document::Document::apply`], so modal edits share undo,
//! dirty tracking and EOL handling with ordinary typing.

mod command_line;
mod engine;
mod engine_command;
mod engine_normal;
mod engine_visual;
pub mod ex_command;
mod key;
mod mapping;
mod mode;
mod motion;
mod operator;
mod register;

pub use command_line::{CommandPrefix, MAX_COMMAND_CHARS, MAX_HISTORY};
pub use engine::{Engine, Outcome};
pub use engine_command::{CommandLineView, CompletionRequest};
pub use ex_command::ExCommand;
pub use key::Key;
pub use mapping::{parse_mappings, Mapping};
pub use mode::Mode;
pub use motion::first_non_space;
