//! Keyboard actions must reach the editor, not just its text-input handler.
//!
//! Typing and IME edits arrive through `ElementInputHandler` and work even
//! when the editor's key context is missing from the focus path; Backspace,
//! Delete, Home and the modified shortcuts only run if the element tracks the
//! editor focus handle and declares the `ChausEditor` context.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
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

fn run(test: impl FnOnce(Entity<Harness>, &mut VisualTestContext)) {
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
    test(harness, cx);
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn editor_text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

#[test]
fn backspace_deletes_the_character_before_the_caret() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("a b c");
        draw(cx);
        assert_eq!(editor_text(&editor, cx), "abc");

        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_eq!(
            editor_text(&editor, cx),
            "ab",
            "Backspace is a key binding, not a text-input edit"
        );
    });
}

#[test]
fn delete_key_and_home_are_bound() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("a b c");
        draw(cx);
        cx.simulate_keystrokes("home");
        draw(cx);
        cx.simulate_keystrokes("delete");
        draw(cx);
        assert_eq!(
            editor_text(&editor, cx),
            "bc",
            "Home then Delete must remove the first character"
        );
    });
}

#[test]
fn select_all_then_backspace_clears_the_document() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("a b c");
        draw(cx);
        cx.simulate_keystrokes("ctrl-a");
        draw(cx);
        cx.update(|_, cx| {
            assert!(
                !editor.read(cx).selection().is_empty(),
                "Ctrl+A must select the whole document"
            );
        });
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_eq!(editor_text(&editor, cx), "");
    });
}

#[test]
fn arrow_keys_move_the_caret() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("a b c home right right");
        draw(cx);
        cx.simulate_keystrokes("x");
        draw(cx);
        assert_eq!(
            editor_text(&editor, cx),
            "abxc",
            "Right must advance the caret before typing"
        );
    });
}

#[test]
fn ctrl_f_opens_the_find_bar_and_escape_closes_it() {
    run(|harness, cx| {
        let _ = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("ctrl-f");
        draw(cx);
        assert!(
            cx.debug_bounds("chaus-find-bar").is_some(),
            "Ctrl+F must dispatch the Find action"
        );
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(
            cx.debug_bounds("chaus-find-bar").is_none(),
            "Escape must close the find bar"
        );
    });
}
