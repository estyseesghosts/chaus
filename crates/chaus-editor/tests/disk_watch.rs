use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, Render,
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

#[test]
fn an_external_change_is_surfaced_without_touching_the_document() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("watched.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    std::fs::write(&path, "two\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
    });
    draw(cx);
    assert!(
        cx.debug_bounds("chaus-disk-banner").is_some(),
        "the watcher must show a changed-on-disk banner"
    );
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).document().text(),
            "one\n",
            "polling must not overwrite the in-memory document"
        );
    });

    let review = cx.debug_bounds("disk-review").unwrap();
    cx.simulate_click(review.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_some());
    let cancel = cx.debug_bounds("conflict-cancel").unwrap();
    cx.simulate_click(cancel.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_none());
    assert!(cx.debug_bounds("chaus-disk-banner").is_some());

    std::fs::write(&path, "one\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.check_disk_now(cx));
    });
    draw(cx);
    assert!(
        cx.debug_bounds("chaus-disk-banner").is_none(),
        "reverting the file clears the warning"
    );
}
