//! The last window close must drain pending recovery work before exit.
//!
//! Regression: retirement was detached from the UI thread, so closing the last
//! window could exit the process before the `.rec` file was deleted. Every
//! discarded snapshot then reappeared on the next launch.
//!
//! The editor's retirement ticket is verified in
//! `chaus-editor/tests/recovery_lifecycle.rs`. This test keeps a ticket held by
//! a real thread, because the deterministic test executor cannot poll a
//! background task while the exit path deliberately blocks the UI thread.

use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use chaus_core::eol::Eol;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::recovery::{self, PendingRecoveryOp, RecoveryRecord};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext,
    VisualTestContext, Window,
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard(Option<std::ffi::OsString>);

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => std::env::set_var("CHAUS_RECOVERY_DIR", value),
            None => std::env::remove_var("CHAUS_RECOVERY_DIR"),
        }
    }
}

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
fn closing_the_last_window_waits_for_pending_recovery_retirement() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let _env = EnvGuard(std::env::var_os("CHAUS_RECOVERY_DIR"));
    std::env::set_var("CHAUS_RECOVERY_DIR", directory.path());

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        // This is the same callback `app::run` installs for the real app.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                chaus_app::app::flush_and_drain_on_exit(cx);
            }
        })
        .detach();
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    let mut cx = cx.clone();
    draw(&mut cx);

    // A snapshot that a real retirement would delete.
    let key = cx.update(|_, cx| editor.read(cx).recovery_key_for_test());
    let record = RecoveryRecord::new(key, None, "unsaved", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();
    assert_eq!(recovery::scan_in_without_age(directory.path()).len(), 1);

    // Hold a ticket in a real thread, release it 200 ms after the close
    // begins, then perform the actual retirement. The app-owned drain must
    // wait for both the ticket and the deletion, not quit immediately.
    let (release, gate) = mpsc::channel();
    let ticket = PendingRecoveryOp::begin();
    let worker = std::thread::spawn(move || {
        let _ticket = ticket;
        let _ = gate.recv();
        recovery::retire(key, 1).expect("retirement succeeds");
    });

    let closed = Instant::now();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        release.send(()).expect("the ticket is still held");
    });

    // Closes the only window. `on_window_closed` runs the exit drain inline.
    cx.update(|window, _| window.remove_window());
    let waited = closed.elapsed();
    worker.join().unwrap();
    releaser.join().unwrap();

    assert!(
        waited >= Duration::from_millis(150),
        "exit must wait for pending recovery work, waited only {waited:?}"
    );
    assert_eq!(
        recovery::pending_operations(),
        0,
        "the shutdown drain must leave no recovery operation behind"
    );
    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "the snapshot must be deleted before the process exits"
    );
}
