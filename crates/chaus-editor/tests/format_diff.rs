//! Formatter output is applied as one bounded edit, not a whole-document
//! replacement.

use chaus_core::position::{CharOffset, Selection};
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

fn with_open(path: &std::path::Path, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let loaded = chaus_io::load(path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded.clone()), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn apply(editor: &Entity<Editor>, cx: &mut VisualTestContext, formatted: &str) -> bool {
    let revision = cx.update(|_, cx| editor.read(cx).document().revision());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.apply_formatted_text(revision, formatted, cx)
        })
    })
}

#[test]
fn a_one_line_change_in_a_large_document_round_trips_through_undo() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.txt");
    let mut original = String::new();
    for index in 0..100_000 {
        original.push_str(&format!("line {index}\n"));
    }
    std::fs::write(&path, &original).unwrap();

    with_open(&path, |editor, cx| {
        let formatted = original.replace("line 50000\n", "line 50000, formatted\n");
        let end = CharOffset(original.chars().count());
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(end), cx);
            });
        });
        assert!(
            apply(&editor, cx, &formatted),
            "a real change must be applied"
        );
        draw(cx);
        assert_eq!(text(&editor, cx), formatted);

        // One logical operation: a single undo restores the whole document.
        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_eq!(text(&editor, cx), original);
    });
}

#[test]
fn a_noop_formatter_result_leaves_history_alone() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("same.txt");
    std::fs::write(&path, "same text\n").unwrap();

    with_open(&path, |editor, cx| {
        assert!(!apply(&editor, cx, "same text\n"));
        draw(cx);
        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_eq!(text(&editor, cx), "same text\n");
    });
}

#[test]
fn a_stale_formatter_result_is_discarded() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stale.txt");
    std::fs::write(&path, "one\n").unwrap();

    with_open(&path, |editor, cx| {
        let stale = cx.update(|_, cx| editor.read(cx).document().revision());
        cx.simulate_input("typed");
        draw(cx);

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert!(
                    !editor.apply_formatted_text(stale, "one\n", cx),
                    "a result for an older revision must not be applied"
                );
                assert_eq!(editor.document().text(), "typedone\n");
            });
        });
        draw(cx);
    });
}

#[test]
fn the_caret_moves_with_the_changed_text() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("caret.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();

    with_open(&path, |editor, cx| {
        // The caret sits after the edited line, on "three".
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(Selection::caret(CharOffset(12)), cx);
            });
        });
        assert!(apply(&editor, cx, "one\nTWO\nthree\n"));
        draw(cx);

        let caret = cx.update(|_, cx| editor.read(cx).selection().head);
        assert_eq!(
            caret,
            CharOffset(12),
            "an equal-length replacement keeps the caret in place"
        );
        assert_eq!(text(&editor, cx), "one\nTWO\nthree\n");
    });
}

#[test]
fn combining_marks_stay_whole_when_the_formatter_removes_one() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unicode.txt");
    std::fs::write(&path, "ae\u{301}b\n").unwrap();

    with_open(&path, |editor, cx| {
        assert!(apply(&editor, cx, "aeb\n"));
        draw(cx);
        assert_eq!(text(&editor, cx), "aeb\n");

        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_eq!(text(&editor, cx), "ae\u{301}b\n");
    });
}
