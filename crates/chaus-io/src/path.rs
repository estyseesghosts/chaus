//! Expansion of the one path spelling the Ex grammar interprets.
//!
//! Only a leading `~` component is special. The parser keeps the typed path so
//! `chaus-vim` never touches the environment; the editor calls this helper once
//! when it turns an Ex path into a file operation.

use std::path::{Path, PathBuf};

/// Replace a leading `~` component with the current user's home directory.
///
/// The grammar only produces `~`, `~/...`, and on Windows `~\...`; `~user` is
/// rejected before this function runs. A quoted path never reaches here, so a
/// literal directory named `~` is preserved. If the platform does not expose a
/// home directory the path is returned unchanged.
#[must_use]
pub fn expand_home(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    let rest = if text == "~" {
        Some("")
    } else if let Some(rest) = text.strip_prefix("~/") {
        Some(rest)
    } else if cfg!(windows) {
        text.strip_prefix("~\\")
    } else {
        None
    };
    let Some(rest) = rest else {
        return path.to_path_buf();
    };
    let Some(home) = home_directory() else {
        return path.to_path_buf();
    };
    if rest.is_empty() {
        home
    } else {
        home.join(rest)
    }
}

/// The current user's home directory from the environment.
fn home_directory() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
            return Some(PathBuf::from(profile));
        }
        let drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty())?;
        let path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty())?;
        let mut home = PathBuf::from(drive);
        home.push(path);
        return Some(home);
    }
    None
}
