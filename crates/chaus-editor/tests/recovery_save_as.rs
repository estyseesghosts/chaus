//! A recovered document keeps its original Save As suggestion and BOM.

use chaus_core::eol::Eol;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::recovery::{self, RecoveryRecord};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, Render,
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

fn send_text(editor: &Entity<Editor>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::replace_text_in_range(editor, None, text, window, cx);
        });
    });
}

#[test]
fn save_as_keeps_the_recovered_path_and_bom() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.txt");
    let target = directory.path().join("resaved.txt");
    let record = RecoveryRecord::new(71, Some(original.clone()), "hello\n", Eol::Lf, true, 4);
    recovery::write_to(directory.path(), &record).unwrap();

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
        settings.vim = true;
        let editor = cx.new(|cx| Editor::new(cx, None, settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);

    let entries = recovery::scan_in_without_age(directory.path());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            assert!(editor.queue_recovery(entries, window, cx));
        });
    });
    draw(cx);
    let recover = cx.debug_bounds("recovery-recover").unwrap();
    cx.simulate_click(recover.center(), Modifiers::default());
    draw(cx);

    cx.update(|_, cx| {
        let (suggested, bom) = editor.read(cx).save_as_suggestion_for_test();
        assert_eq!(
            suggested.as_deref(),
            Some(original.as_path()),
            "Save As must still suggest the recovered original path"
        );
        assert!(bom, "Save As must see the recovered BOM state");
    });

    // Exercise the real Ex path, which also consults the recovered metadata.
    cx.simulate_keystrokes(":");
    send_text(&editor, cx, ":");
    send_text(&editor, cx, &format!("saveas {}", target.display()));
    cx.simulate_keystrokes("enter");
    draw(cx);

    let bytes = std::fs::read(&target).unwrap();
    assert_eq!(
        &bytes[..3],
        &[0xEF, 0xBB, 0xBF],
        "a recovered UTF-8-BOM document must keep its BOM when saved again"
    );
    assert_eq!(&bytes[3..], b"hello\n");
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).document_path(),
            Some(target.as_path()),
            "a successful Save As associates the new file"
        );
    });
    assert!(
        recovery::scan_in_without_age(directory.path()).is_empty(),
        "saving a recovered document retires the snapshot from the store it came from"
    );
}
