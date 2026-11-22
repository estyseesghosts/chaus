//! Selection-aware indent/outdent and whitespace transforms.

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

fn run(text: &str, settings: Settings, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("indent.txt");
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
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

fn undo_depth(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| editor.read(cx).document().undo_depth())
}

#[test]
fn a_multiline_selection_indents_every_line_as_one_undo_step() {
    run("one\ntwo\nthree", Settings::default(), |editor, cx| {
        cx.simulate_keystrokes("ctrl-a");
        let before = undo_depth(&editor, cx);
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.indent_selection(cx)));
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "    one\n    two\n    three",
            "every selected line should gain one tab stop"
        );
        assert_eq!(
            undo_depth(&editor, cx),
            before + 1,
            "the whole indent must be one undo entry"
        );
        cx.simulate_keystrokes("ctrl-z");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "one\ntwo\nthree",
            "one undo must reverse the whole indent"
        );
    });
}

#[test]
fn outdent_removes_one_level_from_every_selected_line() {
    run(
        "    one\n\ttwo\n  three\n",
        Settings::default(),
        |editor, cx| {
            cx.simulate_keystrokes("ctrl-a");
            cx.update(|_, cx| editor.update(cx, |editor, cx| editor.outdent_selection(cx)));
            draw(cx);
            assert_eq!(text(&editor, cx), "one\ntwo\nthree\n");
        },
    );
}

#[test]
fn tab_indents_a_multiline_selection_and_leaves_the_selection_intact() {
    run("one\ntwo\n", Settings::default(), |editor, cx| {
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_keystrokes("tab");
        draw(cx);
        assert_eq!(text(&editor, cx), "    one\n    two\n");
        let selection = cx.update(|_, cx| editor.read(cx).selection());
        assert!(
            !selection.is_empty(),
            "indenting a selection must keep it selected"
        );
    });
}

#[test]
fn trim_trailing_whitespace_removes_spaces_and_tabs_everywhere() {
    run(
        "one  \ntwo\t\nthree\n",
        Settings::default(),
        |editor, cx| {
            cx.update(|_, cx| editor.update(cx, |editor, cx| editor.trim_trailing_whitespace(cx)));
            draw(cx);
            assert_eq!(text(&editor, cx), "one\ntwo\nthree\n");
            cx.simulate_keystrokes("ctrl-z");
            draw(cx);
            assert_eq!(
                text(&editor, cx),
                "one  \ntwo\t\nthree\n",
                "trim must be a single undoable transaction"
            );
        },
    );
}

#[test]
fn indentation_converts_between_tabs_and_spaces_without_touching_text() {
    let mut settings = Settings::default();
    settings.expand_tabs = false;
    settings.tab_stop = 4;
    run("\tfoo\n    bar\n\t\tbaz\n", settings, |editor, cx| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.convert_indentation_to_spaces(cx));
        });
        draw(cx);
        assert_eq!(text(&editor, cx), "    foo\n    bar\n        baz\n");

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.convert_indentation_to_tabs(cx));
        });
        draw(cx);
        assert_eq!(text(&editor, cx), "\tfoo\n\tbar\n\t\tbaz\n");
    });
}

#[test]
fn a_reversed_selection_stays_reversed_through_indentation() {
    run("one\ntwo\n", Settings::default(), |editor, cx| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(
                    chaus_core::position::Selection::new(
                        chaus_core::position::CharOffset(7),
                        chaus_core::position::CharOffset(3),
                    ),
                    cx,
                );
            });
        });
        draw(cx);
        let before = cx.update(|_, cx| editor.read(cx).selection());
        assert!(before.reversed(), "the selection should be reversed");
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.indent_selection(cx)));
        draw(cx);
        let after = cx.update(|_, cx| editor.read(cx).selection());
        assert!(
            after.reversed(),
            "indent must preserve the selection direction"
        );
    });
}
