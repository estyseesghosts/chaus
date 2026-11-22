//! Native file pickers.
//!
//! The picker is synchronous on the UI thread: it is a modal OS dialog, so
//! there is no document work to overlap with it. Paths are returned as
//! `PathBuf` so non-UTF-8 Windows names survive.

#[cfg(windows)]
mod windows;

/// Which dialog to show.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Open,
    Save,
}

/// Show the platform's file picker. Returns `None` when the person cancels or
/// the platform has no picker wired up.
#[must_use]
pub fn choose_file(
    choice: Choice,
    title: &str,
    suggested: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    platform::choose_file(choice, title, suggested)
}

#[cfg(windows)]
use windows as platform;

#[cfg(not(windows))]
mod platform {
    use std::path::{Path, PathBuf};

    use crate::Choice;

    pub(crate) fn choose_file(_: Choice, _: &str, _: Option<&Path>) -> Option<PathBuf> {
        // Linux v0.1 has no portable synchronous picker in GPUI's public API;
        // the caller reports this instead of silently creating a file.
        None
    }
}
