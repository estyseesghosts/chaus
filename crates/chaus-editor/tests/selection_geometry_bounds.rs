//! Native selection queries stay bounded for very large documents.

use std::time::{Duration, Instant};

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

#[test]
fn select_all_on_a_hundred_thousand_lines_returns_only_visible_rectangles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big.txt");
    let source = "short line\n".repeat(100_000);
    std::fs::write(&path, &source).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let utf16_len = source.encode_utf16().count();

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut *cx);

    let started = Instant::now();
    let rectangles = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.selection_bounds_for_range(0..utf16_len, window.bounds(), window, cx)
        })
    });
    let elapsed = started.elapsed();
    assert!(
        rectangles.len() <= 80,
        "select all must not produce a rectangle per hard line: {} rectangles",
        rectangles.len()
    );
    assert!(
        rectangles.len() < 1_000,
        "100k hard lines must yield a viewport-sized result: {} rectangles",
        rectangles.len()
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "selection geometry must be bounded: {elapsed:?}"
    );
    assert!(
        rectangles
            .iter()
            .all(|rectangle| rectangle.size.width > gpui::px(0.0)),
        "visible row rectangles must have positive width"
    );
}
