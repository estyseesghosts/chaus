//! Save As through the real Silvestris SavePanel and Chaus's save engine.

use std::path::Path;
use std::time::Duration;

use chaus_core::settings::Settings;
use chaus_editor::{Editor, FileDialogs};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, Render,
    TestAppContext, VisualContext as _, VisualTestContext, Window,
};
use silvestris::file_panel::FilePanel;

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
    cx.run_until_parked();
}

/// Arm an asynchronous delay for the write to `target`, accept the sheet, and
/// draw until the save is waiting on its timer. The snapshot is already taken
/// and the write is parked, so the caller can edit or create a competing file.
fn accept_with_delayed_write(
    panel: &Entity<FilePanel>,
    editor: &Entity<Editor>,
    directory: &Path,
    name: &str,
    target: &Path,
    cx: &mut VisualTestContext,
) {
    prepare_sheet(panel, directory, name, cx);
    chaus_editor::set_save_delay_for_test(target.to_path_buf(), Duration::from_secs(30));
    submit(panel, cx);
    draw(cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned)),
        Some("Saving...".to_string()),
        "the accepted save is waiting on the test delay"
    );
}

/// Advance the test clock past the delay and let the write finish. The delay
/// entry is consumed by the write itself, so this cannot affect another test.
fn finish_delayed_write(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(31));
    draw(cx);
}

fn open(path: &Path) -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        cx.set_reduce_motion(true);
        chaus_editor::bind_keys(cx);
    });
    let loaded = chaus_io::load(path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, cx| harness.read(cx).editor.clone());
    draw(cx);
    (editor, cx.clone())
}

fn untitled() -> (Entity<Editor>, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        cx.set_reduce_motion(true);
        chaus_editor::bind_keys(cx);
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, cx| harness.read(cx).editor.clone());
    draw(cx);
    (editor, cx.clone())
}

fn invoke_save_as(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let commands = editor.read(cx).commands().clone();
        silvestris::command::CommandRegistry::invoke(
            &commands,
            &silvestris::core::command::CommandId::new("file.save_as"),
            window,
            cx,
        );
    });
    draw(cx);
}

fn last_panel(cx: &mut VisualTestContext) -> Entity<FilePanel> {
    cx.update(|_, cx| {
        cx.global::<FileDialogs>()
            .last_panel()
            .expect("the command presented a SavePanel")
    })
}

/// Point the open Save sheet at `directory` and name it, without accepting.
fn prepare_sheet(
    panel: &Entity<FilePanel>,
    directory: &Path,
    name: &str,
    cx: &mut VisualTestContext,
) {
    let dialogs = cx.update(|_, cx| cx.global::<FileDialogs>().clone());
    let location = dialogs
        .provider()
        .location_of(directory)
        .expect("the provider interns the directory");
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .model()
                .update(cx, |model, cx| model.navigate(location, cx));
        });
    });
    draw(cx);
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.set_name(name, window, cx));
    });
    draw(cx);
}

fn submit(panel: &Entity<FilePanel>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| panel.submit(cx));
    });
}

/// Point the open Save sheet at `directory`, name it, and press Save.
fn choose(panel: &Entity<FilePanel>, directory: &Path, name: &str, cx: &mut VisualTestContext) {
    prepare_sheet(panel, directory, name, cx);
    submit(panel, cx);
    draw(cx);
}

fn alert_window(cx: &VisualTestContext, owner: gpui::AnyWindowHandle) -> VisualTestContext {
    let candidates: Vec<_> = cx
        .cx
        .windows()
        .into_iter()
        .filter(|window| *window != owner)
        .collect();
    let handle = candidates
        .iter()
        .find(|window| window.root_entity_type_name().contains("AlertWindow"))
        .copied()
        .or_else(|| candidates.first().copied())
        .expect("the replace alert is its own application-modal window");
    VisualTestContext::from_window(handle, &cx.cx)
}

fn alert_frame(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn replace_in_alert(cx: &VisualTestContext, owner: gpui::AnyWindowHandle) {
    let mut alert = alert_window(cx, owner);
    alert_frame(&mut alert);
    let replace = alert.debug_bounds("alert-replace").expect("Replace button");
    // The alert window closes as part of delivering its result.
    alert.simulate_click(replace.center(), Modifiers::default());
}

#[test]
fn saving_a_named_document_writes_without_a_panel() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (_editor, mut cx) = open(&path);
    cx.simulate_input("X");
    draw(&mut cx);
    cx.simulate_keystrokes("ctrl-s");
    draw(&mut cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xone\n");
    assert!(
        cx.update(|_, cx| cx.try_global::<FileDialogs>().is_none()
            || cx.global::<FileDialogs>().last_panel().is_none()),
        "Save on a named document must not present Save As"
    );
}

#[test]
fn save_as_new_writes_the_target_and_associates_it() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let target = directory.path().join("fresh-name.txt");
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);
    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    choose(&panel, directory.path(), "fresh-name.txt", &mut cx);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Xone\n");
    assert_eq!(cx.update(|_, cx| editor.read(cx).title()), "fresh-name.txt");
}

#[test]
fn save_as_replace_asks_first_and_then_overwrites() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let target = directory.path().join("target.txt");
    std::fs::write(&source, "one\n").unwrap();
    std::fs::write(&target, "old\n").unwrap();
    let (editor, mut cx) = open(&source);
    let owner = cx.window_handle();
    cx.simulate_input("X");
    draw(&mut cx);
    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    choose(&panel, directory.path(), "target.txt", &mut cx);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "old\n",
        "nothing is written before Replace is confirmed"
    );
    replace_in_alert(&cx, owner);
    draw(&mut cx);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Xone\n");
    assert_eq!(cx.update(|_, cx| editor.read(cx).title()), "target.txt");
}

#[test]
fn cancelling_the_replace_alert_returns_to_the_sheet() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    let target = directory.path().join("target.txt");
    std::fs::write(&source, "one\n").unwrap();
    std::fs::write(&target, "old\n").unwrap();
    let (editor, mut cx) = open(&source);
    let owner = cx.window_handle();
    cx.simulate_input("X");
    draw(&mut cx);
    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    choose(&panel, directory.path(), "target.txt", &mut cx);

    let mut alert = alert_window(&cx, owner);
    alert_frame(&mut alert);
    let cancel = alert.debug_bounds("alert-cancel").expect("alert Cancel");
    // The alert window closes as part of delivering its result.
    alert.simulate_click(cancel.center(), Modifiers::default());
    draw(&mut cx);
    assert!(
        cx.debug_bounds("file-panel").is_some(),
        "Cancel on the alert returns to the Save sheet"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");

    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.close(window, cx));
    });
    draw(&mut cx);
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned)),
        Some("Save cancelled".to_string())
    );
    assert!(cx.update(|_, cx| editor.read(cx).document().is_dirty()));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "old\n");
}

#[test]
fn cancelling_the_save_sheet_keeps_the_document_and_file_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);
    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.close(window, cx));
    });
    draw(&mut cx);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "one\n");
    assert!(cx.update(|_, cx| editor.read(cx).document().is_dirty()));
}

#[test]
fn dirty_close_save_then_panel_cancel_restores_the_close_prompt() {
    let (editor, mut cx) = untitled();
    cx.simulate_input("draft");
    draw(&mut cx);
    let allowed =
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
    assert!(!allowed);
    draw(&mut cx);
    let save = cx.debug_bounds("close-save").expect("Save button");
    cx.simulate_click(save.center(), Modifiers::default());
    draw(&mut cx);
    assert!(
        cx.debug_bounds("chaus-close-sheet").is_none(),
        "the dirty-close prompt is dismissed before Save As"
    );
    let panel = last_panel(&mut cx);
    assert_eq!(cx.update(|_, cx| panel.read(cx).name_text(cx)), "Untitled");
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.close(window, cx));
    });
    draw(&mut cx);
    assert!(
        cx.debug_bounds("chaus-close-sheet").is_some(),
        "cancelling Save As restores the original close decision"
    );
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).document().text()),
        "draft"
    );
}

#[test]
fn an_edit_made_while_the_write_is_in_flight_stays_dirty() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);

    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    accept_with_delayed_write(
        &panel,
        &editor,
        directory.path(),
        "inflight.txt",
        &directory.path().join("inflight.txt"),
        &mut cx,
    );
    // Edit through the input handler because the sheet may still own keyboard
    // focus at this instant.
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, "Y", window, cx);
        });
    });
    finish_delayed_write(&mut cx);

    assert_eq!(
        std::fs::read_to_string(directory.path().join("inflight.txt")).unwrap(),
        "Xone\n",
        "the write covers the snapshot taken at acceptance"
    );
    let (text, dirty) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.document().text(), editor.document().is_dirty())
    });
    assert_eq!(text, "XYone\n", "the later edit stays in the document");
    assert!(
        dirty,
        "a write of an older snapshot must leave the document dirty"
    );
}

#[test]
fn the_three_browser_views_and_the_last_view_persist_across_sheets() {
    use silvestris::file_panel::FileViewMode;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);
    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    for view in [
        FileViewMode::List,
        FileViewMode::Icons,
        FileViewMode::Columns,
    ] {
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| panel.set_view(view, cx));
        });
        draw(&mut cx);
        assert_eq!(
            cx.update(|_, cx| panel.read(cx).model().read(cx).view()),
            view
        );
    }
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| panel.set_view(FileViewMode::List, cx));
    });
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.close(window, cx));
    });
    draw(&mut cx);

    invoke_save_as(&editor, &mut cx);
    let reopened = last_panel(&mut cx);
    assert_eq!(
        cx.update(|_, cx| reopened.read(cx).model().read(cx).view()),
        FileViewMode::List,
        "the state store reopens in the last-used view"
    );
}

#[test]
fn a_target_created_between_acceptance_and_write_is_not_clobbered() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let target = directory.path().join("racy.txt");
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);

    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    accept_with_delayed_write(
        &panel,
        &editor,
        directory.path(),
        "racy.txt",
        &target,
        &mut cx,
    );
    // The panel accepted this target as new; another process creates it before
    // Chaus commits.
    std::fs::write(&target, "other process\n").unwrap();
    finish_delayed_write(&mut cx);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "other process\n",
        "a New target that appeared later is not overwritten"
    );
    assert!(
        cx.update(|_, cx| editor.read(cx).document().is_dirty()),
        "the failed save keeps the document dirty"
    );
    let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned));
    assert!(
        status
            .as_deref()
            .is_some_and(|status| status.starts_with("Save failed")),
        "status: {status:?}"
    );
}

#[test]
fn new_folder_from_the_save_sheet_is_a_real_directory_chaus_can_write_into() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.txt");
    std::fs::write(&source, "one\n").unwrap();
    let (editor, mut cx) = open(&source);
    cx.simulate_input("X");
    draw(&mut cx);

    invoke_save_as(&editor, &mut cx);
    let panel = last_panel(&mut cx);
    let dialogs = cx.update(|_, cx| cx.global::<FileDialogs>().clone());
    let location = dialogs
        .provider()
        .location_of(directory.path())
        .expect("the provider interns the temporary directory");
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .model()
                .update(cx, |model, cx| model.navigate(location, cx));
        });
    });
    draw(&mut cx);

    // Save starts in the minimal form; the browser and New Folder belong to
    // the expanded state.
    if !cx.update(|_, cx| panel.read(cx).expanded()) {
        let disclosure = cx.debug_bounds("save-disclosure").expect("save disclosure");
        cx.simulate_click(disclosure.center(), Modifiers::default());
        draw(&mut cx);
    }
    let new_folder = cx
        .debug_bounds("panel-new-folder")
        .expect("New Folder in the expanded browser");
    cx.simulate_click(new_folder.center(), Modifiers::default());
    draw(&mut cx);

    let created = directory.path().join("untitled folder");
    assert!(
        created.is_dir(),
        "New Folder creates a real directory through StdFsProvider"
    );
    assert!(
        cx.update(|_, cx| panel.read(cx).is_editing(cx)),
        "the new row starts an inline rename"
    );
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert!(!cx.update(|_, cx| panel.read(cx).is_editing(cx)));

    // Chaus writes into the folder the panel just created.
    let inner = dialogs
        .provider()
        .location_of(&created)
        .expect("the provider interns the new directory");
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .model()
                .update(cx, |model, cx| model.navigate(inner, cx));
        });
    });
    draw(&mut cx);
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.set_name("inside.txt", window, cx));
    });
    draw(&mut cx);
    submit(&panel, &mut cx);
    draw(&mut cx);

    assert_eq!(
        std::fs::read_to_string(created.join("inside.txt")).unwrap(),
        "Xone\n"
    );
    assert_eq!(
        cx.update(|_, cx| editor.read(cx).title()),
        "inside.txt",
        "the document adopts the path inside the new folder"
    );
}

#[test]
fn closing_a_dirty_named_document_saves_once_and_closes_the_window() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("closing.txt");
    std::fs::write(&path, "one\n").unwrap();
    let (editor, mut cx) = open(&path);
    cx.simulate_input("X");
    draw(&mut cx);

    let allowed =
        cx.update(|window, cx| editor.update(cx, |editor, cx| editor.confirm_close(window, cx)));
    assert!(!allowed, "a dirty document asks before closing");
    draw(&mut cx);
    let save = cx.debug_bounds("close-save").expect("close sheet Save");
    cx.simulate_click(save.center(), Modifiers::default());
    cx.run_until_parked();

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Xone\n",
        "the close-save writes the current document"
    );
    assert!(
        cx.cx.windows().is_empty(),
        "a successful close-save removes exactly its own window"
    );
}
