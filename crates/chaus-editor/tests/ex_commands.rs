//! Ex commands end to end through the editor: typed into the command line,
//! executed through the shared file session, observed on disk and in state.

use std::path::{Path, PathBuf};

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
    // These tests open one window; once an Ex command closes it there is
    // nothing left to draw.
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

fn init(app: &mut TestAppContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
}

fn vim_settings() -> Settings {
    let mut settings = Settings::default();
    settings.vim = true;
    settings
}

/// Open one Vim-mode window on `file` (or untitled) and run `test`.
fn run(file: Option<&Path>, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    init(&mut app);
    let loaded = file.map(|path| chaus_io::load(path).unwrap());
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, loaded, vim_settings()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

/// Type `:command` and Enter, delivering the text through the platform text
/// path exactly as WM_CHAR / IME commit would.
fn ex(editor: &Entity<Editor>, cx: &mut VisualTestContext, command: &str) {
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes(":");
    send_text(editor, cx, ":"); // the WM_CHAR for the consumed ':' is dropped
    send_text(editor, cx, command);
    cx.simulate_keystrokes("enter");
    settle(cx);
}

fn send_text(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

fn insert(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("G");
    cx.simulate_keystrokes("A");
    send_text(editor, cx, "A"); // the WM_CHAR for the consumed 'A' is dropped
    send_text(editor, cx, text);
    cx.simulate_keystrokes("escape");
    settle(cx);
}

fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).document().text())
}

fn dirty(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| editor.read(cx).document().is_dirty())
}

fn status(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| editor.read(cx).status_text().unwrap_or_default().to_owned())
}

fn file_path(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<PathBuf> {
    cx.update(|_, cx| {
        editor
            .read(cx)
            .file_state()
            .map(|file| file.meta.user_requested_path.clone())
    })
}

fn window_count(cx: &mut VisualTestContext) -> usize {
    cx.run_until_parked();
    cx.windows().len()
}

fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

#[test]
fn write_to_another_path_is_a_copy_and_keeps_the_association() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let b = directory.path().join("copy of a.txt");
    std::fs::write(&a, "one\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "two");
        assert!(dirty(&editor, cx));
        ex(&editor, cx, &format!("w {}", quoted(&b)));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "one\ntwo");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one\n", "A untouched");
        assert!(
            dirty(&editor, cx),
            "a copy does not mark the document saved"
        );
        assert_eq!(file_path(&editor, cx).as_deref(), Some(a.as_path()));

        ex(&editor, cx, "w");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one\ntwo");
        assert!(!dirty(&editor, cx));
    });
}

#[test]
fn write_copy_refuses_an_existing_file_without_bang() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, "mine\n").unwrap();
    std::fs::write(&b, "theirs\n").unwrap();
    run(Some(&a), |editor, cx| {
        ex(&editor, cx, &format!("w {}", quoted(&b)));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "theirs\n");
        assert!(
            status(&editor, cx).contains("exists"),
            "{}",
            status(&editor, cx)
        );
        ex(&editor, cx, &format!("w! {}", quoted(&b)));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "mine\n");
    });
}

#[test]
fn saveas_retargets_only_after_the_write_lands() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let c = directory.path().join("ünïcode c.txt");
    std::fs::write(&a, "x\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "y");
        ex(&editor, cx, &format!("saveas {}", quoted(&c)));
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "x\ny");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(c.as_path()));
        assert!(!dirty(&editor, cx));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "x\n");
    });
}

#[test]
fn saveas_into_a_missing_directory_keeps_the_old_association() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "x\n").unwrap();
    let missing = directory.path().join("no such dir").join("c.txt");
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "y");
        ex(&editor, cx, &format!("saveas {}", quoted(&missing)));
        assert!(
            status(&editor, cx).contains("failed"),
            "{}",
            status(&editor, cx)
        );
        assert_eq!(file_path(&editor, cx).as_deref(), Some(a.as_path()));
        assert!(dirty(&editor, cx));
    });
}

#[test]
fn untitled_write_with_a_path_associates_on_success() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("new.txt");
    run(None, |editor, cx| {
        insert(&editor, cx, "fresh");
        ex(&editor, cx, &format!("w {}", quoted(&target)));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "fresh");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(target.as_path()));
        assert!(!dirty(&editor, cx));
    });
}

#[test]
fn quit_on_a_dirty_document_reports_and_stays_open() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "x\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "y");
        ex(&editor, cx, "q");
        assert_eq!(window_count(cx), 1);
        assert!(status(&editor, cx).contains("No write since last change"));
        assert!(
            cx.debug_bounds("chaus-close-sheet").is_none(),
            ":q must not open the GUI close sheet"
        );
        ex(&editor, cx, "q!");
        assert_eq!(window_count(cx), 0);
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "x\n",
            "q! writes nothing"
        );
    });
}

#[test]
fn quit_closes_a_clean_window_and_wq_saves_first() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "x\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "y");
        ex(&editor, cx, "wq");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "x\ny");
        assert_eq!(window_count(cx), 0);
    });
    run(Some(&a), |editor, cx| {
        ex(&editor, cx, "x");
        assert_eq!(
            window_count(cx),
            0,
            ":x closes a clean window without writing"
        );
    });
}

#[test]
fn edit_refuses_to_replace_a_dirty_document() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "edit");
        ex(&editor, cx, &format!("e {}", quoted(&b)));
        assert_eq!(text(&editor, cx), "a\nedit");
        assert!(status(&editor, cx).contains("No write since last change"));
        ex(&editor, cx, &format!("e! {}", quoted(&b)));
        assert_eq!(text(&editor, cx), "b\n");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(b.as_path()));
        assert!(!dirty(&editor, cx));
    });
}

#[test]
fn enew_replaces_a_clean_document_in_the_same_window() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "a\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "dirty");
        ex(&editor, cx, "enew");
        assert_eq!(text(&editor, cx), "a\ndirty", "dirty :enew is refused");
        ex(&editor, cx, "enew!");
        assert_eq!(text(&editor, cx), "");
        assert_eq!(file_path(&editor, cx), None);
        assert!(!dirty(&editor, cx));
        assert_eq!(window_count(cx), 1);
        ex(&editor, cx, "file");
        assert!(
            status(&editor, cx).contains("[No Name]"),
            "{}",
            status(&editor, cx)
        );
    });
}

#[test]
fn line_addresses_move_by_hard_line() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "one\n  two\nthree").unwrap();
    run(Some(&a), |editor, cx| {
        let caret =
            |cx: &mut VisualTestContext| cx.update(|_, cx| editor.read(cx).selection().head.0);
        ex(&editor, cx, "2");
        assert_eq!(caret(cx), 6, "first non-blank of line 2");
        ex(&editor, cx, "$");
        assert_eq!(caret(cx), 10);
        ex(&editor, cx, "999");
        assert_eq!(caret(cx), 10, "past the end clamps to the last line");
        ex(&editor, cx, "0");
        assert_eq!(caret(cx), 0);
        assert!(!dirty(&editor, cx));
    });
}

#[test]
fn set_queries_report_without_changing_values() {
    run(None, |editor, cx| {
        ex(&editor, cx, "set ts=3 ts? et?");
        let message = status(&editor, cx);
        assert!(message.contains("tabstop=3"), "{message}");
        assert!(message.contains("expandtab"), "{message}");
        ex(&editor, cx, "set bogus");
        assert!(status(&editor, cx).contains("unknown option"));
    });
}

#[test]
fn read_inserts_after_the_current_line_as_one_undo_step() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let inserted = directory.path().join("in.txt");
    std::fs::write(&a, "first\nlast\n").unwrap();
    std::fs::write(&inserted, "\u{feff}r1\r\nr2\r\n").unwrap();
    run(Some(&a), |editor, cx| {
        ex(&editor, cx, &format!("r {}", quoted(&inserted)));
        assert_eq!(text(&editor, cx), "first\nr1\nr2\nlast\n");
        assert_eq!(file_path(&editor, cx).as_deref(), Some(a.as_path()));
        cx.simulate_keystrokes("u");
        settle(cx);
        assert_eq!(text(&editor, cx), "first\nlast\n");
        assert!(!dirty(&editor, cx));
        ex(
            &editor,
            cx,
            &format!("r {}", quoted(&directory.path().join("missing"))),
        );
        assert!(status(&editor, cx).contains("Read failed"));
        assert_eq!(text(&editor, cx), "first\nlast\n");
    });
}

#[test]
fn command_text_and_paste_never_reach_the_document() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "keep\n").unwrap();
    run(Some(&a), |editor, cx| {
        let revision = cx.update(|_, cx| editor.read(cx).document().revision());
        cx.simulate_keystrokes(":");
        send_text(&editor, cx, ":");
        send_text(&editor, cx, "w some thing");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".to_owned()));
        cx.simulate_keystrokes("ctrl-v");
        let line = cx.update(|_, cx| editor.read(cx).command_line_text());
        assert_eq!(line.as_deref(), Some("w some thingpasted"));
        cx.simulate_keystrokes("escape");
        settle(cx);
        assert_eq!(text(&editor, cx), "keep\n");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).document().revision()),
            revision
        );
    });
}

#[test]
fn quit_all_refuses_while_any_window_is_dirty() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    std::fs::write(&a, "a\n").unwrap();
    run(Some(&a), |editor, cx| {
        insert(&editor, cx, "z");
        ex(&editor, cx, "qa");
        assert_eq!(window_count(cx), 1);
        assert!(status(&editor, cx).contains("nothing was closed"));
        ex(&editor, cx, "wa");
        assert!(!dirty(&editor, cx));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\nz");
        ex(&editor, cx, "qa");
        assert_eq!(window_count(cx), 0);
    });
}

#[test]
fn write_all_and_quit_all_reach_every_window() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.txt");
    let b = directory.path().join("b.txt");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    run(Some(&a), |editor, cx| {
        let loaded = chaus_io::load(&b).unwrap();
        let second = cx.update(|_, app| {
            app.open_window(gpui::WindowOptions::default(), |_, cx| {
                cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let second_dirty = |cx: &mut VisualTestContext| {
            cx.update(|_, app| {
                second
                    .read(app)
                    .map(|editor| editor.document().is_dirty())
                    .unwrap()
            })
        };
        cx.update(|_, app| {
            second
                .update(app, |editor, window, cx| {
                    gpui::EntityInputHandler::replace_text_in_range(
                        editor, None, "more ", window, cx,
                    );
                })
                .unwrap();
        });
        assert!(second_dirty(cx));
        assert_eq!(cx.windows().len(), 2);

        ex(&editor, cx, "qa");
        assert_eq!(
            cx.windows().len(),
            2,
            "a dirty window blocks :qa everywhere"
        );

        ex(&editor, cx, "wa");
        cx.run_until_parked();
        assert!(!second_dirty(cx));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "more b\n");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "a\n",
            "clean A untouched"
        );

        ex(&editor, cx, "qa");
        assert_eq!(cx.windows().len(), 0);
    });
}
