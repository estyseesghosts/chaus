//! `ZZ` and `ZQ` through the real window lifecycle.
//!
//! They are thin compositions over the existing Ex save/quit paths, so these
//! tests check disk writes, window closure and in-flight-write guards.

use std::sync::{mpsc, Mutex};

use chaus_core::settings::Settings;
use chaus_editor::{set_save_gate_for_test, Editor};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Keystroke, Render,
    TestAppContext, VisualTestContext, Window,
};

/// The save gate is process-wide, so tests in this binary take turns.
static TEST_LOCK: Mutex<()> = Mutex::new(());

struct Harness {
    editor: Entity<Editor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone().into_any_element()
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    if cx.windows().is_empty() {
        return;
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        draw(cx);
    }
}

fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
    if cx.windows().is_empty() {
        // The key closed the window; a real OS would not deliver WM_CHAR to it.
        return;
    }
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

fn insert(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    press(editor, cx, "G");
    press(editor, cx, "A");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
    cx.simulate_keystrokes("escape");
    settle(cx);
}

/// Dispatch one key without draining the background executor. Used while the
/// save gate holds a write open.
fn dispatch(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = Keystroke::parse(key).expect("valid keystroke");
    cx.update(|window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    });
}

fn run(path: &std::path::Path, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let loaded = chaus_io::load(path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn status(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).status_text().unwrap_or_default().to_owned())
}

#[test]
fn zz_closes_a_clean_window_without_writing() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("clean.txt");
    std::fs::write(&path, "clean\n").unwrap();
    run(&path, |editor, cx| {
        // Deleting the file makes any attempt to "save" fail and keep the
        // window open, so a successful close proves the clean path wrote
        // nothing.
        std::fs::remove_file(&path).unwrap();
        press(&editor, cx, "Z");
        press(&editor, cx, "Z");
        settle(cx);
        assert_eq!(cx.windows().len(), 0);
    });
}

#[test]
fn zz_saves_a_dirty_document_and_closes() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dirty.txt");
    std::fs::write(&path, "one\n").unwrap();
    run(&path, |editor, cx| {
        insert(&editor, cx, "two");
        press(&editor, cx, "Z");
        press(&editor, cx, "Z");
        settle(cx);
        assert_eq!(cx.windows().len(), 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo");
    });
}

#[test]
fn zq_discards_changes_and_closes() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("discard.txt");
    std::fs::write(&path, "keep\n").unwrap();
    run(&path, |editor, cx| {
        insert(&editor, cx, "discard me");
        press(&editor, cx, "Z");
        press(&editor, cx, "Q");
        settle(cx);
        assert_eq!(cx.windows().len(), 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n");
    });
}

#[test]
fn zq_waits_for_an_in_flight_save() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saving.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (release, wait) = mpsc::channel();
    set_save_gate_for_test(Some(wait));
    run(&path, |editor, cx| {
        insert(&editor, cx, "two");
        let started = cx.update(|_, cx| editor.update(cx, |editor, cx| editor.save_now(cx)));
        assert!(started, "the save should start");
        // Do not run the executor: the gated save must stay in flight while
        // ZQ is dispatched.
        dispatch(cx, "Z");
        dispatch(cx, "Q");
        assert_eq!(cx.windows().len(), 1, ":q! must not close during a save");
        assert!(
            status(&editor, cx).contains("still running"),
            "{}",
            status(&editor, cx)
        );
        release.send(()).expect("save task is still waiting");
        settle(cx);
        assert_eq!(
            cx.windows().len(),
            1,
            "the refused ZQ must not close once the save lands"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo");
    });
}
