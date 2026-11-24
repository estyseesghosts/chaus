//! Viewport intents interpreted against real editor geometry.
//!
//! `chaus-vim` only reports `zz`/`zt`/`zb`/Ctrl-D/Ctrl-U; the editor owns the
//! visual rows. These tests check the resulting scroll row and caret placement.

use chaus_core::position::{CharOffset, LineIndex, Selection};
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

fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

fn numbered_lines(count: usize) -> String {
    let mut text = String::new();
    for line in 0..count {
        text.push_str(&format!("line {line}\n"));
    }
    text
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
    let path = directory.path().join("viewport.txt");
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

fn goto_line(editor: &Entity<Editor>, cx: &mut VisualTestContext, line: usize) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let start = editor.document().line_start(LineIndex(line));
            editor.set_selection(Selection::caret(start), cx);
        });
    });
    draw(cx);
}

fn caret_row(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| editor.update(cx, |editor, _| editor.caret_visual_row()))
}

fn scroll_row(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> f32 {
    cx.update(|_, cx| editor.read(cx).viewport_scroll_row())
}

fn visible_rows(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .visible_row_count()
            .expect("the window has been painted")
    })
}

#[test]
fn zz_centers_the_caret_visual_row() {
    run(&numbered_lines(200), |editor, cx| {
        let visible = visible_rows(&editor, cx);
        assert!(visible > 4, "default test geometry: {visible} rows");
        goto_line(&editor, cx, 100);
        press(&editor, cx, "z");
        press(&editor, cx, "z");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 100);
        let expected = 100.0 - (visible.saturating_sub(1) as f32 / 2.0);
        assert!(
            (scroll_row(&editor, cx) - expected).abs() <= 0.51,
            "scroll {} should center row 100 with {visible} visible rows",
            scroll_row(&editor, cx)
        );
    });
}

#[test]
fn zt_puts_the_caret_row_at_the_top_and_zb_at_the_bottom() {
    run(&numbered_lines(200), |editor, cx| {
        let visible = visible_rows(&editor, cx);
        goto_line(&editor, cx, 100);
        press(&editor, cx, "z");
        press(&editor, cx, "t");
        draw(cx);
        assert_eq!(scroll_row(&editor, cx), 100.0);

        press(&editor, cx, "z");
        press(&editor, cx, "b");
        draw(cx);
        assert_eq!(
            scroll_row(&editor, cx),
            (101usize).saturating_sub(visible) as f32,
            "the caret's row must be the last visible row"
        );
        assert_eq!(caret_row(&editor, cx), 100);
    });
}

#[test]
fn ctrl_d_and_ctrl_u_move_the_caret_by_half_a_page() {
    run(&numbered_lines(200), |editor, cx| {
        let visible = visible_rows(&editor, cx);
        let half = (visible / 2).max(1);
        goto_line(&editor, cx, 100);
        cx.simulate_keystrokes("ctrl-d");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 100 + half);

        cx.simulate_keystrokes("ctrl-u");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 100);
    });
}

#[test]
fn half_page_counts_multiply_the_step() {
    run(&numbered_lines(200), |editor, cx| {
        let visible = visible_rows(&editor, cx);
        let half = (visible / 2).max(1);
        goto_line(&editor, cx, 20);
        press(&editor, cx, "3");
        cx.simulate_keystrokes("ctrl-d");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 20 + 3 * half);

        press(&editor, cx, "2");
        cx.simulate_keystrokes("ctrl-u");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 20 + half);
    });
}

#[test]
fn half_page_near_eof_clamps_the_caret_to_the_last_row() {
    run(&numbered_lines(40), |editor, cx| {
        goto_line(&editor, cx, 38);
        let total = cx.update(|_, cx| editor.read(cx).document().len_lines());
        cx.simulate_keystrokes("ctrl-d");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), total.saturating_sub(1));
    });
}

#[test]
fn ctrl_d_in_visual_mode_extends_the_selection_and_keeps_visual_mode() {
    run(&numbered_lines(200), |editor, cx| {
        let visible = visible_rows(&editor, cx);
        let half = (visible / 2).max(1);
        goto_line(&editor, cx, 100);
        press(&editor, cx, "v");
        cx.simulate_keystrokes("ctrl-d");
        draw(cx);

        assert_eq!(caret_row(&editor, cx), 100 + half);
        let anchor = cx.update(|_, cx| editor.read(cx).document().line_start(LineIndex(100)));
        let selection = cx.update(|_, cx| editor.read(cx).selection());
        assert_eq!(selection.anchor, anchor);
        assert!(selection.head > selection.anchor);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).vim_mode_label()),
            Some("VISUAL")
        );
    });
}

#[test]
fn a_caret_at_the_first_row_stays_there_for_ctrl_u() {
    run(&numbered_lines(60), |editor, cx| {
        goto_line(&editor, cx, 0);
        cx.simulate_keystrokes("ctrl-u");
        draw(cx);
        assert_eq!(caret_row(&editor, cx), 0);
        assert_eq!(scroll_row(&editor, cx), 0.0);
        let head = cx.update(|_, cx| editor.read(cx).selection().head);
        assert_eq!(head, CharOffset(0));
    });
}
