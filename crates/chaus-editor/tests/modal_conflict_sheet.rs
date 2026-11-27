//! The save-conflict sheet must own every editing key with the real bindings.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    AppContext as _, Context, Entity, EntityInputHandler as _, Focusable as _, IntoElement, Render,
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

fn open_conflict_sheet(
    app: &mut TestAppContext,
) -> (std::path::PathBuf, Entity<Editor>, &mut VisualTestContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conflict.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded.clone()), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    cx.simulate_keystrokes("X");
    draw(cx);
    std::fs::write(&path, "two\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.save_now(cx), "the save should start");
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_some());
    (path, editor, cx)
}

fn assert_document_is(editor: &Entity<Editor>, cx: &mut VisualTestContext, expected: &str) {
    cx.update(|_, app| {
        assert_eq!(
            editor.read(app).document().text(),
            expected,
            "no key or paste may mutate the document behind the conflict sheet"
        );
    });
}

#[test]
fn editing_keys_cannot_reach_the_document_through_the_conflict_sheet() {
    let mut app = TestAppContext::single();
    let (_path, editor, cx) = open_conflict_sheet(&mut app);

    for keys in [
        "backspace",
        "delete",
        "enter",
        "tab",
        "shift-tab",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "ctrl-a",
    ] {
        cx.simulate_keystrokes(keys);
        draw(cx);
        assert_document_is(&editor, cx, "Xone\n");
        assert!(
            cx.debug_bounds("chaus-conflict-sheet").is_some(),
            "{keys} must not dismiss the conflict sheet"
        );
    }

    cx.simulate_input("Z");
    draw(cx);
    assert_document_is(&editor, cx, "Xone\n");

    cx.update(|window, app| {
        editor.update(app, |editor, cx| {
            editor.replace_text_in_range(None, "PASTED", window, cx);
        });
    });
    draw(cx);
    assert_document_is(&editor, cx, "Xone\n");
}

#[test]
fn escape_cancels_only_the_conflict_sheet() {
    let mut app = TestAppContext::single();
    let (_path, editor, cx) = open_conflict_sheet(&mut app);

    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_none());
    assert_document_is(&editor, cx, "Xone\n");

    cx.simulate_keystrokes("b");
    draw(cx);
    assert_document_is(&editor, cx, "Xbone\n");
}
