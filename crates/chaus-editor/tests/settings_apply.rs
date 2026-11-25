//! Live settings changes must not recreate document or view state.

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

#[test]
fn applying_settings_keeps_text_selection_history_focus_and_file_state() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.txt");
    std::fs::write(&path, "alpha beta gamma\nsecond line\n").unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    cx.simulate_input("hello ");
    cx.simulate_keystrokes("shift-left shift-left");
    draw(cx);

    let before = cx.update(|window, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.selection(),
            editor.focus_handle(cx).is_focused(window),
            editor.document().can_undo(),
            editor.document().is_dirty(),
            editor.focus_handle(cx),
        )
    });

    let mut settings = Settings::default();
    settings.font_family = Some("Geist Mono".to_string());
    settings.font_size = 18.0;
    settings.line_height = 1.9;
    settings.wrap = WrapMode::Character;
    settings.tab_stop = 2;
    settings.expand_tabs = false;
    settings.vim = true;

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_settings(settings, cx));
    });
    draw(cx);

    let after = cx.update(|window, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.selection(),
            editor.focus_handle(cx).is_focused(window),
            editor.document().can_undo(),
            editor.document().is_dirty(),
            editor.wrap_mode(),
            editor.vim_enabled(),
        )
    });
    assert_eq!(after.0, before.0, "text must not change");
    assert_eq!(after.1, before.1, "selection/anchor must not change");
    assert!(after.2, "focus must stay on the document");
    assert_eq!(after.3, before.3, "undo history must survive");
    assert_eq!(after.4, before.4, "dirty state must survive");
    assert_eq!(after.5, WrapMode::Character, "the new wrap mode applies");
    assert!(
        !after.6,
        "the saved Vim default must not switch the live window"
    );
}

#[test]
fn two_editors_keep_independent_applied_settings() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let word_mode = {
        let (harness, cx) = app.add_window_view(|window, cx| {
            let mut settings = Settings::default();
            settings.wrap = WrapMode::Word;
            let editor = cx.new(|cx| Editor::new(cx, None, settings));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            Harness { editor }
        });
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.update(|_, cx| editor.read(cx).wrap_mode())
    };
    let none_mode = {
        let (harness, cx) = app.add_window_view(|window, cx| {
            let mut settings = Settings::default();
            settings.wrap = WrapMode::None;
            let editor = cx.new(|cx| Editor::new(cx, None, settings));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            Harness { editor }
        });
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        cx.update(|_, cx| editor.read(cx).wrap_mode())
    };
    assert_eq!(word_mode, WrapMode::Word);
    assert_eq!(none_mode, WrapMode::None);
}
