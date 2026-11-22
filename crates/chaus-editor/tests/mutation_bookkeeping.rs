//! One mutation-completion contract for Vim, IME and history operations.

use chaus_core::position::CharOffset;
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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        draw(cx);
    }
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

fn fixture(source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("main.rs"), source).unwrap();
    directory
}

fn open(
    directory: &tempfile::TempDir,
    vim: bool,
    test: impl FnOnce(Entity<Editor>, &mut VisualTestContext),
) {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let loaded = chaus_io::load(&directory.path().join("main.rs")).unwrap();
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

#[test]
fn vim_x_refreshes_highlighting_and_marks_the_document_dirty() {
    let directory = fixture("fn main() {}\n");
    open(&directory, true, |editor, cx| {
        settle(cx);
        assert!(!cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()));
        let before = cx.update(|_, cx| editor.read(cx).document().revision());

        cx.simulate_keystrokes("x");
        let (text, revision, spans_empty) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.document().text(),
                editor.document().revision(),
                editor.syntax_spans().is_empty(),
            )
        });
        assert_eq!(text, "n main() {}\n", "x deletes one grapheme");
        assert_ne!(revision, before);
        assert!(
            spans_empty,
            "spans from the previous revision must not be painted"
        );

        settle(cx);
        let (spans_revision, current) = cx.update(|_, cx| {
            (
                editor.read(cx).syntax_revision(),
                editor.read(cx).document().revision(),
            )
        });
        assert_eq!(
            spans_revision,
            Some(current),
            "the Vim edit must schedule a new parse"
        );
        assert!(cx.update(|_, cx| editor.read(cx).document().is_dirty()));
    });
}

#[test]
fn undo_to_the_saved_baseline_is_clean_again() {
    let directory = fixture("fn main() {}\n");
    open(&directory, false, |editor, cx| {
        settle(cx);
        assert!(!cx.update(|_, cx| editor.read(cx).document().is_dirty()));

        cx.simulate_input("x");
        assert!(cx.update(|_, cx| editor.read(cx).document().is_dirty()));

        cx.simulate_keystrokes("ctrl-z");
        settle(cx);
        assert!(
            !cx.update(|_, cx| editor.read(cx).document().is_dirty()),
            "undo back to the saved text must retire dirty recovery state"
        );
        let (revision, spans_revision) = cx.update(|_, cx| {
            (
                editor.read(cx).document().revision(),
                editor.read(cx).syntax_revision(),
            )
        });
        assert_eq!(spans_revision, Some(revision));
    });
}

#[test]
fn vim_movement_does_not_run_mutation_bookkeeping() {
    let directory = fixture("fn main() {}\n");
    open(&directory, true, |editor, cx| {
        settle(cx);
        let before = cx.update(|_, cx| editor.read(cx).document().revision());
        cx.simulate_keystrokes("l");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).document().revision()),
            before,
            "pure motion must not produce a document revision"
        );
        assert!(!cx.update(|_, cx| editor.read(cx).document().is_dirty()));
    });
}

#[test]
fn preedit_updates_preserve_the_marked_range() {
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
    let marked = cx
        .update(|window, cx| editor.update(cx, |editor, cx| editor.marked_text_range(window, cx)));
    assert_eq!(text, "ab😀c");
    assert_eq!(
        marked,
        Some(0..5),
        "the marked range must survive after_change; UTF-16 length includes the surrogate pair"
    );
    assert_eq!(
        selection.range(),
        chaus_core::position::CharRange::new(CharOffset(1), CharOffset(3))
    );

    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "ab😀cd", Some(4..6), window, cx);
        });
    });
    let text = cx.update(|_, cx| editor.read(cx).document().text());
    let marked = cx
        .update(|window, cx| editor.update(cx, |editor, cx| editor.marked_text_range(window, cx)));
    assert_eq!(
        text, "ab😀cd",
        "the previous preedit is replaced, not appended"
    );
    assert_eq!(marked, Some(0..6));
    assert!(
        cx.update(|_, cx| editor.read(cx).document().is_dirty()),
        "committed composition text is part of the dirty document"
    );
}
