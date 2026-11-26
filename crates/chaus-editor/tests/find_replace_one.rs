//! Replace-one and focus-order regressions in the find bar.

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

fn run(test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
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
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    test(editor, cx);
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[test]
fn replace_one_replaces_the_match_selecting_the_current_selection() {
    run(|editor, cx| {
        draw(cx);
        cx.simulate_keystrokes("o n e space t w o space t w o");
        draw(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.open_find(true, window, cx));
        });
        draw(cx);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_find_text_for_test("two", "X", cx);
            });
        });
        draw(cx);

        let next = cx.debug_bounds("find-next").unwrap();
        cx.simulate_click(next.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).selection(),
                Selection::new(CharOffset(4), CharOffset(7)),
                "Find Next selects the first match"
            );
        });

        let replace = cx.debug_bounds("replace-one").unwrap();
        cx.simulate_click(replace.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(
                editor.read(cx).document().text(),
                "one X two",
                "Replace must change the selected match, not the one after it"
            );
        });

        // The post-replace Find Next leaves the following match selected.
        // Pressing Replace again must replace that selection itself.
        cx.simulate_click(replace.center(), Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            assert_eq!(editor.read(cx).document().text(), "one X X");
        });
    });
}

#[test]
fn tab_in_plain_find_does_not_focus_the_hidden_replace_field() {
    run(|editor, cx| {
        draw(cx);
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.open_find(false, window, cx));
        });
        draw(cx);
        cx.update(|_, cx| {
            assert!(!editor.read(cx).find_shows_replace_for_test());
        });

        cx.simulate_keystrokes("tab");
        draw(cx);
        cx.simulate_keystrokes("x y z");
        draw(cx);

        cx.update(|_, cx| {
            let (find, replace) = editor.read(cx).find_text_for_test(cx).unwrap();
            assert_eq!(
                find, "xyz",
                "typing must still reach the visible Find field"
            );
            assert_eq!(
                replace, "",
                "the hidden Replace field must not receive text"
            );
        });
    });
}
