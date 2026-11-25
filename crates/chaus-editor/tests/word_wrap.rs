//! Word-preferred wrapping through the real editor: rows, selection and `:set`.

use chaus_core::position::{CharOffset, LineIndex};
use chaus_core::settings::{Settings, WrapMode};
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

const PARAGRAPH: &str = "Chaus is a small plaintext scratchpad that opens quickly, keeps the \
document intact, and wraps long paragraphs at natural word boundaries rather than cutting \
ordinary words in half. The editor remains a plaintext editor: wrapping is visual only and \
never edits the Rope, never inserts newlines, and never changes saved bytes.";

/// Long enough to wrap over several rows at the test window's default width.
fn long_text() -> String {
    format!("{PARAGRAPH} {PARAGRAPH} {PARAGRAPH}")
}

fn run_text(text: &str, vim: bool, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
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
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = vim;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn segments(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    line: usize,
) -> Vec<chaus_layout::Segment> {
    cx.update(|_, cx| editor.read(cx).visual_segments(LineIndex(line)))
}

fn line_text(editor: &Entity<Editor>, cx: &mut VisualTestContext, line: usize) -> String {
    cx.update(|_, cx| editor.read(cx).document().line_text(LineIndex(line)))
}

#[test]
fn word_mode_breaks_after_whitespace_and_keeps_every_character() {
    run_text(&long_text(), false, |editor, cx| {
        let rows = segments(&editor, cx, 0);
        assert!(rows.len() >= 3, "the paragraph must wrap over several rows");
        assert_eq!(rows[0].char_start, 0);
        assert_eq!(rows.last().unwrap().char_end, long_text().chars().count());
        for pair in rows.windows(2) {
            assert_eq!(pair[0].char_end, pair[1].char_start);
        }
        let source: String = line_text(&editor, cx, 0);
        for row in &rows {
            let start: usize = source
                .chars()
                .take(row.char_start.get())
                .map(char::len_utf8)
                .sum();
            let end: usize = source
                .chars()
                .take(row.char_end.get())
                .map(char::len_utf8)
                .sum();
            if !source[start..end].contains('\t') {
                assert_eq!(row.display, &source[start..end]);
            }
        }
        for row in &rows[..rows.len() - 1] {
            let before: Vec<char> = source.chars().collect();
            assert!(
                before[row.char_end - 1].is_whitespace(),
                "row must end at a word break, not inside a word: {:?}",
                row
            );
        }
        let line_count = cx.update(|_, cx| editor.read(cx).document().len_lines());
        let total: usize = (0..line_count)
            .map(|line| segments(&editor, cx, line).len())
            .sum();
        assert_eq!(
            total,
            cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count())),
            "the cached row total must match the segments used for painting"
        );
    });
}

#[test]
fn character_mode_breaks_inside_words_and_differs_from_word_mode() {
    run_text(&long_text(), false, |editor, cx| {
        let word_rows = segments(&editor, cx, 0);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_wrap_mode(WrapMode::Character, cx);
            });
        });
        draw(cx);
        let char_rows = segments(&editor, cx, 0);
        assert_ne!(word_rows, char_rows);
        let source: Vec<char> = line_text(&editor, cx, 0).chars().collect();
        assert!(
            char_rows[..char_rows.len() - 1]
                .iter()
                .any(|row| !source[row.char_end - 1].is_whitespace()),
            "character wrapping should cut at a column, not only at spaces"
        );
        let rows = cx.update(|_, cx| editor.read(cx).visual_segments(LineIndex(0)).len());
        assert_eq!(rows, char_rows.len());
    });
}

#[test]
fn no_wrap_returns_one_row_per_hard_line() {
    run_text(
        &format!("{PARAGRAPH}\nsecond line\n"),
        false,
        |editor, cx| {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| editor.set_wrap_mode(WrapMode::None, cx));
            });
            draw(cx);
            for line in 0..2 {
                let rows = segments(&editor, cx, line);
                assert_eq!(rows.len(), 1, "nowrap must not split hard line {line}");
                assert_eq!(rows[0].char_start, 0);
                assert_eq!(
                    rows[0].char_end,
                    line_text(&editor, cx, line).chars().count()
                );
            }
            assert_eq!(
                cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count())),
                3,
                "two hard lines plus the empty line after the final newline"
            );
        },
    );
}

#[test]
fn changing_wrap_modes_never_edits_text_or_history() {
    run_text(&long_text(), false, |editor, cx| {
        let (before, dirty, can_undo) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.document().text(),
                editor.document().is_dirty(),
                editor.document().can_undo(),
            )
        });
        for mode in [WrapMode::Character, WrapMode::None, WrapMode::Word] {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| editor.set_wrap_mode(mode, cx));
            });
            draw(cx);
        }
        let (after, dirty_after, can_undo_after) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.document().text(),
                editor.document().is_dirty(),
                editor.document().can_undo(),
            )
        });
        assert_eq!(after, before);
        assert_eq!(dirty_after, dirty);
        assert_eq!(can_undo_after, can_undo);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Word
        );
    });
}

#[test]
fn continuation_rows_start_at_the_normal_left_inset() {
    run_text(&long_text(), false, |editor, cx| {
        let rows = segments(&editor, cx, 0);
        assert!(rows.len() >= 2);
        let first = rows[0].char_start.get();
        let next = rows[1].char_start.get();
        let (first_rect, next_rect) = cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let bounds = window.bounds();
                let first = editor.selection_bounds_for_range(first..first + 1, bounds, window, cx);
                let next = editor.selection_bounds_for_range(next..next + 1, bounds, window, cx);
                (first, next)
            })
        });
        let first = first_rect.first().expect("first row has a rectangle");
        let next = next_rect.first().expect("second row has a rectangle");
        assert!(next.origin.y > first.origin.y, "rows advance downward");
        assert_eq!(
            next.origin.x, first.origin.x,
            "a continuation row starts at the document's left inset, not under the word"
        );
    });
}

#[test]
fn a_selection_across_word_rows_returns_one_positive_rectangle_per_row() {
    run_text(&long_text(), false, |editor, cx| {
        let rows = segments(&editor, cx, 0);
        assert!(rows.len() >= 3);
        let start = CharOffset(rows[0].char_start.get());
        let end = CharOffset(rows[2].char_end.get());
        let rectangles = cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.selection_bounds_for_range(start.0..end.0, window.bounds(), window, cx)
            })
        });
        assert!(
            rectangles.len() >= 3,
            "each covered visual row gets a rectangle"
        );
        for pair in rectangles.windows(2) {
            assert!(pair[1].origin.y > pair[0].origin.y);
        }
        assert!(rectangles
            .iter()
            .all(|rect| rect.size.width > gpui::px(0.0)));
    });
}

fn invoke(editor: &Entity<Editor>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        let commands = editor.read(cx).commands().clone();
        silvestris::command::CommandRegistry::invoke(
            &commands,
            &silvestris::core::command::CommandId::new(id),
            window,
            cx,
        );
    });
}

#[test]
fn the_view_wrap_radios_route_to_the_focused_window() {
    run_text(&long_text(), false, |editor, cx| {
        invoke(&editor, cx, "view.wrap_char");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Character
        );

        invoke(&editor, cx, "view.wrap_none");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::None
        );

        invoke(&editor, cx, "view.wrap_word");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Word
        );
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

fn ex_command(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    press(editor, cx, ":");
    for character in command.chars() {
        let key = if character == ' ' {
            "space".to_string()
        } else {
            character.to_string()
        };
        press(editor, cx, &key);
    }
    press(editor, cx, "enter");
}

#[test]
fn the_runtime_set_command_switches_wrap_modes() {
    run_text(&long_text(), true, |editor, cx| {
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Word
        );

        ex_command(&editor, cx, "set wrap=char");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Character
        );

        ex_command(&editor, cx, "set nowrap");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::None
        );

        ex_command(&editor, cx, "set wrap");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).wrap_mode()),
            WrapMode::Word
        );
    });
}
