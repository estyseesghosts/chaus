//! Bounded watcher error reporting.
//!
//! One failed inspection is a transient event; repeated failures must warn
//! once without repeating the warning on every two-second tick, and a later
//! successful inspection must clear it. Save still performs its own
//! authoritative conflict check regardless of the watcher status.

use std::path::Path;
use std::time::Duration;

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext,
    VisualTestContext, Window,
};

struct Harness {
    editor: Entity<Editor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone().into_any_element()
    }
}

fn initialize(app: &mut TestAppContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    while cx.executor().tick() {}
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// Run exactly one two-second watcher tick. `settle` runs ready work without
/// skipping the clock forward to the next timer.
fn tick(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(2));
    settle(cx);
    draw(cx);
}

fn status(cx: &mut VisualTestContext, editor: &Entity<Editor>) -> Option<String> {
    cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned))
}

/// Move the file aside and put a directory at its path. Opening the path then
/// fails on Windows and reading it fails on Unix: either way inspection fails.
fn make_unreadable(path: &Path, stash: &Path) {
    std::fs::rename(path, stash).expect("stash original file");
    std::fs::create_dir(path).expect("replace file with a directory");
}

/// Put the original file back so its identity and content match the baseline.
fn restore(path: &Path, stash: &Path) {
    std::fs::remove_dir(path).expect("remove directory");
    std::fs::rename(stash, path).expect("restore original file");
}

#[test]
fn transient_failures_stay_quiet_and_repeated_failures_warn_once() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("watched.txt");
    let stash = directory.path().join("stashed.txt");
    std::fs::write(&path, "one\n").expect("write fixture");
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let loaded = chaus_io::load(&path).expect("fixture loads");
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    settle(cx);
    draw(cx);
    assert_eq!(status(cx, &editor), None, "a clean watcher must be quiet");

    make_unreadable(&path, &stash);

    tick(cx);
    assert_eq!(
        status(cx, &editor),
        None,
        "one transient failure must not post a warning"
    );
    tick(cx);
    assert_eq!(
        status(cx, &editor),
        None,
        "two failures are still below the warning threshold"
    );

    tick(cx);
    let warning = status(cx, &editor).expect("repeated failure must warn once");
    assert!(
        warning.contains("Cannot inspect"),
        "unexpected watcher status: {warning}"
    );
    assert!(
        cx.debug_bounds("chaus-disk-banner").is_none(),
        "an inspection failure is not a content conflict banner"
    );

    cx.update(|_, cx| {
        editor.update(cx, |editor, _| editor.show_message("unrelated"));
    });
    tick(cx);
    tick(cx);
    assert_eq!(
        status(cx, &editor).as_deref(),
        Some("unrelated"),
        "later failing ticks must not re-post the warning or erase other status"
    );
}

#[test]
fn a_successful_inspection_clears_the_warning() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("watched.txt");
    let stash = directory.path().join("stashed.txt");
    std::fs::write(&path, "one\n").expect("write fixture");
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let loaded = chaus_io::load(&path).expect("fixture loads");
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    settle(cx);
    draw(cx);

    make_unreadable(&path, &stash);
    tick(cx);
    tick(cx);
    tick(cx);
    assert!(
        status(cx, &editor)
            .as_deref()
            .is_some_and(|message| message.contains("Cannot inspect")),
        "the warning must be visible before recovery"
    );

    restore(&path, &stash);
    tick(cx);
    assert_eq!(
        status(cx, &editor),
        None,
        "a successful inspection must clear the watcher warning"
    );
    assert!(
        cx.debug_bounds("chaus-disk-banner").is_none(),
        "the restored file must not look externally changed"
    );
}

#[test]
fn save_still_fails_hard_when_the_target_cannot_be_inspected() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("watched.txt");
    let stash = directory.path().join("stashed.txt");
    std::fs::write(&path, "one\n").expect("write fixture");
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let loaded = chaus_io::load(&path).expect("fixture loads");
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    settle(cx);
    draw(cx);

    cx.simulate_keystrokes("X");
    draw(cx);
    make_unreadable(&path, &stash);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.save_now(cx), "the save should start");
        });
    });
    settle(cx);
    draw(cx);

    let message = status(cx, &editor).expect("a failed save must be reported");
    assert!(
        message.contains("Save failed"),
        "an unreadable target must not be treated as unchanged, got: {message}"
    );
    assert!(
        cx.debug_bounds("chaus-conflict-sheet").is_none(),
        "an inspection failure is an I/O failure, not an overwrite prompt"
    );
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).document().text(),
            "Xone\n",
            "a failed save must not discard the edit"
        );
    });
    assert!(path.is_dir(), "the unreadable target must remain in place");
}
