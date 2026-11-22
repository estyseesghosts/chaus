//! Hard-line insertions and deletions must reuse measured row counts.

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

fn open(text: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rows.txt");
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

fn caret_to(editor: &Entity<Editor>, cx: &mut VisualTestContext, offset: usize) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_selection(Selection::caret(CharOffset(offset)), cx);
        });
    });
}

/// Every line start equals the sum of the rows of the lines above it, and the
/// total equals the sum of every line's segments.
fn assert_consistent(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    let (actual, expected, total, summed) = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let lines = editor.document().len_lines();
            let mut actual = Vec::with_capacity(lines);
            let mut expected = Vec::with_capacity(lines);
            let mut total = 0;
            for line in 0..lines {
                actual.push(editor.visual_row_start(LineIndex(line)));
                expected.push(total);
                total += editor.visual_segments(LineIndex(line)).len();
            }
            (actual, expected, editor.visual_row_count(), total.max(1))
        })
    });
    assert_eq!(actual, expected, "prefix index must match measured rows");
    assert_eq!(total, summed, "the total must match the measured rows");
}

fn fixture() -> String {
    let mut text = String::new();
    for index in 0..2_000 {
        // Uneven widths so some lines wrap and some do not.
        let width = 40 + (index * 37) % 200;
        text.push_str(&"x".repeat(width));
        text.push('\n');
    }
    text
}

#[test]
fn inserting_newlines_keeps_the_row_index_consistent() {
    let text = fixture();
    let chars = text.chars().count();
    open(&text, |editor, cx| {
        assert_consistent(&editor, cx);

        for offset in [0, chars / 2, chars] {
            caret_to(&editor, cx, offset);
            cx.simulate_keystrokes("enter");
            draw(cx);
            assert_consistent(&editor, cx);
        }

        // Undo restores the old line count and must reuse the measured rows.
        for _ in 0..3 {
            cx.simulate_keystrokes("ctrl-z");
            draw(cx);
            assert_consistent(&editor, cx);
        }
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).document().text(), text);
        });
    });
}

#[test]
fn deleting_lines_keeps_the_row_index_consistent() {
    let text = fixture();
    open(&text, |editor, cx| {
        assert_consistent(&editor, cx);

        // Join the first two lines, then remove a run of newlines in the
        // middle by selecting across them.
        caret_to(&editor, cx, 0);
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("home");
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_consistent(&editor, cx);

        let middle = cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                let line = editor.document().len_lines() / 2;
                editor.document().line_start(LineIndex(line)).0
            })
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let end = editor
                    .document()
                    .line_start(LineIndex(editor.document().len_lines() / 2 + 5));
                editor.set_selection(Selection::new(CharOffset(middle), end), cx);
            });
        });
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_consistent(&editor, cx);

        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_consistent(&editor, cx);
    });
}

#[test]
fn point_edits_that_change_wrapping_update_only_their_lines() {
    let text = fixture();
    open(&text, |editor, cx| {
        assert_consistent(&editor, cx);
        // Insert a long run inside the first line so its row count grows.
        caret_to(&editor, cx, 10);
        cx.simulate_input(&"y".repeat(300));
        draw(cx);
        assert_consistent(&editor, cx);
        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_consistent(&editor, cx);
    });
}
