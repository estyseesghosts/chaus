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

fn run(test: impl FnOnce(Entity<Harness>, &mut VisualTestContext)) {
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
    test(harness, cx);
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[test]
fn a_dirty_close_shows_the_three_choice_sheet_and_cancel_keeps_the_window() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("a");
        draw(cx);
        cx.update(|_, cx| {
            assert!(
                editor.read(cx).document().is_dirty(),
                "typing must mark the document dirty"
            );
        });

        let allowed = cx
            .update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
        assert!(!allowed, "a dirty close must keep the window open");
        draw(cx);
        assert!(
            cx.debug_bounds("chaus-close-sheet").is_some(),
            "the save/discard/cancel sheet must be visible"
        );
        assert!(cx.debug_bounds("close-save").is_some());
        assert!(cx.debug_bounds("close-discard").is_some());
        assert!(cx.debug_bounds("close-cancel").is_some());

        cx.simulate_keystrokes("b");
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).document().text(),
                "a",
                "the sheet must block typing into the document"
            );
        });

        let cancel = cx.debug_bounds("close-cancel").unwrap();
        cx.simulate_click(cancel.center(), Modifiers::default());
        draw(cx);
        assert!(
            cx.debug_bounds("chaus-close-sheet").is_none(),
            "cancelling must dismiss the sheet"
        );
        cx.update(|_, cx| assert!(editor.read(cx).document().is_dirty()));
    });
}

#[test]
fn a_clean_close_does_not_show_the_sheet() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        let allowed = cx
            .update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
        assert!(allowed, "a clean document closes without a prompt");
        draw(cx);
        assert!(cx.debug_bounds("chaus-close-sheet").is_none());
    });
}
