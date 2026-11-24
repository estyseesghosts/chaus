//! Width changes are coalesced and published only when still current.

use std::sync::mpsc;

use chaus_core::settings::Settings;
use chaus_editor::Editor;
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

/// Settle background work without repainting: a paint recomputes the wrap width
/// from the window and would race the explicitly requested width.
fn settle(cx: &VisualTestContext) {
    for _ in 0..6 {
        cx.run_until_parked();
    }
}

fn expected_rows(source: &str, columns: usize) -> usize {
    source
        .lines()
        .map(|line| {
            chaus_layout::row_count_with(line, Some(columns), 4, chaus_layout::WrapPolicy::Word)
        })
        .sum::<usize>()
        // A trailing newline leaves one empty hard line in the Rope.
        + 1
}

fn open(source: &str) -> (Entity<Editor>, VisualTestContext, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("notes.txt");
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
    cx.update(|_, cx| {
        editor.update(cx, |editor, _| editor.lock_wrap_width_for_test());
    });
    draw(&mut *cx);
    (editor, cx.clone(), directory)
}

#[test]
fn a_width_change_is_published_only_after_background_measurement() {
    let source = "one two three four five six seven eight nine ten\n".repeat(40);
    let (editor, mut cx, _directory) = open(&source);
    let (before, before_generation, before_publications) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.committed_wrap_columns(),
            editor.reflow_generation(),
            editor.reflow_publications(),
        )
    });

    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_wrap_columns(Some(18), cx));
    });
    let (committed, in_flight, generation) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.committed_wrap_columns(),
            editor.is_reflowing(),
            editor.reflow_generation(),
        )
    });
    assert_eq!(
        committed, before,
        "the old layout stays committed while measuring"
    );
    assert!(in_flight);
    assert_eq!(generation, before_generation + 1);

    settle(&cx);
    let (committed, publications) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.committed_wrap_columns(),
            editor.reflow_publications(),
        )
    });
    let rows = cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count()));
    assert_eq!(committed, Some(18));
    assert_eq!(publications, before_publications + 1);
    assert_eq!(rows, expected_rows(&source, 18));
}

#[test]
fn rapid_width_changes_publish_only_the_last_one() {
    let source = "alpha beta gamma delta epsilon zeta eta theta\n".repeat(30);
    let (editor, mut cx, _directory) = open(&source);
    let (before_generation, before_publications) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.reflow_generation(), editor.reflow_publications())
    });
    for columns in [12, 16, 24] {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.set_wrap_columns(Some(columns), cx));
        });
    }
    settle(&cx);
    let (committed, publications, generation) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.committed_wrap_columns(),
            editor.reflow_publications(),
            editor.reflow_generation(),
        )
    });
    let rows = cx.update(|_, cx| editor.update(cx, |editor, _| editor.visual_row_count()));
    assert_eq!(committed, Some(24));
    assert_eq!(
        publications,
        before_publications + 1,
        "intermediate widths must not be published"
    );
    assert_eq!(generation, before_generation + 3);
    assert_eq!(rows, expected_rows(&source, 24));
}

#[test]
fn a_document_edit_while_measuring_forces_a_fresh_measurement() {
    let source = "one two three four five six seven\n".repeat(30);
    let (editor, mut cx, _directory) = open(&source);
    let (release, gate) = mpsc::channel();
    chaus_editor::set_reflow_gate_for_test(Some(gate));
    let before_publications = cx.update(|_, cx| editor.read(cx).reflow_publications());
    cx.update(|_, cx| {
        editor.update(cx, |editor, cx| editor.set_wrap_columns(Some(14), cx));
    });
    let revision_before = cx.update(|_, cx| editor.read(cx).document().revision());
    cx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("X").expect("valid keystroke"), cx);
    });
    let revision_after = cx.update(|_, cx| editor.read(cx).document().revision());
    assert_ne!(
        revision_after, revision_before,
        "the test edit must change the document revision"
    );
    release.send(()).expect("reflow task is still waiting");
    chaus_editor::set_reflow_gate_for_test(None);
    settle(&cx);

    let (committed, publications) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (
            editor.committed_wrap_columns(),
            editor.reflow_publications(),
        )
    });
    let (text, dirty) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.document().text(), editor.document().is_dirty())
    });
    assert_eq!(committed, Some(14));
    assert_eq!(
        publications,
        before_publications + 1,
        "a stale measurement must not publish a second layout"
    );
    assert!(
        text.starts_with('X'),
        "the edit while measuring must survive"
    );
    assert!(dirty, "the edited document stays dirty");
}
