//! `~` at the editor boundary: Ex path resolution, completion and quoting.
//!
//! The Vim parser records a leading unquoted `~` symbolically. The editor
//! expands it through `chaus_io::expand_home`; a quoted `"~/..."` stays a
//! literal relative name. These tests use a private HOME so nothing touches
//! the real user profile.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

/// `HOME` is process-wide, so tests that change it must not run concurrently.
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct HomeGuard(Option<OsString>);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        if let Some(home) = self.0.take() {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    if cx.windows().is_empty() {
        return;
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        draw(cx);
    }
}

fn send_text(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

fn press(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes(text);
    send_text(editor, cx, text);
}

fn ex(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(":");
    send_text(editor, cx, ":");
    send_text(editor, cx, command);
    cx.simulate_keystrokes("enter");
    settle(cx);
}

fn insert(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    press(editor, cx, "G");
    press(editor, cx, "A");
    send_text(editor, cx, text);
    cx.simulate_keystrokes("escape");
    settle(cx);
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn status(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).status_text().unwrap_or_default().to_owned())
}

fn dirty(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| editor.read(cx).document().is_dirty())
}

fn file_path(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<PathBuf> {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .file_state()
            .map(|file| file.meta.user_requested_path.clone())
    })
}

fn run(initial: Option<&Path>, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let loaded = initial.map(|path| chaus_io::load(path).unwrap());
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, loaded, settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

#[test]
fn home_read_inserts_below_the_current_line() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let notes = home.join("notes.txt");
    let extra = home.join("extra.txt");
    std::fs::write(&notes, "from home\n").unwrap();
    std::fs::write(&extra, "extra line\n").unwrap();

    let _home_lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    let _guard = HomeGuard(previous);

    run(None, |editor, cx| {
        ex(&editor, cx, "e ~/notes.txt");
        ex(&editor, cx, "r ~/extra.txt");
        assert_eq!(text(&editor, cx), "from home\nextra line\n");
    });
}

#[test]
fn home_paths_open_save_complete_and_quote_literally() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let notes = home.join("notes.txt");
    let other = home.join("other.txt");
    std::fs::write(&notes, "from home\n").unwrap();
    std::fs::write(&other, "other\n").unwrap();

    let _home_lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let previous = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    let _guard = HomeGuard(previous);

    run(None, |editor, cx| {
        ex(&editor, cx, "e ~/notes.txt");
        assert_eq!(text(&editor, cx), "from home\n");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(notes.as_path()));

        insert(&editor, cx, "edited");
        ex(&editor, cx, "saveas ~/copy.txt");
        let copy = home.join("copy.txt");
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "from home\nedited");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(copy.as_path()));

        ex(&editor, cx, "o ~/other.txt");
        assert_eq!(text(&editor, cx), "other\n");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(other.as_path()));

        ex(&editor, cx, "o! ~/notes.txt");
        assert_eq!(text(&editor, cx), "from home\n");

        // Completion lists the expanded home directory but spells the result
        // back as `~/...` so the command keeps its meaning.
        cx.simulate_keystrokes("escape");
        cx.simulate_keystrokes(":");
        send_text(&editor, cx, ":");
        send_text(&editor, cx, "e ~/no");
        cx.simulate_keystrokes("tab");
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).command_line_text()),
            Some("e ~/notes.txt".to_owned())
        );
        cx.simulate_keystrokes("escape");
        settle(cx);

        // A quoted tilde is a literal relative name, not the home directory.
        let before = text(&editor, cx);
        ex(&editor, cx, "e \"~/chaus-literal-missing-file.txt\"");
        assert_eq!(text(&editor, cx), before);
        assert!(
            status(&editor, cx).contains("Open failed"),
            "{}",
            status(&editor, cx)
        );
    });

    run(Some(&notes), |editor, cx| {
        insert(&editor, cx, "dirty");
        ex(&editor, cx, &format!("o \"{}\"", other.display()));
        assert_eq!(text(&editor, cx), "from home\ndirty");
        assert!(
            status(&editor, cx).contains("No write since last change"),
            "{}",
            status(&editor, cx)
        );
        ex(&editor, cx, &format!("o! \"{}\"", other.display()));
        assert_eq!(text(&editor, cx), "other\n");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(other.as_path()));
    });

    run(Some(&notes), |editor, cx| {
        insert(&editor, cx, "dirty");
        ex(&editor, cx, "w ~/other.txt");
        assert_eq!(
            std::fs::read_to_string(&other).unwrap(),
            "other\n",
            "an existing home target is not overwritten without bang"
        );
        assert!(
            status(&editor, cx).contains("exists"),
            "{}",
            status(&editor, cx)
        );
        assert!(dirty(&editor, cx));
        ex(&editor, cx, "w! ~/other.txt");
        assert_eq!(
            std::fs::read_to_string(&other).unwrap(),
            "from home\ndirty",
            "bang replaces the existing home target"
        );
        assert_eq!(
            file_path(&editor, cx).as_deref(),
            Some(notes.as_path()),
            "writing a copy keeps the original association"
        );
        assert!(dirty(&editor, cx), "writing a copy is not a save");
    });
}
