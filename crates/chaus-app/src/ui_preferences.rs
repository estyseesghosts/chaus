//! Chaus's UI-preferences document.
//!
//! Window frames and file-panel presentation are consumer state, not rc
//! settings. They live in a small versioned JSON document beside the rc file so
//! a malformed value or an unwritable profile can fail soft instead of
//! preventing Chaus from starting. `.chausrc` keeps the editor settings it
//! already owns.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::App;
use serde::{Deserialize, Serialize};
use silvestris::core::prefs::{Loaded, Versioned};
use silvestris::file_panel::{PanelState, PanelStateStore, PreferencePanelStates, StdFsProvider};
use silvestris::prefs::Preferences;
use silvestris::window_role::{PreferenceFrames, SavedFrames};

use chaus_editor::FileDialogs;

use crate::app::settings_path;

const SCHEMA_VERSION: u32 = 1;

/// The state Chaus persists between sessions outside the rc file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    #[serde(default = "schema")]
    pub schema_version: u32,
    /// File → Open: view, expanded sidebar width.
    #[serde(default)]
    pub open_panel: PanelState,
    /// Save As: expanded state and Hide extension.
    #[serde(default)]
    pub save_panel: PanelState,
    /// Window frames by Silvestris autosave key.
    #[serde(default)]
    pub frames: SavedFrames,
    /// Fields written by a newer version or by a consumer; kept verbatim.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn schema() -> u32 {
    SCHEMA_VERSION
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            open_panel: PanelState::default(),
            save_panel: PanelState::default(),
            frames: SavedFrames::default(),
            extra: Default::default(),
        }
    }
}

impl Versioned for UiPreferences {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }

    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

/// `~/.chaus-ui.json` on Unix and `%USERPROFILE%\_chaus-ui.json` on Windows,
/// beside the rc file it complements.
#[must_use]
pub fn ui_preferences_path() -> Option<PathBuf> {
    settings_path().map(|path| {
        path.with_file_name(if cfg!(windows) {
            "_chaus-ui.json"
        } else {
            ".chaus-ui.json"
        })
    })
}

/// Install the application-lifetime file-dialog service. A missing or
/// malformed UI document yields defaults; only the preferences entity refuses
/// later writes, and the panels keep working from their in-memory state.
pub(crate) fn install_file_dialogs(cx: &mut App) {
    let Some(path) = ui_preferences_path() else {
        eprintln!("chaus: no home directory; file-panel state will not persist");
        FileDialogs::install(cx, FileDialogs::ephemeral());
        return;
    };
    let (preferences, loaded) = Preferences::<UiPreferences>::open(path.clone(), &[], cx);
    if let Loaded::Malformed { error, .. } = &loaded {
        eprintln!(
            "chaus: {}: {error}; using defaults and leaving the file untouched",
            path.display()
        );
    }
    let open_store = Rc::new(PreferencePanelStates::new(
        preferences.clone(),
        |value| &value.open_panel,
        |value| &mut value.open_panel,
    ));
    let save_store = Rc::new(PreferencePanelStates::new(
        preferences.clone(),
        |value| &value.save_panel,
        |value| &mut value.save_panel,
    ));
    let frames = Rc::new(PreferenceFrames::new(
        preferences,
        |value| &value.frames,
        |value| &mut value.frames,
    ));
    FileDialogs::install(
        cx,
        FileDialogs::new(
            Rc::new(StdFsProvider::new()),
            open_store as Rc<dyn PanelStateStore>,
            save_store as Rc<dyn PanelStateStore>,
            frames,
        ),
    );
}
