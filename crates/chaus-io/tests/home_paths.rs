//! Only a leading `~` component is expanded; everything else stays literal.

use std::path::{Path, PathBuf};

use chaus_io::expand_home;

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

#[test]
fn a_leading_tilde_component_expands_to_the_home_directory() {
    let Some(home) = home_directory() else {
        return;
    };
    assert_eq!(expand_home(Path::new("~")), home);
    assert_eq!(
        expand_home(Path::new("~/notes.txt")),
        home.join("notes.txt")
    );
    assert_eq!(
        expand_home(Path::new("~/nested/dir/file.txt")),
        home.join("nested").join("dir").join("file.txt")
    );
    if cfg!(windows) {
        assert_eq!(
            expand_home(Path::new("~\\notes.txt")),
            home.join("notes.txt")
        );
    }
}

#[test]
fn a_tilde_that_is_not_a_leading_component_is_literal() {
    for path in ["~user/notes", "notes/~/file", "a~b", "~/"] {
        let expanded = expand_home(Path::new(path));
        if path == "~/" {
            assert_eq!(expanded, home_directory().unwrap_or_default());
        } else {
            assert_eq!(expanded, Path::new(path), "{path:?} must stay literal");
        }
    }
}

#[test]
fn empty_and_relative_paths_are_unchanged() {
    assert_eq!(expand_home(Path::new("")), PathBuf::new());
    assert_eq!(expand_home(Path::new("notes.txt")), Path::new("notes.txt"));
    assert_eq!(
        expand_home(Path::new("nested/file.txt")),
        Path::new("nested/file.txt")
    );
}
