//! The recovery sheet must be usable without a pointer: focus starts on
//! Recover, Tab cycles the three buttons, and Enter/Space activate the focused
//! one. Regression: the sheet showed accessible buttons but only mouse clicks
//! worked, so Tab/Enter fell through to the editor behind it.

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

fn editor_window(app: &mut TestAppContext) -> (Entity<Harness>, &mut VisualTestContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        // The real app binds Tab/Enter to editor actions. Without these the
        // test would not catch a sheet that lets those bindings run behind it.
        chaus_editor::bind_keys(cx);
    });
    app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    })
}

fn queue(editor: &Entity<Editor>, cx: &mut VisualTestContext, directory: &std::path::Path) {
    let entries = recovery::scan_in_without_age(directory);
    assert!(!entries.is_empty());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.queue_recovery(entries, window, cx));
        });
    });
}

/// GPUI synthesizes a button's keyboard click on key-up, the same way a real
/// platform delivers an Enter/Space press. `simulate_keystrokes` only sends the
/// key-down, so tests must release the key explicitly.
fn press_and_release(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse(key).unwrap(),
    });
    draw(cx);
}

#[test]
fn enter_activates_the_initially_focused_recover_button() {
    let directory = tempfile::tempdir().unwrap();
    let record = RecoveryRecord::new(
        71,
        Some(std::path::PathBuf::from("C:/notes/keyboard.txt")),
        "recovered by keyboard",
        Eol::Lf,
        false,
        1,
    );
    recovery::write_to(directory.path(), &record).unwrap();

    let mut app = TestAppContext::single();
    let (harness, cx) = editor_window(&mut app);
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    // The real app has drawn and focused the document before the background
    // scan resolves, so reproduce that order here.
    draw(cx);
    queue(&editor, cx, directory.path());
    draw(cx);

    press_and_release(cx, "enter");

    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_none(),
        "Enter on the focused Recover button must close the sheet"
    );
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "recovered by keyboard");
    });
}

#[test]
fn tab_reaches_discard_all_and_enter_clears_every_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    for id in [72, 73, 74] {
        let record = RecoveryRecord::new(id, None, format!("snapshot {id}"), Eol::Lf, false, 1);
        recovery::write_to(directory.path(), &record).unwrap();
    }

    let mut app = TestAppContext::single();
    let (harness, cx) = editor_window(&mut app);
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    queue(&editor, cx, directory.path());
    draw(cx);

    // Recover -> Discard -> Discard All, then activate it.
    cx.simulate_keystrokes("tab tab");
    press_and_release(cx, "enter");

    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_none(),
        "Discard All must close the sheet"
    );
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "");
    });
    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "the keyboard path must delete every snapshot file"
    );
}

#[test]
fn space_activates_the_focused_discard_button() {
    let directory = tempfile::tempdir().unwrap();
    let record = RecoveryRecord::new(75, None, "discard from keyboard", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();

    let mut app = TestAppContext::single();
    let (harness, cx) = editor_window(&mut app);
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    queue(&editor, cx, directory.path());
    draw(cx);

    // Recover -> Discard, then activate it with Space.
    cx.simulate_keystrokes("tab");
    press_and_release(cx, "space");

    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "");
    });
    assert!(recovery::scan_in_without_age(directory.path()).is_empty());
}

/// One Enter press must activate Discard exactly once. GPUI synthesizes the
/// click on key-up, so a sheet-level key handler would otherwise discard twice.
#[test]
fn one_enter_press_discards_only_one_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    for id in [76, 77, 78] {
        let record = RecoveryRecord::new(id, None, format!("snapshot {id}"), Eol::Lf, false, 1);
        recovery::write_to(directory.path(), &record).unwrap();
    }

    let mut app = TestAppContext::single();
    let (harness, cx) = editor_window(&mut app);
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    queue(&editor, cx, directory.path());
    draw(cx);

    cx.simulate_keystrokes("tab");
    press_and_release(cx, "enter");

    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_some(),
        "two snapshots still need a choice"
    );
    assert_eq!(
        recovery::scan_in_without_age(directory.path()).len(),
        2,
        "a single Discard must retire exactly one snapshot"
    );
}

/// The background scan can land after the window already focused the document.
/// The sheet must still consume Tab/Enter from the document instead of letting
/// them type into it.
#[test]
fn tab_and_enter_work_when_the_document_still_owns_focus() {
    let directory = tempfile::tempdir().unwrap();
    let record = RecoveryRecord::new(79, None, "focus fallback", Eol::Lf, false, 1);
    recovery::write_to(directory.path(), &record).unwrap();

    let mut app = TestAppContext::single();
    let (harness, cx) = editor_window(&mut app);
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    queue(&editor, cx, directory.path());

    // Deliberately put focus back on the document without drawing, so the
    // first-frame focus repair cannot help. Keys must still reach the sheet.
    let editor_focus = cx.update(|_, cx| editor.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&editor_focus, cx));

    cx.simulate_keystrokes("tab tab");
    press_and_release(cx, "enter");

    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_none(),
        "Tab then Enter must discard through the sheet even when the document \
         owned focus"
    );
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).document().text(),
            "",
            "Tab must not indent, and Enter must not insert a newline"
        );
    });
    assert!(recovery::scan_in_without_age(directory.path()).is_empty());
}
