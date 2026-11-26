//! A disk check that loses its epoch must still release the in-flight flag.

use std::sync::{mpsc, Mutex};

use chaus_core::settings::Settings;
use chaus_editor::{set_disk_check_gate_for_test, Editor};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext,
    VisualTestContext, Window,
};

/// The gate is process-wide, so tests in this binary take turns.
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

#[test]
fn an_epoch_change_during_a_check_does_not_wedge_the_watcher() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wedge.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    let (release, gate) = mpsc::channel();
    set_disk_check_gate_for_test(Some(gate));
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
        assert!(editor.read(cx).is_checking_disk());
    });

    // The save/reload path bumps the epoch while the inspection is running.
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| editor.advance_disk_epoch_for_test());
    });
    release
        .send(())
        .expect("the inspection must still be waiting");
    draw(cx);

    cx.update(|_, cx| {
        assert!(
            !editor.read(cx).is_checking_disk(),
            "a stale-epoch result still owns and must clear the in-flight flag"
        );
    });

    // A later check must actually start and notice the change.
    std::fs::write(&path, "two\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
    });
    draw(cx);
    assert!(
        cx.debug_bounds("chaus-disk-banner").is_some(),
        "the watcher must not believe a stale check is still running"
    );
    cx.update(|_, cx| {
        assert!(!editor.read(cx).is_checking_disk());
    });
}

/// Keeping the gate armed across the test binary would stall unrelated tests.
#[test]
fn the_disk_check_gate_clears_itself() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    set_disk_check_gate_for_test(None);
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plain.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
    });
    draw(cx);
    cx.update(|_, cx| {
        assert!(!editor.read(cx).is_checking_disk());
    });
}
