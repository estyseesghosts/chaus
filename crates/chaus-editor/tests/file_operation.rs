//! Request identity and completion guards for per-editor open/save work.

use std::io;
use std::path::Path;
use std::sync::{mpsc, Mutex, OnceLock};

use chaus_core::settings::Settings;
use chaus_editor::{Editor, LoadHook};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Keystroke, Render,
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

fn open(path: &Path, hook: Option<LoadHook>) -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let loaded = chaus_io::load(path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        if let Some(hook) = hook {
            editor.update(cx, |editor, _| editor.set_load_hook_for_test(hook));
        }
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    (editor, cx.clone())
}

static RELEASE_LOAD: OnceLock<Mutex<Option<mpsc::Receiver<()>>>> = OnceLock::new();

// Only one test at a time may own the process-wide release channel.
static GATE_LOCK: Mutex<()> = Mutex::new(());

fn arm_load_gate() -> mpsc::Sender<()> {
    let (release, wait) = mpsc::channel();
    let slot = RELEASE_LOAD.get_or_init(|| Mutex::new(None));
    *slot.lock().expect("load gate lock poisoned") = Some(wait);
    release
}

fn gated_load(path: &Path) -> Result<chaus_io::LoadedFile, chaus_io::LoadError> {
    let wait = RELEASE_LOAD
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|mut slot| slot.take()))
        .expect("gated_load needs an armed release channel");
    wait.recv().expect("load gate sender dropped");
    chaus_io::load(path)
}

#[test]
fn a_second_save_as_while_the_picker_is_open_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let target = directory.path().join("target.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source, None);

    cx.simulate_input("X");
    draw(&mut cx);
    cx.simulate_keystrokes("ctrl-shift-s");
    assert!(
        cx.did_prompt_for_new_path(),
        "Save As must open a nonblocking picker"
    );

    cx.simulate_keystrokes("ctrl-shift-s");
    let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned));
    assert_eq!(
        status.as_deref(),
        Some("A file operation is already in progress")
    );
    assert!(
        cx.did_prompt_for_new_path(),
        "the second command must not open another picker"
    );

    cx.simulate_new_path_selection(|suggested| {
        assert_eq!(
            suggested,
            directory.path(),
            "the current file's directory is suggested"
        );
        Some(target.clone())
    });
    draw(&mut cx);
    assert!(
        !cx.did_prompt_for_new_path(),
        "the first picker result must settle the operation"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Xone\n");
    assert_eq!(cx.update(|_, cx| editor.read(cx).title()), "target.txt");
}

#[test]
fn a_forced_reload_does_not_discard_edits_made_after_it_started() {
    let _gate = GATE_LOCK.lock().expect("gate lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let replacement = directory.path().join("replacement.txt");
    std::fs::write(&source, "one\n").unwrap();
    std::fs::write(&replacement, "two\n").unwrap();
    let release = arm_load_gate();
    let (editor, mut cx) = open(&source, Some(gated_load));

    let started =
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.open_path(&replacement, true, cx)));
    assert!(
        started,
        "a forced reload starts while no operation is active"
    );
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("x").expect("valid keystroke"), cx);
    });
    let edited = cx.update(|_, cx| editor.read(cx).document().text());
    assert_eq!(
        edited, "xone\n",
        "the edit lands while the load is in flight"
    );

    release.send(()).expect("load task is still waiting");
    draw(&mut cx);

    let (text, title, status) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.title(),
            editor.status_text().map(str::to_owned),
        )
    });
    assert_eq!(
        text, "xone\n",
        "a late forced reload must not wipe newer edits"
    );
    assert_eq!(title, "source.txt — Edited");
    assert_eq!(
        status.as_deref(),
        Some("Open cancelled: the document changed while loading")
    );
    assert_eq!(std::fs::read_to_string(&replacement).unwrap(), "two\n");
}

#[test]
fn save_is_rejected_while_a_reload_is_pending() {
    let _gate = GATE_LOCK.lock().expect("gate lock poisoned");
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let replacement = directory.path().join("replacement.txt");
    std::fs::write(&source, "one\n").unwrap();
    std::fs::write(&replacement, "two\n").unwrap();
    let release = arm_load_gate();
    let (editor, mut cx) = open(&source, Some(gated_load));

    let started = cx
        .update(|_, cx| editor.update(cx, |editor, cx| editor.open_path(&replacement, false, cx)));
    assert!(started);
    let saved = cx.update(|_, cx| editor.update(cx, |editor, cx| editor.save_now(cx)));
    assert!(!saved, "Save must not overlap an in-flight open");
    let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned));
    assert_eq!(
        status.as_deref(),
        Some("A file operation is already in progress")
    );

    release.send(()).expect("load task is still waiting");
    draw(&mut cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().text()),
        "two\n"
    );
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).title()),
        "replacement.txt"
    );
}

#[test]
fn a_load_hook_failure_keeps_the_original_document() {
    fn failing_load(path: &Path) -> Result<chaus_io::LoadedFile, chaus_io::LoadError> {
        Err(chaus_io::LoadError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "blocked by test"),
        })
    }

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let replacement = directory.path().join("missing.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source, Some(failing_load));

    let started = cx
        .update(|_, cx| editor.update(cx, |editor, cx| editor.open_path(&replacement, false, cx)));
    assert!(started);
    draw(&mut cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().text()),
        "one\n"
    );
    assert_eq!(cx.update(|_, cx| editor.read(cx).title()), "source.txt");
    assert!(cx
        .update(|_, cx| editor.read(cx).status_text().map(str::to_owned))
        .as_deref()
        .is_some_and(|status| status.starts_with("Open failed:")));
}
