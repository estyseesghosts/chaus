//! Text-preserving repaints must reuse shaped rows.

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

fn calls(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> u64 {
    cx.update(|_, cx| editor.read(cx).shaped_row_calls())
}

fn open(test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wrap.txt");
    let mut text = String::new();
    for index in 0..400 {
        text.push_str(&format!("line {index} {}", "word ".repeat(6 + index % 25)));
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

#[test]
fn caret_blinks_and_status_repaints_reuse_shaped_rows() {
    open(|editor, cx| {
        let warm = calls(&editor, cx);
        assert!(warm > 0, "the first frame must shape its visible rows");

        // Repaints with no text, highlight, font, theme or width change stand
        // in for a caret blink or a transient status message.
        for _ in 0..3 {
            draw(cx);
        }
        assert_eq!(
            calls(&editor, cx),
            warm,
            "a text-preserving repaint must not shape rows again"
        );
    });
}

#[test]
fn editing_reshapes_rows_and_undo_does_too() {
    open(|editor, cx| {
        let warm = calls(&editor, cx);
        cx.simulate_input("x");
        draw(cx);
        let after_edit = calls(&editor, cx);
        assert!(
            after_edit > warm,
            "a text change must invalidate shaped rows"
        );

        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert!(
            calls(&editor, cx) > after_edit,
            "undo changes the revision and must reshape"
        );
    });
}

#[test]
fn an_unchanged_text_survives_many_repaints_without_growth() {
    open(|editor, cx| {
        let warm = calls(&editor, cx);
        for _ in 0..20 {
            draw(cx);
        }
        assert_eq!(calls(&editor, cx), warm);
    });
}
