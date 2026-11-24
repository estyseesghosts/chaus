//! Local-first diagnostics for GUI launches.
//!
//! Events are small, structured lines in the user's state directory. They
//! contain phases, correlation ids and outcomes, never document text,
//! clipboard contents, keystrokes or file paths. The panic hook is installed
//! before argument parsing so initialization failures are recorded too.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Build identity injected by `build.rs`.
pub const BUILD_SHA: &str = env!("CHAUS_BUILD_SHA");
const MAX_MESSAGE: usize = 240;
const DEFAULT_MAX_BYTES: u64 = 256 * 1024;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SINK: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Best-effort local diagnostics directory, overridable for tests and portable runs.
#[must_use]
pub fn diagnostic_directory() -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os("CHAUS_DIAGNOSTICS_DIR") {
        return Some(PathBuf::from(directory));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("Chaus").join("Diagnostics"));
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .map(|base| base.join("chaus").join("diagnostics"))
}

/// Replace path-like tokens before a message is written to disk. This keeps a
/// panic message useful without turning it into a shareable document path.
fn redact(message: &str) -> String {
    let mut redacted = String::with_capacity(message.len().min(MAX_MESSAGE));
    for token in message.split_whitespace() {
        let looks_like_path = token.contains('/')
            || token.contains('\\')
            || (token.len() > 1 && token.as_bytes()[1] == b':');
        if !redacted.is_empty() {
            redacted.push(' ');
        }
        if looks_like_path {
            redacted.push_str("<path>");
        } else {
            redacted.push_str(token);
        }
        if redacted.len() >= MAX_MESSAGE {
            break;
        }
    }
    redacted.truncate(MAX_MESSAGE);
    redacted
}

fn rotate_if_needed(path: &Path) {
    let limit = std::env::var("CHAUS_DIAGNOSTICS_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_BYTES);
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < limit {
        return;
    }
    let backup = path.with_extension("1.log");
    let _ = fs::remove_file(&backup);
    let _ = fs::rename(path, &backup);
}

/// Append one structured event. Failures are ignored: diagnostics must never
/// change application behavior or recurse into the panic hook.
fn write_event(phase: &str, correlation: u64, outcome: &str) {
    let path = {
        let Ok(sink) = SINK.lock() else {
            return;
        };
        let Some(path) = sink.as_ref() else {
            return;
        };
        path.clone()
    };
    rotate_if_needed(&path);
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let _ = writeln!(
        file,
        "seq={sequence} time={timestamp} build={BUILD_SHA} correlation={correlation} phase={phase} outcome={outcome:?}"
    );
}

/// Replace or clear the active sink, for deterministic filesystem tests.
pub fn set_sink_for_test(path: Option<PathBuf>) {
    if let Ok(mut sink) = SINK.lock() {
        *sink = path;
    }
}

/// Install the sink and panic hook. Safe to call more than once; later calls
/// only replace the sink path.
pub fn init() {
    let Some(directory) = diagnostic_directory() else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let path = directory.join("chaus.log");
    {
        let Ok(mut sink) = SINK.lock() else {
            return;
        };
        *sink = Some(path);
    }
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("panic");
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), |site| site.file().to_owned());
        let location = Path::new(&location).file_name().map_or_else(
            || "unknown".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        write_event(
            "panic",
            0,
            &format!("{} at {}", redact(message), redact(&location)),
        );
    }));
    write_event("startup", 0, "ok");
}

/// Record a phase transition. `correlation` ties events for one window/save.
pub fn event(phase: &str, correlation: u64, outcome: &str) {
    write_event(&redact(phase), correlation, &redact(outcome));
}

/// Record an orderly shutdown.
pub fn shutdown() {
    write_event("shutdown", 0, "ok");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_path_like_tokens_and_keeps_messages_bounded() {
        let message = "failed C:/Users/terezi/notes.txt /home/terezi/secret.txt plain text";
        let redacted = redact(message);
        assert!(!redacted.contains("notes.txt"));
        assert!(!redacted.contains("secret.txt"));
        assert!(redacted.contains("<path>"));
        assert!(redacted.contains("plain text"));

        let long = "x".repeat(MAX_MESSAGE * 2);
        assert_eq!(redact(&long).len(), MAX_MESSAGE);
    }
}
