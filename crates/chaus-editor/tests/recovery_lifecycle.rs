//! Retirement is off the UI thread and keeps the epoch ordering barrier.

use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use chaus_core::eol::Eol;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::recovery::{self, RecoveryRecord};
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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn retirement_does_not_block_the_ui_and_removes_the_snapshot() {
    let _test = TEST_LOCK
        .lock()
        .expect("recovery lifecycle test lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let previous = std::env::var_os("CHAUS_RECOVERY_DIR");
    std::env::set_var("CHAUS_RECOVERY_DIR", directory.path());

    let (release, gate) = mpsc::channel();
    recovery::set_retire_gate_for_test(Some(gate));

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    let key = cx.update(|_, cx| editor.read(cx).recovery_key_for_test());
    let record = RecoveryRecord::new(key, None, "dirty", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();
    assert_eq!(recovery::list_in(directory.path()).len(), 1);

    // The gate holds the background retirement. The editor update must return
    // without waiting for the filesystem.
    let started = Instant::now();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.retire_snapshot_for_test(cx));
    });
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "retirement blocked the editor update: {elapsed:?}"
    );
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).title()),
        "Untitled",
        "the editor must still answer state queries while retirement is pending"
    );
    assert_eq!(
        recovery::list_in(directory.path()).len(),
        1,
        "the gate must hold the retirement before it touches the filesystem"
    );

    release.send(()).expect("retirement task is still waiting");
    draw(cx);
    assert!(
        recovery::list_in(directory.path()).is_empty(),
        "the snapshot must be gone once the background retirement runs"
    );

    recovery::set_retire_gate_for_test(None);
    match previous {
        Some(value) => std::env::set_var("CHAUS_RECOVERY_DIR", value),
        None => std::env::remove_var("CHAUS_RECOVERY_DIR"),
    }
}
