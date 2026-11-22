//! Application-global settings. Settings may be app-global, but each window
//! receives its own copy and never shares view state.

use chaus_core::settings::Settings;
use gpui::Global;

pub(crate) struct SettingsGlobal(pub(crate) Settings);

impl Global for SettingsGlobal {}
