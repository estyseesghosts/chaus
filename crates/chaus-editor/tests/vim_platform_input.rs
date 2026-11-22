//! Platform delivery: Windows sends a separate WM_CHAR after key-down.
//!
//! `simulate_keystrokes` only dispatches key-down/up, so the earlier Vim tests
//! could pass while the real app inserted every Normal-mode command as text.
//! These tests replay the platform sequence: key-down, then character input.

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

/// Replay a Normal/Command-mode key as the OS delivers it: key-down followed by
/// the separate character message even when the key-down was consumed.
fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
    send_char(editor, cx, text);
}

/// Deliver one standalone character message, like WM_CHAR on Windows.
fn send_char(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

/// Type in Insert mode. The test harness already sends the character for an
/// unconsumed key-down, exactly like the platform.
fn type_text(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
}

fn run(text: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vim.txt");
    std::fs::write(&path, text).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, mut cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut cx);
    test(editor, &mut cx);
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn mode(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<&'static str> {
    cx.update(|_, cx| editor.read(cx).vim_mode_label())
}

#[test]
fn normal_mode_commands_are_not_inserted_as_text() {
    run("", |editor, cx| {
        assert_eq!(mode(&editor, cx), Some("NORMAL"));
        press(&editor, cx, "j");
        assert_eq!(text(&editor, cx), "", "'j' must move, not insert");
        press(&editor, cx, "d");
        assert_eq!(text(&editor, cx), "", "'d' must wait for a motion");
        press(&editor, cx, "x");
        assert_eq!(
            text(&editor, cx),
            "",
            "'x' must not insert on an empty line"
        );
    });
}

#[test]
fn insert_mode_still_receives_character_input() {
    run("", |editor, cx| {
        press(&editor, cx, "i");
        assert_eq!(mode(&editor, cx), Some("INSERT"));
        type_text(cx, "a");
        type_text(cx, "b");
        assert_eq!(
            text(&editor, cx),
            "ab",
            "Insert-mode letters must still be inserted"
        );
    });
}

#[test]
fn a_normal_mode_delete_removes_instead_of_inserting() {
    run("abc", |editor, cx| {
        press(&editor, cx, "x");
        assert_eq!(text(&editor, cx), "bc");
        press(&editor, cx, "d");
        press(&editor, cx, "l");
        assert_eq!(text(&editor, cx), "c");
    });
}

#[test]
fn command_line_letters_build_the_command_without_entering_the_buffer() {
    run("one   \ntwo\n", |editor, cx| {
        for character in ":trim".chars() {
            press(&editor, cx, &character.to_string());
        }
        assert_eq!(mode(&editor, cx), Some("COMMAND"));
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned)),
            None,
            "a successful :trim should not leave a message"
        );
        assert_eq!(text(&editor, cx), "one\ntwo\n");
        assert!(
            !text(&editor, cx).contains(':'),
            "the command text must never enter the buffer"
        );
    });
}

#[test]
fn escape_returns_to_normal_and_stops_inserting() {
    run("", |editor, cx| {
        press(&editor, cx, "i");
        type_text(cx, "a");
        assert_eq!(text(&editor, cx), "a");
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert_eq!(mode(&editor, cx), Some("NORMAL"));
        press(&editor, cx, "b");
        assert_eq!(text(&editor, cx), "a", "Normal mode must not insert 'b'");
    });
}

#[test]
fn navigation_and_enter_reach_the_modal_engine_in_normal_mode() {
    run("ab\ncd\n", |editor, cx| {
        // Right, Enter (line down), Backspace (left), Delete (x).
        cx.simulate_keystrokes("right");
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "ab\ncd\n",
            "navigation and Enter must not insert or delete in Normal mode"
        );
        cx.simulate_keystrokes("delete");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "ab\nd\n",
            "Delete in Normal mode should act as x"
        );
    });
}

#[test]
fn insert_mode_keeps_native_enter_and_backspace() {
    run("", |editor, cx| {
        press(&editor, cx, "i");
        type_text(cx, "a");
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("b");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "a\nb",
            "Enter must insert a line break in Insert mode"
        );
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_eq!(text(&editor, cx), "a\n");
        cx.simulate_keystrokes("backspace");
        draw(cx);
        assert_eq!(
            text(&editor, cx),
            "a",
            "a second Backspace joins the lines in Insert mode"
        );
    });
}
