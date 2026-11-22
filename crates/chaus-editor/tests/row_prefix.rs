//! The incremental row prefix must match the summed visual rows per line.

use chaus_core::position::LineIndex;
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

fn assert_prefix_matches_rows(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    let (starts, expected) = cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let lines = editor.document().len_lines();
            let mut starts = Vec::with_capacity(lines);
            let mut expected = Vec::with_capacity(lines);
            let mut total = 0;
            for line in 0..lines {
                starts.push(editor.visual_row_start(LineIndex(line)));
                expected.push(total);
                total += editor.visual_segments(LineIndex(line)).len();
            }
            (starts, expected)
        })
    });
    assert_eq!(
        starts, expected,
        "each prefix entry must be the sum of the preceding lines' visual rows"
    );
}

fn open(text: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prefix.txt");
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
fn multi_line_replacement_keeps_intermediate_prefixes_correct() {
    let line = "a".repeat(200);
    let original = std::iter::repeat_n(line.as_str(), 40)
        .collect::<Vec<_>>()
        .join("\n");
    open(&original, |editor, cx| {
        assert_prefix_matches_rows(&editor, cx);

        // The first line shrinks sharply and the second grows sharply, so the
        // combined delta is not representative of either intermediate entry.
        let replacement = format!(
            "{}\n{}\n{}",
            "b".repeat(30),
            "c".repeat(600),
            "d".repeat(120)
        );
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(&replacement);
        draw(cx);
        assert_prefix_matches_rows(&editor, cx);

        let total = cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count()));
        let summed = cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                (0..editor.document().len_lines())
                    .map(|line| editor.visual_segments(LineIndex(line)).len())
                    .sum::<usize>()
                    .max(1)
            })
        });
        assert_eq!(total, summed);

        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_prefix_matches_rows(&editor, cx);
    });
}
