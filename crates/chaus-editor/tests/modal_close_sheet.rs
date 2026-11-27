//! The close sheet must own every editing key with the real Chaus bindings.
//!
//! The older `close_sheet.rs` test never calls `bind_keys`, so it cannot
//! reproduce GPUI dispatching bindings before the sheet's key handler.

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

fn open_dirty_close_sheet(
    app: &mut TestAppContext,
) -> (Entity<Harness>, Entity<Editor>, &mut VisualTestContext) {
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
    cx.simulate_keystrokes("a");
    draw(cx);
    let allowed =
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
    assert!(!allowed, "a dirty close must show the sheet");
    draw(cx);
    assert!(cx.debug_bounds("chaus-close-sheet").is_some());
    (harness, editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn assert_document_is(editor: &Entity<Editor>, cx: &mut VisualTestContext, expected: &str) {
    cx.update(|_, app| {
        assert_eq!(
            editor.read(app).document().text(),
            expected,
            "no key or paste may mutate the document behind the close sheet"
        );
    });
}

#[test]
fn editing_keys_cannot_reach_the_document_through_the_close_sheet() {
    let mut app = TestAppContext::single();
    let (_harness, editor, cx) = open_dirty_close_sheet(&mut app);

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
        assert_document_is(&editor, cx, "a");
        assert!(
            cx.debug_bounds("chaus-close-sheet").is_some(),
            "{keys} must not dismiss the sheet"
        );
    }

    // Printable input can arrive independently of key-down.
    cx.simulate_input("Z");
    draw(cx);
    assert_document_is(&editor, cx, "a");

    // The real paste path goes through EntityInputHandler; its modal guard
    // must reject it even when the focus handle still points at the editor.
    cx.update(|window, app| {
        editor.update(app, |editor, cx| {
            editor.replace_text_in_range(None, "PASTED", window, cx);
        });
    });
    draw(cx);
    assert_document_is(&editor, cx, "a");
}

#[test]
fn escape_dismisses_only_the_close_sheet_and_leaves_the_document_untouched() {
    let mut app = TestAppContext::single();
    let (_harness, editor, cx) = open_dirty_close_sheet(&mut app);

    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.debug_bounds("chaus-close-sheet").is_none());
    assert_document_is(&editor, cx, "a");

    // Command shortcuts resume as soon as the sheet is gone.
    cx.simulate_keystrokes("b");
    draw(cx);
    assert_document_is(&editor, cx, "ab");
}
