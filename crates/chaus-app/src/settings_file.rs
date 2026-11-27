//! Application policy for the rc file: locate, read, merge and write it.
//!
//! The generic I/O crate owns atomic replacement only. Which file is the rc
//! file, how its diagnostics are formatted, and how `Settings` are merged into
//! an existing file belong to the application.

use std::io;
use std::path::{Path, PathBuf};

use chaus_core::settings::Settings;

/// Result of reading and validating the rc file.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsRead {
    pub settings: Settings,
    /// Human-readable `line N: message` warnings for the Preferences window.
    pub diagnostics: Vec<String>,
    pub path: Option<PathBuf>,
}

impl SettingsRead {
    /// Build a read result, mainly for deterministically testing the store.
    #[must_use]
    pub fn new(settings: Settings, diagnostics: Vec<String>, path: Option<PathBuf>) -> Self {
        Self {
            settings,
            diagnostics,
            path,
        }
    }
}

/// Conventional rc path for this platform.
#[must_use]
pub fn settings_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("_chausrc"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".chausrc"))
}

/// Parse the rc file from `path`, reporting diagnostics. A missing file is not
/// an error; it yields defaults and the path where the file would be created.
#[must_use]
pub fn read_settings_in(path: Option<&Path>) -> SettingsRead {
    let Some(path) = path else {
        return SettingsRead {
            settings: Settings::default(),
            diagnostics: Vec::new(),
            path: None,
        };
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return SettingsRead {
                settings: Settings::default(),
                diagnostics: Vec::new(),
                path: Some(path.to_path_buf()),
            };
        }
        Err(error) => {
            return SettingsRead {
                settings: Settings::default(),
                diagnostics: vec![format!("Could not read {}: {error}", path.display())],
                path: Some(path.to_path_buf()),
            };
        }
    };
    let parsed = chaus_core::settings::parse_rc(&source);
    let diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| format!("line {}: {}", diagnostic.line, diagnostic.message))
        .collect();
    SettingsRead {
        settings: parsed.settings,
        diagnostics,
        path: Some(path.to_path_buf()),
    }
}

/// Parse the rc file from the conventional location.
#[must_use]
pub fn read_settings() -> SettingsRead {
    read_settings_in(settings_path().as_deref())
}

/// Merge `settings` into the current rc source and commit it atomically.
///
/// The file is re-read immediately before the commit. A concurrent external
/// edit is reported instead of overwritten; `chaus_io::write_atomic` handles
/// the temp-file/replace part without knowing anything about rc syntax.
pub fn write_rc(path: &Path, settings: &Settings) -> io::Result<()> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let updated = chaus_core::settings::update_rc(&source, settings);
    let current = match std::fs::read_to_string(path) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    if current != source {
        return Err(io::Error::other(format!(
            "{} changed on disk; reopen Preferences and try again",
            path.display()
        )));
    }
    chaus_io::write_atomic(path, updated.as_bytes())
}
