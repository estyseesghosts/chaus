//! Linux file pickers backed by the desktop's command-line chooser.
//!
//! `zenity` is used on GNOME-family sessions and `kdialog` on KDE. Both are
//! plain helper processes, so this works under Wayland and X11 without linking
//! GTK or depending on a specific portal client.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Choice;

#[derive(Clone, Copy)]
enum Helper {
    Zenity,
    Kdialog,
}

impl Helper {
    fn program(self) -> &'static str {
        match self {
            Self::Zenity => "zenity",
            Self::Kdialog => "kdialog",
        }
    }
}

pub(crate) fn is_available() -> bool {
    helper().is_some()
}

pub(crate) fn choose_file(
    choice: Choice,
    title: &str,
    suggested: Option<&Path>,
) -> Option<PathBuf> {
    let helper = helper()?;
    let mut command = Command::new(helper.program());
    match helper {
        Helper::Zenity => {
            command.arg("--file-selection");
            if choice == Choice::Save {
                command.arg("--save").arg("--confirm-overwrite");
            }
            command.arg("--title").arg(title);
            if let Some(path) = suggested {
                command.arg("--filename").arg(path);
            }
        }
        Helper::Kdialog => {
            command.arg("--title").arg(title);
            command.arg(match choice {
                Choice::Open => "--getopenfilename",
                Choice::Save => "--getsavefilename",
            });
            if let Some(directory) = suggested.and_then(Path::parent) {
                if !directory.as_os_str().is_empty() {
                    command.arg(directory);
                }
            }
        }
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    path_from_stdout(output.stdout)
}

fn helper() -> Option<Helper> {
    [Helper::Zenity, Helper::Kdialog]
        .into_iter()
        .find(|helper| is_executable_in_path(helper.program()))
}

fn is_executable_in_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| {
        let candidate = directory.join(name);
        candidate
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    })
}

fn path_from_stdout(mut bytes: Vec<u8>) -> Option<PathBuf> {
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b'\n' | b'\r'))
        .map_or(0, |index| index + 1);
    bytes.truncate(end);
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}
