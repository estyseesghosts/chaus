//! Chaus ↔ Silvestris consumer boundary: File → Open through the real panel.

use chaus_core::settings::Settings;
use chaus_editor::{Editor, FileDialogs};
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement, Render,
    TestAppContext, VisualContext as _, VisualTestContext, Window,
};
use silvestris::command::CommandRegistry;
use silvestris::core::command::CommandId;

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

fn app() -> (Entity<Editor>, AnyWindowHandle, VisualTestContext) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        cx.set_reduce_motion(true);
        chaus_editor::bind_keys(cx);
        chaus_app::app::install_settings(cx, Settings::default());
        FileDialogs::install(cx, FileDialogs::ephemeral());
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, cx| harness.read(cx).editor.clone());
    cx.update(|_, cx| {
        chaus_app::app::register_app_commands(&editor, cx);
        let commands = editor.read(cx).commands().clone();
        silvestris::command::apply_bindings(&commands, &[chaus_editor::KEY_CONTEXT], cx)
            .expect("bind document and application commands");
    });
    let mut cx = cx.clone();
    draw(&mut cx);
    let editor = cx.update(|_, cx| harness.read(cx).editor.clone());
    let owner = cx.window_handle();
    (editor, owner, cx)
}

fn invoke(editor: &Entity<Editor>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        let commands = editor.read(cx).commands().clone();
        CommandRegistry::invoke(&commands, &CommandId::new(id), window, cx);
    });
    draw(cx);
}

fn open_window(
    cx: &VisualTestContext,
    owner: AnyWindowHandle,
) -> (AnyWindowHandle, VisualTestContext) {
    let handle = cx
        .cx
        .windows()
        .into_iter()
        .find(|window| *window != owner)
        .expect("the Open command presents its own application-modal window");
    let context = VisualTestContext::from_window(handle, &cx.cx);
    (handle, context)
}

#[test]
fn cancelling_the_open_panel_opens_nothing() {
    let (editor, owner, mut cx) = app();
    invoke(&editor, &mut cx, "file.open");
    let (_handle, mut panel_cx) = open_window(&cx, owner);
    let panel = cx
        .update(|_, cx| cx.global::<FileDialogs>().last_panel())
        .expect("OpenPanel entity");
    panel_cx.update(|window, cx| {
        panel.update(cx, |panel, cx| panel.close(window, cx));
    });
    // The Open window is gone once the modal result is delivered.
    draw(&mut cx);
    assert_eq!(
        cx.cx.windows().len(),
        1,
        "Cancel must close the Open window and create no document window"
    );
}

#[test]
fn accepting_a_unicode_file_opens_it_in_its_own_window() {
    let directory = tempfile::tempdir().unwrap();
    let name = "nöte-文件.txt";
    let path = directory.path().join(name);
    std::fs::write(&path, "hello\n").unwrap();

    let (editor, owner, mut cx) = app();
    invoke(&editor, &mut cx, "file.open");
    let (_open_handle, mut panel_cx) = open_window(&cx, owner);
    let panel = cx
        .update(|_, cx| cx.global::<FileDialogs>().last_panel())
        .expect("OpenPanel entity");
    let dialogs = cx.update(|_, cx| cx.global::<FileDialogs>().clone());
    let location = dialogs
        .provider()
        .location_of(directory.path())
        .expect("the temporary directory is a host location");
    panel_cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .model()
                .update(cx, |model, cx| model.navigate(location, cx));
        });
    });
    draw(&mut panel_cx);
    let entry = panel_cx.update(|_, cx| {
        panel
            .read(cx)
            .model()
            .read(cx)
            .controller()
            .read(cx)
            .visible_items()
            .into_iter()
            .find(|entry| entry.name == name)
            .cloned()
    });
    let entry = entry.expect("the Unicode file is listed exactly");
    panel_cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.select_entry(&entry.id, cx);
            panel.submit(cx);
        });
    });
    // Accepting closes the modal window; the load and new document window are
    // driven by the owner application.
    draw(&mut cx);

    // The opened document window's root view is the editor itself; the test
    // harness wrapper only exists for the original window.
    let opened: Vec<String> = cx
        .cx
        .windows()
        .into_iter()
        .filter_map(|window| {
            window
                .downcast::<Editor>()
                .and_then(|handle| handle.read_with(&cx.cx, |editor, _| editor.title()).ok())
        })
        .collect();
    assert!(
        opened.iter().any(|title| title == name),
        "accepted file reaches chaus_io and gets its own window: {opened:?}"
    );
}
