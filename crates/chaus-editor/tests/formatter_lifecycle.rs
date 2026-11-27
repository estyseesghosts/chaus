//! One formatter job per editor, revision-checked on completion.

use std::sync::{mpsc, Mutex};

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_format::{set_format_gate_for_test, Formatter};
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

static TEST_LOCK: Mutex<()> = Mutex::new(());

struct GateGuard;

impl Drop for GateGuard {
    fn drop(&mut self) {
        set_format_gate_for_test(None);
    }
}

#[cfg(windows)]
fn changing_formatter() -> Formatter {
    Formatter::new(
        "powershell",
        vec![
            "-NoProfile".to_owned(),
            "-Command".to_owned(),
            "$t = [Console]::In.ReadToEnd(); [Console]::Out.Write($t.Replace('one','ONE'))"
                .to_owned(),
        ],
    )
}

#[cfg(not(windows))]
fn changing_formatter() -> Formatter {
    Formatter::new("sh", vec!["-c".to_owned(), "sed 's/one/ONE/'".to_owned()])
}

fn open(source: &str) -> (Entity<Editor>, VisualTestContext, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("code.rs");
    std::fs::write(&path, source).unwrap();
    let loaded = chaus_io::load(&path).unwrap();

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(&mut *cx);
    (editor, cx.clone(), directory)
}

#[test]
fn repeated_format_commands_do_not_stack_jobs() {
    let _test = TEST_LOCK.lock().expect("formatter lifecycle lock poisoned");
    let _guard = GateGuard;
    let (release, gate) = mpsc::channel();
    set_format_gate_for_test(Some(gate));
    let (editor, mut cx, _directory) = open("one\ntwo\n");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_formatter_for_test(changing_formatter());
            editor.format_document(cx);
            editor.format_document(cx);
        });
    });
    let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned));
    assert_eq!(status.as_deref(), Some("Format already in progress"));

    release.send(()).expect("formatter task is still waiting");
    draw(&mut cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().text()),
        "ONE\ntwo\n",
        "the single accepted job must apply its result"
    );
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().undo_depth()),
        1,
        "one accepted format is one undo step"
    );
}

#[test]
fn an_edit_while_the_job_is_pending_discards_its_result() {
    let _test = TEST_LOCK.lock().expect("formatter lifecycle lock poisoned");
    let _guard = GateGuard;
    let (release, gate) = mpsc::channel();
    set_format_gate_for_test(Some(gate));
    let (editor, mut cx, _directory) = open("one\ntwo\n");
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_formatter_for_test(changing_formatter());
            editor.format_document(cx);
        });
    });
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("X").expect("valid keystroke"), cx);
    });
    let (edited, depth_after_edit) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.document().text(), editor.document().undo_depth())
    });
    assert_eq!(edited, "Xone\ntwo\n");

    release.send(()).expect("formatter task is still waiting");
    draw(&mut cx);
    let (text, status, depth) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.status_text().map(str::to_owned),
            editor.document().undo_depth(),
        )
    });
    assert_eq!(
        text, "Xone\ntwo\n",
        "stale output must not replace newer edits"
    );
    assert_eq!(
        status.as_deref(),
        Some("Document changed while formatting; result discarded")
    );
    assert_eq!(
        depth, depth_after_edit,
        "a discarded format must not add an undo entry"
    );
}
