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
fn a_save_conflict_offers_explicit_choices() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conflict.txt");
    std::fs::write(&path, "one\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded.clone()), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    cx.simulate_keystrokes("X");
    draw(cx);
    std::fs::write(&path, "two\n").unwrap();
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.save_now(cx), "the save should start");
        });
    });
    draw(cx);

    assert!(
        cx.debug_bounds("chaus-conflict-sheet").is_some(),
        "a conflicting save must not silently overwrite"
    );
    assert!(cx.debug_bounds("conflict-reload").is_some());
    assert!(cx.debug_bounds("conflict-save-as").is_some());
    assert!(cx.debug_bounds("conflict-overwrite").is_some());

    let cancel = cx.debug_bounds("conflict-cancel").unwrap();
    cx.simulate_click(cancel.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "Xone\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
    });
}
