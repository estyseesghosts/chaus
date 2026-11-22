//! Scheduling behaviour of the periodic conflict watcher.

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
    });
}

#[test]
fn a_same_size_external_edit_is_caught_by_the_hash() {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("same-size.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, mut cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut cx);

    // Same byte count as "one\n": only a content hash can notice this.
    std::fs::write(&path, "two\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
    });
    draw(&mut cx);

    assert!(
        cx.debug_bounds("chaus-disk-banner").is_some(),
        "a size-preserving edit must still surface as a conflict"
    );
}

#[test]
fn a_check_in_flight_is_not_stacked_or_duplicated() {
    let mut app = TestAppContext::single();
    initialize(&mut app);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("stack.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, mut cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut cx);

    std::fs::write(&path, "changed\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.check_disk_now(cx);
            assert!(editor.is_checking_disk());
            // A second request while the first is running must be ignored
            // rather than spawning another read of the same file.
            editor.check_disk_now(cx);
            assert!(editor.is_checking_disk());
        });
    });
    draw(&mut cx);

    cx.update(|_, cx| {
        assert!(
            !editor.read(cx).is_checking_disk(),
            "the completed check must clear the in-flight flag"
        );
    });
    assert!(cx.debug_bounds("chaus-disk-banner").is_some());
}
