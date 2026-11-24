//! `:wqa` across real windows: save every dirty named document, close only
//! after all saves landed, and leave everything open on any failure.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

use chaus_core::settings::Settings;
use chaus_editor::{set_save_gate_for_test, Editor};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Keystroke, Render,
    TestAppContext, VisualTestContext, Window, WindowOptions,
};

/// The save gate is process-wide, so tests in this binary take turns.
static TEST_LOCK: Mutex<()> = Mutex::new(());

struct Harness {
    editor: Entity<Editor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone().into_any_element()
    }
}

fn init(app: &mut TestAppContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
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
    for _ in 0..6 {
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

fn start_ex(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(":");
    send_text(editor, cx, ":");
    send_text(editor, cx, command);
    cx.simulate_keystrokes("enter");
}

fn ex(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    start_ex(editor, cx, command);
    settle(cx);
}

fn insert(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    press(editor, cx, "G");
    press(editor, cx, "A");
    send_text(editor, cx, text);
    cx.simulate_keystrokes("escape");
    settle(cx);
}

/// Dispatch one key without draining the background executor.
fn dispatch(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = Keystroke::parse(key).expect("valid keystroke");
    cx.update(|window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    });
}

/// Start an Ex command without letting its background work run, so a gated
/// save remains pending while the test dispatches more keys.
fn start_ex_without_draining(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    dispatch(cx, "escape");
    dispatch(cx, ":");
    send_text(editor, cx, ":");
    send_text(editor, cx, command);
    dispatch(cx, "enter");
}

fn status(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).status_text().unwrap_or_default().to_owned())
}

fn dirty(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| editor.read(cx).document().is_dirty())
}

fn window_count(cx: &mut VisualTestContext) -> usize {
    cx.run_until_parked();
    cx.windows().len()
}

fn add_first<'a>(
    app: &'a mut TestAppContext,
    path: Option<&Path>,
) -> (Entity<Harness>, &'a mut VisualTestContext) {
    let loaded = path.map(|path| chaus_io::load(path).unwrap());
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, loaded, settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    draw(cx);
    (harness, cx)
}

fn open_named(cx: &mut VisualTestContext, path: &Path) -> gpui::WindowHandle<Editor> {
    let loaded = chaus_io::load(path).unwrap();
    let handle = cx.update(|_, app| {
        app.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()))
        })
        .unwrap()
    });
    draw(cx);
    handle
}

fn open_untitled(cx: &mut VisualTestContext) -> gpui::WindowHandle<Editor> {
    let handle = cx.update(|_, app| {
        app.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|cx| Editor::new(cx, None, Settings::default()))
        })
        .unwrap()
    });
    draw(cx);
    handle
}

fn type_into(handle: &gpui::WindowHandle<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|_, app| {
        handle
            .update(app, |editor, window, cx| {
                gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
            })
            .unwrap();
    });
    draw(cx);
}

fn handle_text(handle: &gpui::WindowHandle<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, app| {
        handle
            .read(app)
            .map(|editor| editor.document().text())
            .unwrap_or_default()
    })
}

fn handle_dirty(handle: &gpui::WindowHandle<Editor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, app| {
        handle
            .read(app)
            .map(|editor| editor.document().is_dirty())
            .unwrap_or(false)
    })
}

fn fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(name);
    (directory, path)
}

#[test]
fn wqa_saves_every_dirty_named_window_then_closes() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let (directory, a) = fixture("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let mut app = TestAppContext::single();
    init(&mut app);
    let (harness, cx) = add_first(&mut app, Some(&a));
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    insert(&editor, cx, "one");
    let second = open_named(cx, &b);
    type_into(&second, cx, "more ");
    assert!(dirty(&editor, cx));
    assert!(handle_dirty(&second, cx));

    ex(&editor, cx, "wqa");
    assert_eq!(window_count(cx), 0);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\none");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "more b\n");
}

#[test]
fn wqa_stops_for_a_dirty_untitled_window() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let (_directory, a) = fixture("a.txt");
    std::fs::write(&a, "a\n").unwrap();
    let mut app = TestAppContext::single();
    init(&mut app);
    let (harness, cx) = add_first(&mut app, Some(&a));
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    insert(&editor, cx, "one");
    let second = open_untitled(cx);
    type_into(&second, cx, "untitled");

    ex(&editor, cx, "wqa");
    assert_eq!(window_count(cx), 2);
    assert!(
        status(&editor, cx).contains("untitled"),
        "{}",
        status(&editor, cx)
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\n");
    assert!(dirty(&editor, cx));
    assert!(handle_dirty(&second, cx));
    assert_eq!(handle_text(&second, cx), "untitled");
}

#[test]
fn wqa_stops_when_two_windows_target_the_same_file() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let (_directory, a) = fixture("a.txt");
    std::fs::write(&a, "a\n").unwrap();
    let mut app = TestAppContext::single();
    init(&mut app);
    let (harness, cx) = add_first(&mut app, Some(&a));
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    insert(&editor, cx, "one");
    let second = open_named(cx, &a);
    type_into(&second, cx, "two ");
    assert!(dirty(&editor, cx));
    assert!(handle_dirty(&second, cx));

    ex(&editor, cx, "wqa");
    assert_eq!(window_count(cx), 2);
    assert!(
        status(&editor, cx).contains("same file"),
        "{}",
        status(&editor, cx)
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\n");
    assert!(dirty(&editor, cx));
}

#[test]
fn a_conflicting_window_keeps_all_windows_open_after_other_saves() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let (directory, a) = fixture("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let mut app = TestAppContext::single();
    init(&mut app);
    let (harness, cx) = add_first(&mut app, Some(&a));
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    insert(&editor, cx, "one");
    let second = open_named(cx, &b);
    type_into(&second, cx, "more ");
    std::fs::write(&a, "external\n").unwrap();

    ex(&editor, cx, "wqa");
    assert_eq!(window_count(cx), 2);
    assert!(
        status(&editor, cx).contains("changed on disk"),
        "{}",
        status(&editor, cx)
    );
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "external\n",
        "the conflicting target must not be overwritten"
    );
    assert!(dirty(&editor, cx));
    assert!(cx.debug_bounds("chaus-conflict-sheet").is_some());
}

#[test]
fn an_edit_during_the_pending_save_keeps_the_window_open() {
    let _guard = TEST_LOCK.lock().expect("test lock poisoned");
    let (_directory, a) = fixture("a.txt");
    std::fs::write(&a, "a\n").unwrap();
    let (release, wait) = mpsc::channel();
    set_save_gate_for_test(Some(wait));
    let mut app = TestAppContext::single();
    init(&mut app);
    let (harness, cx) = add_first(&mut app, Some(&a));
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    insert(&editor, cx, "one");

    // Start `:wqa` without letting the save task run, then edit.
    start_ex_without_draining(&editor, cx, "wqa");
    dispatch(cx, "x");
    release.send(()).expect("save task is still waiting");
    settle(cx);

    assert_eq!(window_count(cx), 1);
    assert!(
        status(&editor, cx).contains("changed while its save was running"),
        "{}",
        status(&editor, cx)
    );
    assert!(dirty(&editor, cx));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\none");
}
