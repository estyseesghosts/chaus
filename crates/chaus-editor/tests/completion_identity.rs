//! Real-window completion identity: a result is verified before any status,
//! command-line edit or repaint, including errors and candidate lists.

use std::ffi::OsString;
use std::sync::Mutex;
use std::time::Duration;

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Keystroke, Render,
    TestAppContext, VisualTestContext, Window,
};

struct Harness {
    editor: Entity<Editor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone().into_any_element()
    }
}

/// Longer than any number of executor turns, shorter than the four-second
/// transient-status timer, so a status from a newer result stays observable.
const COMPLETION_DELAY: Duration = Duration::from_secs(1);

/// The completion delay is process-wide, so delayed tests take turns.
static TEST_LOCK: Mutex<()> = Mutex::new(());
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct DelayGuard;

impl Drop for DelayGuard {
    fn drop(&mut self) {
        Editor::clear_completion_delay_for_test();
    }
}

struct HomeGuard(Option<OsString>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        if let Some(home) = self.0.take() {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }
}

fn open() -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, None, settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    cx.run_until_parked();
    (editor, cx.clone())
}

/// Dispatch one key without running the executor. Used while a result is held.
fn dispatch(cx: &mut VisualTestContext, key: &'static str) {
    if cx.windows().is_empty() {
        return;
    }
    let keystroke = Keystroke::parse(key).unwrap_or_else(|_| panic!("valid key: {key}"));
    cx.update(|window, cx| window.dispatch_keystroke(keystroke, cx));
}

fn send_text(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    if cx.windows().is_empty() {
        return;
    }
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

/// Type an Ex line. The first text message is the WM_CHAR for `:` and is
/// suppressed by the modal engine, as on Windows.
fn type_ex(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    dispatch(cx, ":");
    send_text(editor, cx, ":");
    send_text(editor, cx, text);
}

/// Run ready tasks only. `run_until_parked` would skip the clock to the
/// transient-status timer and erase the message under test.
fn drain(cx: &mut VisualTestContext) {
    while cx.executor().tick() {}
}

fn line(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| editor.read(cx).command_line_text())
}

fn status(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).status_text().unwrap_or_default().to_owned())
}

fn document_text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn delay(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.delay_next_completion_for_test(COMPLETION_DELAY);
        });
    });
}

fn deliver(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(COMPLETION_DELAY);
    drain(cx);
}

#[test]
fn tab_escape_and_a_new_identical_line_drops_the_directory_error() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let typed = format!("e {}", blocked.join("miss").display());

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(line(&editor, &mut cx).as_deref(), Some(typed.as_str()));

    dispatch(&mut cx, "escape");
    // The new line has the same text and the same per-line revision count.
    type_ex(&editor, &mut cx, &typed);
    assert_eq!(line(&editor, &mut cx).as_deref(), Some(typed.as_str()));
    deliver(&mut cx);

    let late = status(&editor, &mut cx);
    assert!(!late.contains("Cannot list"), "{late}");
    assert_eq!(line(&editor, &mut cx).as_deref(), Some(typed.as_str()));

    // The current request still gets its own error.
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    let current = status(&editor, &mut cx);
    assert!(current.contains("Cannot list"), "{current}");
}

#[test]
fn tab_escape_and_a_new_identical_line_drops_the_completion() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("alpha.txt");
    std::fs::write(&file, "alpha\n").unwrap();
    let typed = format!("e {}", directory.path().join("al").display());

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    dispatch(&mut cx, "escape");
    type_ex(&editor, &mut cx, &typed);
    deliver(&mut cx);
    assert_eq!(
        line(&editor, &mut cx).as_deref(),
        Some(typed.as_str()),
        "a completion from the previous line must not edit this one"
    );

    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(
        line(&editor, &mut cx).as_deref(),
        Some(format!("e {}", file.display()).as_str())
    );
}

#[test]
fn a_slow_error_does_not_disturb_a_valid_command() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    let valid = directory.path().join("valid.txt");
    std::fs::write(&valid, "from valid\n").unwrap();

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(
        &editor,
        &mut cx,
        &format!("e {}", blocked.join("miss").display()),
    );
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    dispatch(&mut cx, "escape");
    type_ex(&editor, &mut cx, &format!("e {}", valid.display()));
    dispatch(&mut cx, "enter");
    deliver(&mut cx);

    assert_eq!(document_text(&editor, &mut cx), "from valid\n");
    let late = status(&editor, &mut cx);
    assert!(!late.contains("Cannot list"), "{late}");
}

#[test]
fn repeated_tabs_accept_only_the_newest_result() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("alpha.txt");
    std::fs::write(&first, "alpha\n").unwrap();
    let typed = format!("e {}", directory.path().join("al").display());

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    // The second request sees no match; its status is the authoritative one.
    std::fs::rename(&first, directory.path().join("beta.txt")).unwrap();
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(status(&editor, &mut cx), "No matching files");

    deliver(&mut cx);
    assert_eq!(
        status(&editor, &mut cx),
        "No matching files",
        "the first Tab must not overwrite a newer result"
    );
    assert_eq!(line(&editor, &mut cx).as_deref(), Some(typed.as_str()));
}

#[test]
fn toggling_vim_off_discards_a_pending_result() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("alpha.txt");
    std::fs::write(&file, "alpha\n").unwrap();
    let typed = format!("e {}", directory.path().join("al").display());

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    dispatch(&mut cx, "ctrl-shift-v");
    drain(&mut cx);
    assert_eq!(line(&editor, &mut cx), None);
    deliver(&mut cx);
    assert_eq!(line(&editor, &mut cx), None);
    assert!(
        !status(&editor, &mut cx).contains("alpha.txt"),
        "a stale candidate must not appear after Vim was toggled off"
    );

    dispatch(&mut cx, "ctrl-shift-v");
    drain(&mut cx);
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(
        line(&editor, &mut cx).as_deref(),
        Some(format!("e {}", file.display()).as_str())
    );
}

#[test]
fn unicode_quoted_and_home_paths_complete_with_a_current_token() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let completed = directory.path().join("naïve.txt");
    std::fs::write(&completed, "naive\n").unwrap();

    let (editor, mut cx) = open();
    let typed = format!("e \"{}", directory.path().join("na").display());
    type_ex(&editor, &mut cx, &typed);
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(
        line(&editor, &mut cx).as_deref(),
        Some(format!("e \"{}\"", completed.display()).as_str())
    );

    let home = directory.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("café.txt"), "cafe\n").unwrap();
    let _home_lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    let _home_guard = HomeGuard(previous);

    dispatch(&mut cx, "escape");
    type_ex(&editor, &mut cx, "e ~/caf");
    dispatch(&mut cx, "tab");
    drain(&mut cx);
    assert_eq!(
        line(&editor, &mut cx).as_deref(),
        Some("e ~/café.txt"),
        "a bare `~` still expands for the scan but keeps its spelling"
    );
}

#[test]
fn a_closed_window_ignores_a_late_result() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("alpha.txt");
    std::fs::write(&file, "alpha\n").unwrap();

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(
        &editor,
        &mut cx,
        &format!("e {}", directory.path().join("al").display()),
    );
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    dispatch(&mut cx, "escape");
    type_ex(&editor, &mut cx, "q");
    dispatch(&mut cx, "enter");
    assert!(cx.windows().is_empty(), "a clean document closes on :q");

    deliver(&mut cx);
    assert!(cx.windows().is_empty());
}

#[test]
fn submitting_the_command_line_invalidates_a_pending_result() {
    let _test = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = DelayGuard;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("alpha.txt");
    std::fs::write(&file, "alpha\n").unwrap();

    let (editor, mut cx) = open();
    delay(&editor, &mut cx);
    type_ex(
        &editor,
        &mut cx,
        &format!("e {}", directory.path().join("al").display()),
    );
    dispatch(&mut cx, "tab");
    drain(&mut cx);

    dispatch(&mut cx, "escape");
    type_ex(&editor, &mut cx, "set ts=2");
    dispatch(&mut cx, "enter");
    assert_eq!(line(&editor, &mut cx), None);

    deliver(&mut cx);
    let late = status(&editor, &mut cx);
    assert!(!late.contains("alpha.txt"), "{late}");
    assert_eq!(line(&editor, &mut cx), None);
}
