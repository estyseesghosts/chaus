//! Explicit format application: revision guard and one-undo replacement.

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

fn run(path_name: &str, text: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(path_name);
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

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

#[test]
fn formatting_an_untitled_document_explains_what_is_required() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    cx.update(|_, cx| editor.update(cx, |editor, cx| editor.format_document(cx)));
    draw(cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned)),
        Some("Save the file before formatting".to_owned())
    );
}

#[test]
fn an_unknown_language_reports_that_no_formatter_is_configured() {
    run("notes.txt", "one\n", |editor, cx| {
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.format_document(cx)));
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned)),
            Some("No formatter is configured for this file type".to_owned())
        );
        assert_eq!(text(&editor, cx), "one\n");
    });
}

#[test]
fn formatter_output_replaces_the_document_as_one_undo_step() {
    run("code.rs", "one\ntwo\n", |editor, cx| {
        let revision = cx.update(|_, cx| editor.read(cx).document().revision());
        let depth = cx.update(|_, cx| editor.read(cx).document().undo_depth());
        let applied = cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(
                    chaus_core::position::Selection::caret(chaus_core::position::CharOffset(2)),
                    cx,
                );
                editor.apply_formatted_text(revision, "one\n  two\n", cx)
            })
        });
        assert!(applied, "fresh formatter output must apply");
        draw(cx);
        assert_eq!(text(&editor, cx), "one\n  two\n");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).document().undo_depth()),
            depth + 1,
            "format must be a single undo entry"
        );
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).selection()),
            chaus_core::position::Selection::caret(chaus_core::position::CharOffset(2)),
            "the caret should survive a whole-document replacement"
        );
        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_eq!(text(&editor, cx), "one\ntwo\n");
    });
}

#[test]
fn stale_formatter_output_is_discarded_without_touching_the_document() {
    run("code.rs", "one\ntwo\n", |editor, cx| {
        let revision = cx.update(|_, cx| editor.read(cx).document().revision());
        // A change after the formatter started must invalidate its result.
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.indent_selection(cx);
            });
        });
        draw(cx);
        let edited = text(&editor, cx);
        let applied = cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.apply_formatted_text(revision, "formatted\n", cx)
            })
        });
        assert!(!applied, "stale output must be refused");
        assert_eq!(text(&editor, cx), edited);
        assert!(cx.update(|_, cx| {
            editor
                .read(cx)
                .status_text()
                .is_some_and(|status| status.contains("discarded"))
        }));
    });
}

#[test]
fn already_formatted_text_is_a_no_op() {
    run("code.rs", "one\n", |editor, cx| {
        let revision = cx.update(|_, cx| editor.read(cx).document().revision());
        let applied = cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.apply_formatted_text(revision, "one\n", cx)
            })
        });
        assert!(!applied);
        assert_eq!(text(&editor, cx), "one\n");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).document().undo_depth()),
            0,
            "a no-op must not create history"
        );
    });
}
