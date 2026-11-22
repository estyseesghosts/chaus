//! Up/Down must preserve x within the visual row, not within the hard line.

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
    let path = directory.path().join("vertical.txt");
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

/// Row index and x (in display columns) of the caret within its visual row.
fn row_and_x(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> (usize, usize) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            let head = editor.selection().head;
            let line = editor.document().line_of(head);
            let line_start = editor.document().line_start(line).0;
            let text = editor.document().line_text(line);
            let segments = editor.visual_segments(line);
            let local = head.0 - line_start;
            let row = segments
                .iter()
                .position(|segment| local >= segment.char_start && local <= segment.char_end)
                .unwrap_or(segments.len().saturating_sub(1));
            let column = chaus_layout::column_at(&text, local, editor.settings().tab_stop);
            (row, column.saturating_sub(segments[row].start_column))
        })
    })
}

#[test]
fn repeated_up_and_down_stay_on_successive_visual_rows_at_constant_x() {
    let paragraph = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu \
                     nu xi omicron pi rho sigma tau upsilon phi chi psi omega ";
    let text = paragraph.repeat(30);
    open(&text, |editor, cx| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(CharOffset(6)), cx);
            });
        });
        draw(cx);
        let (start_row, start_x) = row_and_x(&editor, cx);
        assert_eq!(start_row, 0);

        let mut rows = vec![start_row];
        let mut xs = vec![start_x];
        for _ in 0..6 {
            cx.simulate_keystrokes("down");
            let (row, x) = row_and_x(&editor, cx);
            rows.push(row);
            xs.push(x);
        }
        assert!(
            rows.windows(2).all(|pair| pair[1] == pair[0] + 1),
            "each Down must advance one visual row: {rows:?}"
        );
        assert!(
            xs.iter().all(|x| *x <= start_x),
            "x must stay at the preferred column when the row is wide enough: {xs:?}"
        );
        assert!(
            xs[..xs.len() - 1].iter().all(|x| *x == start_x),
            "wide rows must all keep the same x: {xs:?}"
        );

        for _ in 0..6 {
            cx.simulate_keystrokes("up");
        }
        let caret = cx.update(|_, cx| editor.read(cx).selection().head);
        assert_eq!(
            caret,
            CharOffset(6),
            "an equal number of Ups must return to the original caret"
        );
    });
}

#[test]
fn vertical_motion_into_a_short_final_row_clamps_to_its_end() {
    let text = format!("{}\nshort", "x".repeat(800));
    open(&text, |editor, cx| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(CharOffset(700)), cx);
            });
        });
        draw(cx);
        let (row, _) = row_and_x(&editor, cx);
        assert!(row > 0, "the long line must wrap before its end");
        cx.simulate_keystrokes("down");
        let line = cx.update(|_, cx| {
            let editor = editor.read(cx);
            editor.document().line_of(editor.selection().head)
        });
        assert_eq!(line, LineIndex(1));
    });
}
