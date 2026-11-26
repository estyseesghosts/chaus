use chaus_core::eol::Eol;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::recovery::{self, RecoveryRecord};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, Render,
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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn queue(editor: &Entity<Editor>, cx: &mut VisualTestContext, directory: &std::path::Path) {
    let entries = recovery::scan_in_without_age(directory);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.queue_recovery(entries.clone(), window, cx));
        });
    });
}

#[test]
fn a_recovery_snapshot_can_be_recovered_or_discarded() {
    let directory = tempfile::tempdir().unwrap();
    let first = RecoveryRecord::new(
        41,
        Some(std::path::PathBuf::from("C:/notes/example.txt")),
        "one\r\ntwo\r\n",
        Eol::CrLf,
        false,
        3,
    );
    recovery::write_to(directory.path(), &first).unwrap();

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
    queue(&editor, cx, directory.path());
    draw(cx);
    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_some(),
        "the launch recovery sheet must be visible"
    );

    let recover = cx.debug_bounds("recovery-recover").unwrap();
    cx.simulate_click(recover.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "one\ntwo\n");
        assert!(editor.read(cx).document().is_dirty());
        assert_eq!(
            editor.read(cx).recovery_metadata_for_test(),
            (
                Some(std::path::PathBuf::from("C:/notes/example.txt")),
                false
            )
        );
    });
}

#[test]
fn a_snapshot_can_be_discarded_without_recovering_it() {
    let directory = tempfile::tempdir().unwrap();
    let record = RecoveryRecord::new(42, None, "discard me", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();

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
    queue(&editor, cx, directory.path());
    draw(cx);
    let discard = cx.debug_bounds("recovery-discard").unwrap();
    cx.simulate_click(discard.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "");
    });
    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "Discard must remove the snapshot from disk"
    );
}

/// A window that already contains typed work must refuse the queue instead of
/// replacing it when the person later clicks Recover.
#[test]
fn a_dirty_host_window_refuses_the_recovery_queue() {
    let directory = tempfile::tempdir().unwrap();
    let record = RecoveryRecord::new(50, None, "snapshot", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    cx.simulate_keystrokes("t y p e d");
    draw(cx);

    let entries = recovery::scan_in_without_age(directory.path());
    let accepted = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.queue_recovery(entries, window, cx))
    });
    assert!(
        !accepted,
        "a dirty document must not accept recovery records"
    );
    draw(cx);
    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "typed");
    });
}

/// Discard All must remove every snapshot file, not just clear the in-memory
/// queue. Otherwise the whole pile returns on the next launch.
#[test]
fn discard_all_removes_every_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    for id in [60, 61, 62] {
        let record = RecoveryRecord::new(id, None, format!("snapshot {id}"), Eol::Lf, false, 1);
        recovery::write_to(directory.path(), &record).unwrap();
    }

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
    queue(&editor, cx, directory.path());
    draw(cx);
    let discard_all = cx.debug_bounds("recovery-discard-all").unwrap();
    cx.simulate_click(discard_all.center(), Modifiers::default());
    draw(cx);

    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "Discard All must remove every .rec file"
    );
}
