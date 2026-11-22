//! Transient status lifetimes and stale-timer protection.

use std::time::Duration;

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

/// Tick only runnable tasks. `run_until_parked` would jump the clock to the
/// next timer, which would immediately dismiss the message under test.
fn settle(cx: &mut VisualTestContext) {
    while cx.executor().tick() {}
}

fn status(cx: &mut VisualTestContext, editor: &Entity<Editor>) -> Option<String> {
    cx.update(|_, cx| editor.read(cx).status_text().map(str::to_owned))
}

#[test]
fn a_routine_message_disappears_but_a_failure_does_not() {
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
    settle(cx);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.show_transient_message("Saved", cx);
        });
    });
    assert_eq!(
        status(cx, &editor).as_deref(),
        Some("Saved"),
        "the transient message must be visible immediately"
    );

    cx.executor().advance_clock(Duration::from_secs(5));
    settle(cx);
    assert_eq!(
        status(cx, &editor),
        None,
        "Saved must not remain as permanent decoration"
    );

    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.show_message("Save failed: disk full");
        });
    });
    cx.executor().advance_clock(Duration::from_secs(30));
    settle(cx);
    assert_eq!(
        status(cx, &editor).as_deref(),
        Some("Save failed: disk full"),
        "an unresolved failure must remain visible"
    );
}

#[test]
fn a_stale_timer_cannot_clear_a_newer_message() {
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
    settle(cx);

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| {
            editor.show_transient_message("Saved", cx);
        });
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    settle(cx);
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| {
            editor.show_message("Save failed: disk full");
        });
    });
    cx.executor().advance_clock(Duration::from_secs(10));
    settle(cx);
    assert_eq!(
        status(cx, &editor).as_deref(),
        Some("Save failed: disk full"),
        "the timer armed for Saved must not clear the later failure"
    );
}
