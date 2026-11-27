//! Pointer hit testing on wrapped rows, horizontal bounds and drag autoscroll.

use chaus_core::position::LineIndex;
use chaus_core::settings::{Settings, WrapMode};
use chaus_editor::Editor;
use gpui::{
    point, px, AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers,
    MouseButton, Render, TestAppContext, VisualTestContext, Window,
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

fn open(text: &str, wrap: WrapMode) -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("text.txt");
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.wrap = wrap;
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
fn clicking_right_of_a_wrapped_row_clamps_to_that_row() {
    let text = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz\n";
    let (editor, mut cx) = open(text, WrapMode::Character);
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.lock_wrap_width_for_test();
            editor.set_wrap_columns(Some(10), cx);
        });
    });
    draw(&mut cx);
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).committed_wrap_columns(), Some(10));
        let geometry = editor
            .read(cx)
            .geometry_for_test()
            .expect("painted geometry");
        let (line, span) = editor.update(cx, |editor, _| editor.row_span_for_test(0));
        assert!(span.end > span.start, "the first row must cover characters");
        let expected = editor.update(cx, |editor, _| {
            editor.offset_at_for_test(line, span.end.get())
        });
        let click = point(
            geometry.bounds.right() - px(2.0),
            geometry.bounds.top() + geometry.padding_for_test() + geometry.line_height / 2.0,
        );
        let actual = editor
            .update(cx, |editor, _| editor.offset_for_point_for_test(click))
            .expect("a click inside the viewport resolves");
        assert_eq!(
            actual, expected,
            "blank space past a wrapped row must not resolve into a later visual row"
        );
        assert_eq!(line, LineIndex(0));
    });
}

#[test]
fn horizontal_scroll_stops_at_the_widest_visible_row() {
    let text = format!("{}\n", "x".repeat(400));
    let (editor, mut cx) = open(&text, WrapMode::None);
    cx.update(|_, cx| {
        let geometry = editor
            .read(cx)
            .geometry_for_test()
            .expect("painted geometry");
        let char_width = f32::from(geometry.char_width);
        let viewport =
            f32::from(geometry.bounds.size.width) - 2.0 * f32::from(geometry.padding_for_test());
        let expected = (400.0 * char_width - viewport).max(0.0);
        assert!(
            expected > 0.0,
            "the test line must be wider than the viewport"
        );

        editor.update(cx, |editor, _| {
            editor.set_scroll_x_for_test(px(100_000.0));
            editor.clamp_scroll_for_test();
        });
        let actual = f32::from(editor.read(cx).scroll_x_for_test());
        assert!(
            (actual - expected).abs() < 1.0,
            "horizontal scroll must clamp to the row width: expected {expected}, got {actual}"
        );
    });
}

#[test]
fn holding_the_pointer_outside_the_viewport_keeps_scrolling() {
    let text: String = (0..500).map(|line| format!("line {line}\n")).collect();
    let (editor, mut cx) = open(&text, WrapMode::Word);
    let geometry = cx.update(|_, cx| editor.read(cx).geometry_for_test().unwrap());
    let start = point(
        geometry.bounds.left() + px(20.0),
        geometry.bounds.top() + px(20.0),
    );
    let outside = point(
        geometry.bounds.left() + px(20.0),
        geometry.bounds.bottom() - px(2.0),
    );

    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    draw(&mut cx);
    cx.update(|_, cx| assert!(editor.read(cx).selecting_for_test()));
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    draw(&mut cx);
    let after_move = cx.update(|_, cx| editor.read(cx).scroll_row_for_test());
    cx.update(|_, cx| {
        assert!(
            editor.read(cx).drag_autoscroll_active_for_test(),
            "a drag outside the viewport must start autoscroll"
        );
    });

    // No further mouse events: the timer alone must keep moving the viewport.
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(60));
        draw(&mut cx);
    }
    let after_hold = cx.update(|_, cx| editor.read(cx).scroll_row_for_test());
    assert!(
        after_hold > after_move,
        "holding the pointer below the viewport must continue scrolling: {after_move} -> {after_hold}"
    );

    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    draw(&mut cx);
    let released = cx.update(|_, cx| editor.read(cx).scroll_row_for_test());
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(60));
        draw(&mut cx);
    }
    let after_release = cx.update(|_, cx| editor.read(cx).scroll_row_for_test());
    assert_eq!(
        released, after_release,
        "releasing the mouse must stop autoscroll"
    );
}
