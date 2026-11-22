//! Wrapped selection rectangles and IME marked-text selection.

use chaus_core::position::{CharOffset, CharRange};
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

fn initialize(cx: &mut TestAppContext) {
    cx.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[test]
fn a_selection_spanning_wrapped_rows_returns_one_rectangle_per_row() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long.txt");
    std::fs::write(&path, "a".repeat(400)).unwrap();
    let loaded = chaus_io::load(&path).expect("load fixture");
    let mut settings = Settings::default();
    settings.wrap = true;

    let mut app = TestAppContext::single();
    initialize(&mut app);
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    let rectangles = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.selection_bounds_for_range(50..250, window.bounds(), window, cx)
        })
    });

    assert!(
        rectangles.len() >= 2,
        "a 100-character range must cover multiple wrapped rows: {rectangles:?}"
    );
    for pair in rectangles.windows(2) {
        assert!(
            pair[1].origin.y > pair[0].origin.y,
            "rows must advance vertically: {rectangles:?}"
        );
    }
    assert!(
        rectangles
            .iter()
            .all(|rectangle| rectangle.size.width > gpui::px(0.0)),
        "every row rectangle must have a positive width"
    );
}

#[test]
fn ime_marked_text_honors_the_native_selected_range() {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "ab😀c", Some(1..4), window, cx);
        });
    });
    let (text, selection) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.document().text(), editor.selection())
    });
    assert_eq!(text, "ab😀c");
    assert_eq!(
        selection.range(),
        CharRange::new(CharOffset(1), CharOffset(3)),
        "the selection is relative to the marked text and counts a surrogate pair correctly"
    );
}
