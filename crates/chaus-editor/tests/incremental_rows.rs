//! Incremental row-cache consistency after edits.
//!
//! The editor updates only the hard lines reported by `Document` instead of
//! rebuilding every line. This test proves the incrementally maintained prefix
//! agrees with a fresh rebuild of the same final text at the same wrap width.

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

fn initialize(app: &mut TestAppContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
}

fn editor_rows(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count()))
}

#[test]
fn incremental_rows_match_a_fresh_rebuild_after_an_edit() {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rows.txt");
    let long_line = "a".repeat(200);
    let contents = std::iter::repeat_n(long_line.as_str(), 40)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, &contents).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, mut cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut cx);
    let rows_before = editor_rows(&editor, &mut cx);

    // Append enough text to the first line to add several wrapped rows.
    cx.simulate_keystrokes("shift-a");
    cx.simulate_input(&"b".repeat(500));
    draw(&mut cx);

    let rows_after = editor_rows(&editor, &mut cx);
    assert!(
        rows_after > rows_before,
        "the edit must actually change the wrapped row total ({rows_before} -> {rows_after})"
    );

    // A second window opened from the same final text must wrap to the same
    // number of rows if the incremental prefix stayed consistent.
    let final_text = cx.update(|_, cx| editor.read(cx).document().text());
    let fresh_path = directory.path().join("fresh.txt");
    std::fs::write(&fresh_path, &final_text).unwrap();
    let fresh_loaded = chaus_io::load(&fresh_path).unwrap();
    let (fresh_harness, mut fresh_cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(fresh_loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let fresh = fresh_cx.update(|_, app| fresh_harness.read(app).editor.clone());
    draw(&mut fresh_cx);
    let fresh_rows = editor_rows(&fresh, &mut fresh_cx);

    assert_eq!(
        rows_after, fresh_rows,
        "incremental rows must agree with a fresh rebuild"
    );
}
