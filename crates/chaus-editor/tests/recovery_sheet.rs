use chaus_core::eol::Eol;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::recovery::RecoveryRecord;
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
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[test]
fn a_recovery_snapshot_can_be_recovered_or_discarded() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
    });
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, None, Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    let record = RecoveryRecord::new(
        41,
        Some(std::path::PathBuf::from("C:/notes/example.txt")),
        "one\r\ntwo\r\n",
        Eol::CrLf,
        false,
        3,
    );
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.queue_recovery(vec![record.clone()], window, cx);
        });
    });
    draw(cx);
    assert!(
        cx.debug_bounds("chaus-recovery-sheet").is_some(),
        "the launch recovery sheet must be visible"
    );

    let recover = cx.debug_bounds("recovery-recover").unwrap();
    cx.simulate_click(recover.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "one\ntwo\n");
        assert!(editor.read(cx).document().is_dirty());
    });

    let discard = RecoveryRecord::new(42, None, "discard me", Eol::Lf, false, 1);
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.queue_recovery(vec![discard], window, cx);
        });
    });
    draw(cx);
    let discard = cx.debug_bounds("recovery-discard").unwrap();
    cx.simulate_click(discard.center(), Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("chaus-recovery-sheet").is_none());
    cx.update(|_, cx| {
        assert_eq!(editor.read(cx).document().text(), "one\ntwo\n");
    });
}
