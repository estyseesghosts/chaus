//! Deferred startup loads install into an owned untitled window.

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

fn open() -> (Entity<Editor>, VisualTestContext) {
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
    cx.run_until_parked();
    (editor, cx.clone())
}

#[test]
fn adopting_a_deferred_load_installs_the_file_without_reloading() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deferred.txt");
    std::fs::write(&path, "hello\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (editor, mut cx) = open();

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.adopt_loaded(&path, loaded, cx));
    });
    let (text, title, dirty) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.title(),
            editor.document().is_dirty(),
        )
    });
    assert_eq!(text, "hello\n");
    assert_eq!(title, "deferred.txt");
    assert!(!dirty);
}

#[test]
fn adopting_a_load_never_replaces_unsaved_edits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deferred.txt");
    std::fs::write(&path, "hello\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (editor, mut cx) = open();
    cx.simulate_input("draft");

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.adopt_loaded(&path, loaded, cx));
    });
    let (text, title, status) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.title(),
            editor.status_text().map(str::to_owned),
        )
    });
    assert_eq!(text, "draft");
    assert_eq!(title, "Untitled — Edited");
    assert_eq!(
        status.as_deref(),
        Some("Open cancelled: the document has unsaved edits")
    );
}
