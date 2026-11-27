//! chaus application library.
//!
//! The binary is a thin CLI wrapper; window lifecycle, settings persistence
//! and preferences live here so they can be integration-tested.

pub mod app;
pub mod diagnostics;

pub mod font_validation;
pub mod fonts;
mod platform;
pub mod preferences;
pub mod settings_file;
mod settings_global;
pub mod settings_store;
pub mod theme_choice;
pub mod ui_preferences;
