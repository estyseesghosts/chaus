//! Per-window menu command state and routing.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    point, px, AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers,
    MouseButton, Render, TestAppContext, VisualTestContext, Window,
};
use silvestris::command::CommandRegistry;
use silvestris::core::command::CommandId;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

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

fn run(vim: bool, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = vim;
        let editor = cx.new(|cx| Editor::new(cx, None, settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn descriptor(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    id: &str,
) -> Option<silvestris::core::command::CommandDescriptor> {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .commands()
            .read(cx)
            .descriptors()
            .get(&CommandId::new(id))
            .cloned()
    })
}

fn invoke(editor: &Entity<Editor>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        let commands = editor.read(cx).commands().clone();
        CommandRegistry::invoke(&commands, &CommandId::new(id), window, cx);
    });
}

#[test]
fn the_vim_item_is_checked_for_the_window_state() {
    run(true, |editor, cx| {
        let vim = descriptor(&editor, cx, "view.vim_mode").unwrap();
        assert!(vim.checked, "Vim-enabled window must show a checkmark");
        invoke(&editor, cx, "view.vim_mode");
        draw(cx);
        let vim = descriptor(&editor, cx, "view.vim_mode").unwrap();
        assert!(!vim.checked, "menu toggle must update the checkmark");
        assert!(!cx.update(|_, cx| editor.read(cx).vim_enabled()));
    });
}

#[test]
fn two_windows_report_independent_checked_state() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let vim_checked = {
        let (harness, cx) = app.add_window_view(|window, cx| {
            let mut settings = Settings::default();
            settings.vim = true;
            let editor = cx.new(|cx| Editor::new(cx, None, settings));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            Harness { editor }
        });
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        descriptor(&editor, cx, "view.vim_mode").unwrap().checked
    };
    let plain_checked = {
        let (harness, cx) = app.add_window_view(|window, cx| {
            let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            Harness { editor }
        });
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        draw(cx);
        descriptor(&editor, cx, "view.vim_mode").unwrap().checked
    };

    assert!(vim_checked, "the Vim window keeps its checkmark");
    assert!(!plain_checked, "the other window stays unchecked");
}

#[test]
fn undo_is_disabled_until_there_is_history() {
    run(false, |editor, cx| {
        assert!(!descriptor(&editor, cx, "edit.undo").unwrap().enabled);
        assert!(!descriptor(&editor, cx, "edit.redo").unwrap().enabled);

        cx.simulate_input("a");
        draw(cx);
        assert!(descriptor(&editor, cx, "edit.undo").unwrap().enabled);

        invoke(&editor, cx, "edit.undo");
        draw(cx);
        assert!(!descriptor(&editor, cx, "edit.undo").unwrap().enabled);
        assert!(descriptor(&editor, cx, "edit.redo").unwrap().enabled);
    });
}

#[test]
fn cut_and_copy_are_disabled_without_a_selection() {
    run(false, |editor, cx| {
        assert!(!descriptor(&editor, cx, "edit.copy").unwrap().enabled);
        assert!(!descriptor(&editor, cx, "edit.cut").unwrap().enabled);
        cx.simulate_input("abc");
        cx.simulate_keystrokes("shift-left");
        draw(cx);
        assert!(descriptor(&editor, cx, "edit.copy").unwrap().enabled);
        assert!(descriptor(&editor, cx, "edit.cut").unwrap().enabled);
    });
}

#[test]
fn the_documented_shortcut_leaves_vim_normal_mode() {
    run(true, |editor, cx| {
        assert!(cx.update(|_, cx| editor.read(cx).vim_enabled()));
        cx.simulate_keystrokes("ctrl-shift-v");
        draw(cx);
        assert!(
            !cx.update(|_, cx| editor.read(cx).vim_enabled()),
            "Ctrl+Shift+V must toggle from Normal mode too"
        );
    });
}

#[test]
fn right_clicking_the_titlebar_opens_the_menus_without_a_toolbar() {
    run(false, |editor, cx| {
        assert!(
            cx.debug_bounds("menu-file").is_none(),
            "there must be no visible menu bar"
        );

        cx.simulate_mouse_down(
            point(px(450.), px(20.)),
            MouseButton::Right,
            Modifiers::none(),
        );
        draw(cx);
        assert!(
            cx.debug_bounds("menu-shield").is_some(),
            "right-clicking the titlebar must open the popup"
        );

        let view = cx
            .debug_bounds("menu-item-0-2")
            .expect("View is the third root submenu");
        cx.simulate_click(view.center(), Modifiers::none());
        draw(cx);

        let vim = cx
            .debug_bounds("menu-item-1-0")
            .expect("Vim Mode is the first View item");
        cx.simulate_click(vim.center(), Modifiers::none());
        draw(cx);

        assert!(
            cx.debug_bounds("menu-shield").is_none(),
            "choosing an item closes the menu"
        );
        assert!(
            cx.update(|_, cx| editor.read(cx).vim_enabled()),
            "the menu item must toggle the focused window, not a global default"
        );
        assert!(descriptor(&editor, cx, "view.vim_mode").unwrap().checked);
    });
}

#[test]
fn right_clicking_the_document_does_not_open_the_menu() {
    run(false, |_editor, cx| {
        cx.simulate_mouse_down(
            point(px(450.), px(200.)),
            MouseButton::Right,
            Modifiers::none(),
        );
        draw(cx);
        assert!(
            cx.debug_bounds("menu-shield").is_none(),
            "the document canvas stays free of menus"
        );
    });
}

#[test]
fn the_vim_shortcut_is_ignored_while_the_find_bar_owns_the_keyboard() {
    run(false, |editor, cx| {
        cx.simulate_keystrokes("ctrl-f");
        draw(cx);
        assert!(cx.debug_bounds("chaus-find-bar").is_some());
        cx.simulate_keystrokes("ctrl-shift-v");
        draw(cx);
        assert!(
            !cx.update(|_, cx| editor.read(cx).vim_enabled()),
            "Vim must not toggle behind the find bar"
        );
    });
}

/// Deterministic stand-in for the native Save As picker. A real dialog would
/// block this headless test and pop a window on the developer's desktop.
#[derive(Default)]
struct PickerLog {
    calls: usize,
    suggested: Option<Option<PathBuf>>,
    result: Option<PathBuf>,
}

thread_local! {
    static PICKER: RefCell<PickerLog> = RefCell::new(PickerLog::default());
}

fn fake_picker(suggested: Option<&Path>) -> Option<PathBuf> {
    PICKER.with(|log| {
        let mut log = log.borrow_mut();
        log.calls += 1;
        log.suggested = Some(suggested.map(Path::to_path_buf));
        log.result.clone()
    })
}

fn arm_picker(result: PathBuf) {
    PICKER.with(|log| {
        *log.borrow_mut() = PickerLog {
            calls: 0,
            suggested: None,
            result: Some(result),
        };
    });
}

fn picker_calls() -> (usize, Option<Option<PathBuf>>) {
    PICKER.with(|log| {
        let log = log.borrow();
        (log.calls, log.suggested.clone())
    })
}

/// Press Save on a fresh untitled window through `trigger`, then report the
/// window title the save settled on.
fn save_untitled_through(
    target: &Path,
    trigger: impl FnOnce(&Entity<Editor>, &mut VisualTestContext),
) -> String {
    let mut title = String::new();
    run(false, |editor, cx| {
        arm_picker(target.to_path_buf());
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| editor.set_save_picker_for_test(fake_picker));
        });
        trigger(&editor, cx);
        draw(cx);

        let (calls, suggested) = picker_calls();
        assert_eq!(calls, 1, "Save must ask for a path instead of refusing");
        assert_eq!(
            suggested,
            Some(None),
            "an untitled document has no path to suggest"
        );
        let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned));
        assert_ne!(
            status.as_deref(),
            Some("Untitled document: use Save As"),
            "Save must open Save As, not print a refusal"
        );
        title = cx.update(|_, cx| editor.read(cx).title());
    });
    title
}

#[test]
fn menu_save_and_the_shortcut_both_route_untitled_documents_to_save_as() {
    let menu_target = std::env::temp_dir().join("chaus-menu-save.txt");
    let key_target = std::env::temp_dir().join("chaus-key-save.txt");

    let menu_title = save_untitled_through(&menu_target, |editor, cx| {
        invoke(editor, cx, "file.save");
    });
    assert_eq!(menu_title, "chaus-menu-save.txt");

    let key_title = save_untitled_through(&key_target, |_editor, cx| {
        cx.simulate_keystrokes("ctrl-s");
    });
    assert_eq!(key_title, "chaus-key-save.txt");

    let _ = std::fs::remove_file(menu_target);
    let _ = std::fs::remove_file(key_target);
}

#[test]
fn cancelling_the_menu_save_as_leaves_the_untitled_document_alone() {
    let target = std::env::temp_dir().join("chaus-cancelled-save.txt");
    run(false, |editor, cx| {
        arm_picker(target.clone());
        PICKER.with(|log| log.borrow_mut().result = None);
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| editor.set_save_picker_for_test(fake_picker));
        });
        cx.simulate_input("draft");
        draw(cx);

        invoke(&editor, cx, "file.save");
        draw(cx);

        let (calls, _) = picker_calls();
        assert_eq!(calls, 1, "cancelling still goes through the picker");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.title(), "Untitled — Edited");
            assert_eq!(editor.document().text(), "draft");
            assert_eq!(editor.status_text(), Some("Save cancelled"));
        });
        assert!(
            !target.exists(),
            "a cancelled picker must not create the target file"
        );
    });
}
