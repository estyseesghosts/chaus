//! `:o!` uses the same asynchronous open lifecycle and stale-load guard as
//! `:e!`; a late load must not overwrite edits made after it started.

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
    if cx.windows().is_empty() {
        return;
    }
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// Dispatch one key without draining the background executor.
fn dispatch(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = Keystroke::parse(key).expect("valid keystroke");
    cx.update(|window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    });
}

static RELEASE_LOAD: OnceLock<Mutex<Option<mpsc::Receiver<()>>>> = OnceLock::new();

fn gated_load(path: &Path) -> Result<chaus_io::LoadedFile, chaus_io::LoadError> {
    let wait = RELEASE_LOAD
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|mut slot| slot.take()))
        .expect("gated_load needs an armed release channel");
    wait.recv().expect("load gate sender dropped");
    chaus_io::load(path)
}

#[test]
fn a_late_open_alias_load_keeps_edits_made_while_it_was_running() {
    let (release, wait) = mpsc::channel();
    *RELEASE_LOAD
        .get_or_init(|| Mutex::new(None))
        .lock()
        .expect("load gate lock poisoned") = Some(wait);

    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let replacement = directory.path().join("replacement.txt");
    std::fs::write(&source, "one\n").unwrap();
    std::fs::write(&replacement, "two\n").unwrap();
    let loaded = chaus_io::load(&source).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let mut settings = Settings::default();
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        editor.update(cx, |editor, _| {
            editor.set_load_hook_for_test(gated_load as LoadHook)
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    dispatch(cx, "escape");
    dispatch(cx, ":");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, ":", window, cx);
            gpui::EntityInputHandler::replace_text_in_range(
                editor,
                None,
                &format!("o! \"{}\"", replacement.display()),
                window,
                cx,
            );
        });
    });
    // Dispatch Enter without running the executor: the gated load must stay in
    // flight while the test edits the current document.
    dispatch(cx, "enter");

    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("x").expect("valid keystroke"), cx);
    });
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().text()),
        "ne\n",
        "the edit lands while the open is in flight"
    );

    release.send(()).expect("load task is still waiting");
    draw(cx);

    let (text, title, status) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.document().text(),
            editor.title(),
            editor.status_text().map(str::to_owned),
        )
    });
    assert_eq!(text, "ne\n", "a late :o! must not wipe newer edits");
    assert_eq!(title, "source.txt");
    assert_eq!(
        status.as_deref(),
        Some("Open cancelled: the document changed while loading")
    );
    assert_eq!(std::fs::read_to_string(&replacement).unwrap(), "two\n");
}
