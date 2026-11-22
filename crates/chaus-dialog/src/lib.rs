//! Native file pickers.
//!
//! The picker is synchronous on the UI thread: it is a modal OS dialog, so
//! there is no document work to overlap with it. Paths are returned as
//! `PathBuf` so non-UTF-8 names survive.

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

/// Which dialog to show.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Open,
    Save,
}

/// Whether this build has a native file picker.
///
/// Callers should check this before [`choose_file`] so that "the platform has
/// no picker" is not reported to people as "you cancelled".
#[must_use]
pub fn is_available() -> bool {
    platform::is_available()
}

/// Show the platform's file picker. Returns `None` when the person cancels or
/// the platform has no picker wired up; callers distinguish the two with
/// [`is_available`].
#[must_use]
pub fn choose_file(choice: Choice, title: &str, suggested: Option<&Path>) -> Option<PathBuf> {
    platform::choose_file(choice, title, suggested)
}

#[cfg(windows)]
use windows as platform;

/// Linux uses the desktop's command-line chooser when one is installed:
/// `zenity` on GNOME-family desktops and `kdialog` on KDE. This keeps the
/// dialog out of the process and works under both Wayland and X11 without
/// linking GTK or a portal client.
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use std::path::{Path, PathBuf};

    use crate::Choice;

    pub(crate) fn is_available() -> bool {
        false
    }

    pub(crate) fn choose_file(_: Choice, _: &str, _: Option<&Path>) -> Option<PathBuf> {
        None
    }
}
