//! Vim operators and history go through the same mutation contract as typing.

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

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        draw(cx);
    }
}

fn run(source: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.rs");
    std::fs::write(&path, source).unwrap();
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
    settle(cx);
    test(editor, cx);
}

fn state(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> (String, bool, bool) {
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let current = editor.document().revision();
        (
            editor.document().text(),
            editor.document().is_dirty(),
            editor.syntax_revision() == Some(current),
        )
    })
}

#[test]
fn vim_undo_and_redo_schedule_recovery_and_a_fresh_parse() {
    run("fn main() {}\n", |editor, cx| {
        cx.simulate_keystrokes("x");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "n main() {}\n");
        assert!(
            dirty && parsed,
            "Vim edits must schedule recovery and parsing"
        );

        cx.simulate_keystrokes("u");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "fn main() {}\n", "Vim undo restores the saved text");
        assert!(!dirty, "undo to the saved baseline retires recovery");
        assert!(parsed, "undo must reparse the restored document");

        cx.simulate_keystrokes("ctrl-r");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "n main() {}\n", "Vim redo reapplies the edit");
        assert!(dirty, "redo marks the document dirty again");
        assert!(parsed, "redo must reparse the changed document");
    });
}

#[test]
fn vim_delete_and_paste_are_tracked_like_ordinary_edits() {
    run("one\ntwo\n", |editor, cx| {
        cx.simulate_keystrokes("d");
        cx.simulate_keystrokes("d");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "two\n", "dd deletes the current hard line");
        assert!(dirty && parsed);

        cx.simulate_keystrokes("p");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "two\none\n", "p puts the deleted line back");
        assert!(dirty && parsed);

        cx.simulate_keystrokes("u");
        settle(cx);
        let (text, _, parsed) = state(&editor, cx);
        assert_eq!(text, "two\n");
        assert!(parsed);
    });
}

#[test]
fn undo_and_redo_after_toggling_vim_still_run_the_contract() {
    run("fn main() {}\n", |editor, cx| {
        cx.simulate_keystrokes("x");
        settle(cx);
        cx.simulate_keystrokes("ctrl-shift-v");
        draw(cx);
        cx.simulate_keystrokes("ctrl-z");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "fn main() {}\n");
        assert!(!dirty, "undo after leaving Vim mode must retire recovery");
        assert!(parsed, "undo after a mode switch must reparse");

        cx.simulate_keystrokes("ctrl-shift-z");
        settle(cx);
        let (text, dirty, parsed) = state(&editor, cx);
        assert_eq!(text, "n main() {}\n");
        assert!(dirty);
        assert!(parsed);
    });
}
