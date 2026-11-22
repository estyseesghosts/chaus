//! The editor as the window's root view must dispatch every key binding.
//!
//! The app opens an `Editor` directly (no harness wrapper). Typing goes through
//! `ElementInputHandler` and can work even when action bindings fail, so these
//! tests assert the action path separately: Backspace, Delete, Shift+Arrow,
//! Ctrl+A, Ctrl+F and Ctrl+Shift+V. This mirrors the regression that motivated
//! tracking the focus handle on the rendered root element.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{Focusable as _, TestAppContext, VisualTestContext};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn text(editor: &gpui::Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

#[test]
fn root_editor_dispatches_deletion_selection_and_shortcuts() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let (editor, mut cx) = app.add_window_view(|window, cx| {
        let editor = Editor::new(cx, None, Settings::default());
        let focus = editor.focus_handle(cx);
        window.focus(&focus, cx);
        editor
    });
    draw(&mut cx);

    // The focused handle must be the one rendered with `ChausEditor`.
    let focus = cx.update(|_, cx| editor.read(cx).focus_handle(cx));
    assert!(
        cx.update(|window, _| focus.is_focused(window)),
        "the editor's focus handle must own keyboard focus"
    );

    cx.simulate_input("abc");
    draw(&mut cx);
    assert_eq!(text(&editor, &mut cx), "abc");

    // Backspace deletes the character before the caret.
    cx.simulate_keystrokes("backspace");
    draw(&mut cx);
    assert_eq!(
        text(&editor, &mut cx),
        "ab",
        "Backspace must dispatch through its action binding"
    );

    // Shift+Right extends a selection and Backspace deletes it.
    cx.simulate_keystrokes("home");
    draw(&mut cx);
    cx.simulate_keystrokes("shift-right");
    draw(&mut cx);
    assert!(
        !cx.update(|_, cx| editor.read(cx).selection().is_empty()),
        "Shift+Right must create a selection"
    );
    cx.simulate_keystrokes("backspace");
    draw(&mut cx);
    assert_eq!(
        text(&editor, &mut cx),
        "b",
        "Backspace must delete the selected character"
    );

    // Delete removes forward.
    cx.simulate_keystrokes("delete");
    draw(&mut cx);
    assert_eq!(
        text(&editor, &mut cx),
        "",
        "Delete must dispatch through its action binding"
    );

    // Ctrl+A selects everything, proving the modified shortcut path works.
    cx.simulate_input("hello");
    draw(&mut cx);
    cx.simulate_keystrokes("ctrl-a");
    draw(&mut cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).selection().range()),
        chaus_core::position::CharRange::new(
            chaus_core::position::CharOffset(0),
            chaus_core::position::CharOffset(5)
        ),
        "Ctrl+A must select the whole document"
    );

    // Ctrl+F opens the find bar, another bound action.
    cx.simulate_keystrokes("ctrl-f");
    draw(&mut cx);
    assert!(
        cx.debug_bounds("chaus-find-bar").is_some(),
        "Ctrl+F must open the find bar"
    );
    cx.simulate_keystrokes("escape");
    draw(&mut cx);

    // Ctrl+Shift+V toggles the modal engine.
    assert!(!cx.update(|_, cx| editor.read(cx).vim_enabled()));
    cx.simulate_keystrokes("ctrl-shift-v");
    draw(&mut cx);
    assert!(
        cx.update(|_, cx| editor.read(cx).vim_enabled()),
        "Ctrl+Shift+V must toggle Vim mode"
    );
}
