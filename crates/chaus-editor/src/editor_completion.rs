//! Tab completion for the path argument of `:e`, `:o`, `:w`, `:saveas` and
//! `:r`.
//!
//! The directory is listed on the background executor with fixed bounds, and
//! the answer is applied only if the command line is still the one it was
//! computed for. Completion never opens, creates or writes a file. A leading
//! `~` is expanded for the directory scan only; the visible line keeps the
//! `~/...` spelling so the command still means the same thing when it runs.

use std::path::{Path, PathBuf};

use gpui::Context;

use crate::editor::Editor;

/// Directory entries examined per Tab press.
const MAX_SCANNED: usize = 4096;
/// Matching names kept per Tab press.
const MAX_CANDIDATES: usize = 256;
/// Names listed in the status when several match.
const SHOWN_CANDIDATES: usize = 8;

#[derive(Debug, Default)]
struct Listing {
    /// Matching UTF-8 names and whether each is a directory, sorted.
    names: Vec<(String, bool)>,
    /// More entries existed than were examined or kept.
    truncated: bool,
    error: Option<String>,
}

impl Editor {
    pub(crate) fn complete_command_path(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.vim_engine.completion_request() else {
            self.flash_status(
                "Tab completes a file path at the end of :e, :o, :w, :saveas or :r",
                cx,
            );
            return;
        };
        let (directory, prefix) = if request.typed == "~" {
            // Bare `~` means the home directory. Spell the completed argument
            // as `~/...`; `/` is accepted on both platforms.
            ("~/".to_owned(), String::new())
        } else {
            let (directory, prefix) = split_typed(&request.typed);
            (directory.to_owned(), prefix.to_owned())
        };
        let listed_directory = if request.quoted {
            PathBuf::from(display_dir(&directory))
        } else {
            chaus_io::expand_home(Path::new(display_dir(&directory)))
        };
        let listed_prefix = prefix.clone();
        let task = cx
            .background_executor()
            .spawn(async move { list_matches(&listed_directory, &listed_prefix) });
        cx.spawn(async move |editor, cx| {
            let listing = task.await;
            editor
                .update(cx, |editor, cx| {
                    editor.finish_completion(request.revision, &directory, &prefix, listing, cx);
                })
                .ok();
        })
        .detach();
    }

    fn finish_completion(
        &mut self,
        revision: u64,
        directory: &str,
        prefix: &str,
        listing: Listing,
        cx: &mut Context<Self>,
    ) {
        if let Some(error) = listing.error {
            self.flash_status(
                format!("Cannot list {}: {error}", display_dir(directory)),
                cx,
            );
            cx.notify();
            return;
        }
        let separator = if directory.contains('\\') { '\\' } else { '/' };
        let (completed, finished) = match listing.names.as_slice() {
            [] => {
                self.flash_status("No matching files", cx);
                cx.notify();
                return;
            }
            [(name, is_directory)] => {
                let mut completed = format!("{directory}{name}");
                if *is_directory {
                    completed.push(separator);
                }
                (completed, !*is_directory)
            }
            names => {
                let mut common = common_prefix(names.iter().map(|(name, _)| name.as_str()));
                if common.chars().count() < prefix.chars().count() {
                    common = prefix.to_owned();
                }
                let mut shown: Vec<&str> = names
                    .iter()
                    .take(SHOWN_CANDIDATES)
                    .map(|(name, _)| name.as_str())
                    .collect();
                let more = names.len() - shown.len();
                if more > 0 || listing.truncated {
                    shown.push("...");
                }
                self.flash_status(shown.join("  "), cx);
                (format!("{directory}{common}"), false)
            }
        };
        if let Some(message) = self
            .vim_engine
            .apply_completion(revision, &completed, finished)
        {
            self.flash_status(message, cx);
        }
        cx.notify();
    }
}

/// Split the typed path at its last separator: the directory part (kept as
/// typed, including the separator) and the name prefix being completed.
fn split_typed(typed: &str) -> (&str, &str) {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    match typed.rfind(separators) {
        Some(index) => typed.split_at(index + 1),
        None => ("", typed),
    }
}

fn display_dir(directory: &str) -> &str {
    if directory.is_empty() {
        "."
    } else {
        directory
    }
}

fn matches_prefix(name: &str, prefix: &str) -> bool {
    if cfg!(windows) {
        name.to_lowercase().starts_with(&prefix.to_lowercase())
    } else {
        name.starts_with(prefix)
    }
}

/// List names in `directory` that start with `prefix`. The caller has already
/// expanded a leading `~`; non-UTF-8 names are skipped because they cannot be
/// spelled back into the command line losslessly.
fn list_matches(directory: &Path, prefix: &str) -> Listing {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            return Listing {
                error: Some(error.to_string()),
                ..Listing::default()
            };
        }
    };
    let mut listing = Listing::default();
    for (scanned, entry) in entries.enumerate() {
        if scanned >= MAX_SCANNED {
            listing.truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Hidden entries only when asked for, as in a shell.
        if name.starts_with('.') && !prefix.starts_with('.') {
            continue;
        }
        if !matches_prefix(&name, prefix) {
            continue;
        }
        if listing.names.len() >= MAX_CANDIDATES {
            listing.truncated = true;
            break;
        }
        // Follow symlinks so a link to a directory completes like one.
        let is_directory = std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
        listing.names.push((name, is_directory));
    }
    listing.names.sort();
    listing
}

/// Longest common prefix by Unicode scalar, compared exactly. On Windows a
/// case-insensitive match can share no exact prefix; the typed text then stays.
fn common_prefix<'a>(mut names: impl Iterator<Item = &'a str>) -> String {
    let Some(first) = names.next() else {
        return String::new();
    };
    let mut common: Vec<char> = first.chars().collect();
    for name in names {
        let shared = common
            .iter()
            .zip(name.chars())
            .take_while(|(left, right)| **left == *right)
            .count();
        common.truncate(shared);
    }
    common.into_iter().collect()
}
