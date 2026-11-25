//! Cheap preferences must not schedule layout work; layout-affecting ones go
//! through the generation-checked background reflow.

use chaus_core::settings::{Settings, WrapMode};
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

fn settle(cx: &VisualTestContext) {
    for _ in 0..6 {
        cx.run_until_parked();
    }
}

fn open(source: &str) -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.txt");
    std::fs::write(&path, source).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_wrap_columns(Some(24), cx));
    });
    draw(&mut *cx);
    (editor, cx.clone())
}

fn apply(cx: &mut VisualTestContext, editor: &Entity<Editor>, change: impl FnOnce(&mut Settings)) {
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            let mut settings = editor.settings().clone();
            change(&mut settings);
            editor.set_settings(settings, cx);
        });
    });
}

fn snapshot(cx: &mut VisualTestContext, editor: &Entity<Editor>) -> (u64, u64, usize) {
    cx.update(|_, cx| {
        let generation = editor.read(cx).reflow_generation();
        let publications = editor.read(cx).reflow_publications();
        let rows = editor.update(cx, |editor, _| editor.visual_row_count());
        (generation, publications, rows)
    })
}

#[test]
fn cheap_preferences_do_not_touch_layout() {
    let source = "one two three four five six seven eight\n".repeat(40);
    let (editor, mut cx) = open(&source);
    let before = snapshot(&mut cx, &editor);

    apply(&mut cx, &editor, |settings| settings.expand_tabs = true);
    apply(&mut cx, &editor, |settings| settings.vim = true);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    settle(&cx);

    let after = snapshot(&mut cx, &editor);
    assert_eq!(after.0, before.0, "indent/Vim defaults must not reflow");
    assert_eq!(after.1, before.1);
    assert_eq!(after.2, before.2);
}

#[test]
fn line_height_only_updates_geometry_without_rewrapping() {
    let source = "one two three four five six seven eight\n".repeat(40);
    let (editor, mut cx) = open(&source);
    let before = snapshot(&mut cx, &editor);

    apply(&mut cx, &editor, |settings| settings.line_height = 2.2);
    settle(&cx);

    let after = snapshot(&mut cx, &editor);
    assert_eq!(after.0, before.0, "line height must not rewrap text");
    assert_eq!(after.2, before.2);
    let line_height = cx.update(|_, cx| editor.read(cx).settings().line_height);
    assert!((line_height - 2.2).abs() < f32::EPSILON);
}

#[test]
fn wrap_and_tab_stop_use_the_background_reflow() {
    let source = "one two three four five six seven eight\n".repeat(40);
    let (editor, mut cx) = open(&source);
    let before = snapshot(&mut cx, &editor);

    apply(&mut cx, &editor, |settings| {
        settings.wrap = WrapMode::Character
    });
    apply(&mut cx, &editor, |settings| settings.tab_stop = 8);
    settle(&cx);

    let after = snapshot(&mut cx, &editor);
    assert!(after.0 > before.0, "a new measurement must be requested");
    assert!(
        after.1 > before.1,
        "the generation-checked job must publish exactly once"
    );
}
