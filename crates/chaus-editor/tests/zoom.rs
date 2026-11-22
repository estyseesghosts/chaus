//! Per-window Ctrl+Plus/Minus zoom.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{AppContext as _, Entity, TestAppContext};

fn new_editor(app: &mut TestAppContext, font_size: f32) -> Entity<Editor> {
    app.update(|cx| {
        let mut settings = Settings::default();
        settings.font_size = font_size;
        cx.new(|cx| Editor::new(cx, None, settings))
    })
}

#[test]
fn steps_through_the_documented_scale_and_clamps() {
    let mut app = TestAppContext::single();
    let editor = new_editor(&mut app, 14.0);

    app.update(|cx| {
        editor.update(cx, |editor, cx| editor.zoom_in(cx));
    });
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 15.0);
    app.update(|cx| {
        editor.update(cx, |editor, cx| editor.zoom_in(cx));
    });
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 16.0);

    app.update(|cx| {
        editor.update(cx, |editor, cx| editor.zoom_out(cx));
    });
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 15.0);

    for _ in 0..80 {
        app.update(|cx| {
            editor.update(cx, |editor, cx| editor.zoom_out(cx));
        });
    }
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 9.0);

    for _ in 0..80 {
        app.update(|cx| {
            editor.update(cx, |editor, cx| editor.zoom_in(cx));
        });
    }
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 48.0);
}

#[test]
fn a_configured_size_between_stops_steps_to_the_next_whole_stop() {
    let mut app = TestAppContext::single();
    let editor = new_editor(&mut app, 15.5);

    app.update(|cx| {
        editor.update(cx, |editor, cx| editor.zoom_in(cx));
    });
    assert_eq!(app.update(|cx| editor.read(cx).editor_font_size()), 16.0);

    let stepped_down = new_editor(&mut app, 15.5);
    app.update(|cx| {
        stepped_down.update(cx, |editor, cx| editor.zoom_out(cx));
    });
    assert_eq!(
        app.update(|cx| stepped_down.read(cx).editor_font_size()),
        15.0
    );
}

#[test]
fn zoom_is_per_window_and_reset_returns_to_the_configured_default() {
    let mut app = TestAppContext::single();
    let left = new_editor(&mut app, 14.0);
    let right = new_editor(&mut app, 14.0);

    app.update(|cx| {
        left.update(cx, |editor, cx| {
            editor.zoom_in(cx);
            editor.zoom_in(cx);
            editor.zoom_in(cx);
            editor.zoom_in(cx);
        });
    });
    assert_eq!(app.update(|cx| left.read(cx).editor_font_size()), 18.0);
    assert_eq!(
        app.update(|cx| right.read(cx).editor_font_size()),
        14.0,
        "zooming one window must not touch another"
    );

    app.update(|cx| {
        left.update(cx, |editor, cx| editor.reset_zoom(cx));
    });
    assert_eq!(app.update(|cx| left.read(cx).editor_font_size()), 14.0);
    assert_eq!(app.update(|cx| right.read(cx).editor_font_size()), 14.0);
}
