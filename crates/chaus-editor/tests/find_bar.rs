use chaus_core::position::{CharOffset, Selection};
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
        chaus_editor::bind_keys(cx);
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
fn find_next_selects_the_next_literal_match() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("o n e space t w o space o n e");
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).document().text(), "one two one");
        });

        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.open_find(false, window, cx));
        });
        draw(cx);
        assert!(cx.debug_bounds("chaus-find-bar").is_some());

        cx.simulate_keystrokes("t w o");
        draw(cx);
        let next = cx.debug_bounds("find-next").unwrap();
        cx.simulate_click(next.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).selection(),
                Selection::new(CharOffset(4), CharOffset(7))
            );
        });

        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(cx.debug_bounds("chaus-find-bar").is_none());
    });
}

#[test]
fn replace_all_rewrites_every_match() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("o n e space t w o space t w o");
        draw(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.open_find(true, window, cx));
        });
        draw(cx);
        cx.simulate_keystrokes("t w o");
        draw(cx);
        let all = cx.debug_bounds("replace-all").unwrap();
        cx.simulate_click(all.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).document().text(),
                "one  ",
                "Replace All with an empty replacement removes every match"
            );
        });
    });
}

#[test]
fn replace_all_text_is_one_undoable_transaction() {
    run(|harness, cx| {
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.simulate_keystrokes("o n e space t w o space t w o");
        draw(cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.replace_all_text("two", "X", false, cx), 2);
                assert_eq!(editor.document().text(), "one X X");
            });
        });
    });
}
