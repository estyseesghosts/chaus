//! chaus application library.
//!
//! The binary is a thin CLI wrapper; window lifecycle, settings persistence
//! and preferences live here so they can be integration-tested.

pub mod app;

pub mod fonts;
mod platform;
mod preferences;
mod settings_global;
mod theme_choice;
