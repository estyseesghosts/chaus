//! The Preferences window reads persistence status from the store, not from a
//! snapshot taken when a write was queued. Both tests use GPUI's deterministic
//! executor; neither waits on a timer.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use chaus_app::preferences::{apply_for_test, new_for_test, save_error_for_test};
use chaus_app::settings_store::{install_store_for_test, SettingsStoreGlobal};
use chaus_core::settings::Settings;
use gpui::TestAppContext;

fn settings(tab_stop: usize) -> Settings {
    let mut value = Settings::default();
    value.tab_stop = tab_stop;
    value
}

/// A path whose parent is a regular file makes every disk write fail without
/// relying on permissions or a mock clock.
fn unwritable_path(directory: &Path) -> PathBuf {
    let blocker = directory.join("not-a-directory");
    std::fs::write(&blocker, b"occupied").expect("blocker file");
    blocker.join("rc")
}

fn initialize(cx: &mut gpui::App, path: PathBuf) {
    silvestris::test_support::initialize(
        silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
        cx,
    );
    chaus_app::app::install_settings(cx, settings(4));
    install_store_for_test(cx, Some(path), Vec::new());
}

#[test]
fn a_failed_background_write_is_visible_to_preferences() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = unwritable_path(directory.path());
    let mut app = TestAppContext::single();
    app.update(|cx| initialize(cx, path.clone()));

    let (preferences, cx) = app.add_window_view(new_for_test);
    let (notified, seen) = mpsc::channel();
    cx.update(|_, cx| {
        cx.observe(&preferences, move |_, _| {
            let _ = notified.send(());
        })
        .detach();
    });
    cx.run_until_parked();

    assert_eq!(
        cx.update(|_, cx| preferences.read_with(cx, save_error_for_test)),
        None,
        "queueing a write must not snapshot a status"
    );

    cx.update(|_, cx| {
        preferences.update(cx, apply_for_test);
    });
    while seen.try_recv().is_ok() {}

    cx.run_until_parked();
    let error = cx
        .update(|_, cx| preferences.read_with(cx, save_error_for_test))
        .expect("the failed write becomes visible after the background task settles");
    assert!(!error.is_empty(), "a disk failure must be reported");
    assert!(
        !path.exists(),
        "the failed write must not leave an rc file behind"
    );
    assert!(
        seen.try_recv().is_ok(),
        "the store must notify Preferences when its status changes"
    );
}

#[test]
fn shutdown_persists_the_newest_snapshot_after_an_older_writer_settles() {
    let directory = tempfile::tempdir().expect("temp directory");
    let path = directory.path().join("rc");
    std::fs::write(&path, b"set tabstop=2\n").expect("rc file");
    let app = TestAppContext::single();
    app.update(|cx| initialize(cx, path.clone()));

    let store = app.update(|cx| cx.global::<SettingsStoreGlobal>().0.clone());
    app.update(|cx| {
        store.update(cx, |owner, cx| owner.enqueue(settings(4), cx));
        store.update(cx, |owner, cx| owner.enqueue(settings(8), cx));
        store.update(cx, |owner, cx| owner.shutdown(cx)).detach();
    });
    app.run_until_parked();

    let source = std::fs::read_to_string(&path).expect("rc file still exists");
    assert!(
        source.contains("set tabstop=8"),
        "the newest accepted snapshot must win: {source}"
    );
    assert!(
        !source.contains("set tabstop=4"),
        "an older write must not remain: {source}"
    );
    assert!(
        app.update(|cx| store.read(cx).state().is_finished()),
        "shutdown must finish its serialized write"
    );
}
