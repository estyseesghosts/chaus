//! chaus application library.
//!
//! The binary is a thin CLI wrapper; window lifecycle, settings persistence
//! and preferences live here so they can be integration-tested.

pub mod app;
pub mod diagnostics;

pub mod font_validation;
pub mod fonts;
mod platform;
mod preferences;
mod settings_global;
pub mod settings_writer;
pub mod theme_choice;
pub mod ui_preferences;
