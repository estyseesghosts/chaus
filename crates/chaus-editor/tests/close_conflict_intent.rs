//! Resolving a save conflict must preserve the close intent that caused it.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
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
    if cx.windows().is_empty() {
        return;
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
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

/// Press a Vim key and deliver the platform text message it suppresses, so the
/// next real `send_text` is not swallowed.
fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, key: &str) {
    cx.simulate_keystrokes(key);
    send_text(editor, cx, key);
}

fn open(path: &std::path::Path, vim: bool) -> (Entity<Editor>, VisualTestContext) {
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
        settings.vim = vim;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    (editor, cx.clone())
}

#[test]
fn overwriting_a_close_conflict_finishes_the_original_close() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("close-conflict.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (editor, mut cx) = open(&path, false);

    cx.simulate_keystrokes("X");
    draw(&mut cx);
    std::fs::write(&path, "external\n").unwrap();

    let allowed =
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
    assert!(!allowed);
    draw(&mut cx);
    let save = cx.debug_bounds("close-save").unwrap();
    cx.simulate_click(save.center(), Modifiers::default());
    draw(&mut cx);
    assert!(
        cx.debug_bounds("chaus-conflict-sheet").is_some(),
        "the save started from the close sheet must surface the conflict"
    );

    let overwrite = cx.debug_bounds("conflict-overwrite").unwrap();
    cx.simulate_click(overwrite.center(), Modifiers::default());
    draw(&mut cx);

    assert_eq!(
        cx.windows().len(),
        0,
        "resolving the conflict with Overwrite must complete the close, not leave the window open"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xone\n");
}

#[test]
fn cancelling_a_close_conflict_restores_the_close_sheet() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel-conflict.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (editor, mut cx) = open(&path, false);

    cx.simulate_keystrokes("X");
    draw(&mut cx);
    std::fs::write(&path, "external\n").unwrap();
    let _ =
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
    draw(&mut cx);
    let save = cx.debug_bounds("close-save").unwrap();
    cx.simulate_click(save.center(), Modifiers::default());
    draw(&mut cx);

    let cancel = cx.debug_bounds("conflict-cancel").unwrap();
    cx.simulate_click(cancel.center(), Modifiers::default());
    draw(&mut cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_none());
    assert!(
        cx.debug_bounds("chaus-close-sheet").is_some(),
        "Cancel must return to the close decision, not silently abandon it"
    );
}

#[test]
fn wq_conflict_keeps_the_exit_intent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wq-conflict.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (editor, mut cx) = open(&path, true);

    // Insert then return to Normal mode.
    press(&editor, &mut cx, "G");
    press(&editor, &mut cx, "A");
    send_text(&editor, &mut cx, "X");
    cx.simulate_keystrokes("escape");
    draw(&mut cx);
    std::fs::write(&path, "external\n").unwrap();

    // `:wq` must keep its Ex close intent when the save reports a conflict.
    cx.simulate_keystrokes(":");
    send_text(&editor, &mut cx, ":");
    send_text(&editor, &mut cx, "wq");
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert!(
        cx.debug_bounds("chaus-conflict-sheet").is_some(),
        "the Ex save must surface the conflict"
    );

    let reload = cx.debug_bounds("conflict-overwrite").unwrap();
    cx.simulate_click(reload.center(), Modifiers::default());
    draw(&mut cx);
    assert_eq!(
        cx.windows().len(),
        0,
        "an Overwrite chosen during `:wq` must still quit after the write"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\nX");
}
