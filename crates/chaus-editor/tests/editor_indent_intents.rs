//! Vim indent commands routed through the editor's indentation engine.
//!
//! The engine returns `Outcome::Indent`; these tests verify that the editor
//! applies it to hard lines, keeps it undoable as one step and supports `.`.

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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// Deliver a Normal-mode key as Windows does: key-down, then WM_CHAR.
fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

fn run(text: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("indent.txt");
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn undo_depth(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| editor.read(cx).document().undo_depth())
}

fn mode(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<&'static str> {
    cx.update(|_, cx| editor.read(cx).vim_mode_label())
}

#[test]
fn shift_right_and_left_apply_to_the_current_line() {
    run("one\ntwo\n", |editor, cx| {
        press(&editor, cx, ">");
        press(&editor, cx, ">");
        draw(cx);
        assert_eq!(text(&editor, cx), "    one\ntwo\n");
        assert_eq!(mode(&editor, cx), Some("NORMAL"));

        press(&editor, cx, "<");
        press(&editor, cx, "<");
        draw(cx);
        assert_eq!(text(&editor, cx), "one\ntwo\n");
    });
}

#[test]
fn a_count_indents_that_many_hard_lines_as_one_undo_step() {
    run("one\ntwo\nthree\nfour\n", |editor, cx| {
        let before = undo_depth(&editor, cx);
        press(&editor, cx, "3");
        press(&editor, cx, ">");
        press(&editor, cx, ">");
        draw(cx);
        assert_eq!(text(&editor, cx), "    one\n    two\n    three\nfour\n");
        assert_eq!(
            undo_depth(&editor, cx),
            before + 1,
            "the whole range must be one user-visible undo step"
        );

        cx.simulate_keystrokes("u");
        draw(cx);
        assert_eq!(text(&editor, cx), "one\ntwo\nthree\nfour\n");
    });
}

#[test]
fn dot_repeats_the_indent_on_the_new_current_line() {
    run("a\nb\nc\n", |editor, cx| {
        press(&editor, cx, ">");
        press(&editor, cx, ">");
        press(&editor, cx, "j");
        press(&editor, cx, ".");
        draw(cx);
        assert_eq!(text(&editor, cx), "    a\n    b\nc\n");
    });
}

#[test]
fn outdent_stops_at_the_start_of_the_line() {
    run("    one\n\ttwo\nthree\n", |editor, cx| {
        press(&editor, cx, "3");
        press(&editor, cx, "<");
        press(&editor, cx, "<");
        draw(cx);
        assert_eq!(text(&editor, cx), "one\ntwo\nthree\n");
    });
}

#[test]
fn visual_line_shift_indents_every_highlighted_line() {
    run("one\ntwo\nthree\n", |editor, cx| {
        press(&editor, cx, "V");
        press(&editor, cx, "j");
        press(&editor, cx, ">");
        draw(cx);
        assert_eq!(text(&editor, cx), "    one\n    two\nthree\n");
        assert_eq!(
            mode(&editor, cx),
            Some("NORMAL"),
            "a visual shift finishes the Visual operation"
        );
        cx.simulate_keystrokes("u");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "one\ntwo\nthree\n",
            "both lines undo together"
        );
    });
}
