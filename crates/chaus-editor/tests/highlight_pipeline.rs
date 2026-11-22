//! Coalescing highlight worker: latest work wins, accepted trees stay usable.

use chaus_core::position::DocumentRevision;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_highlight::LanguageId;
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
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..8 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        draw(cx);
    }
}

const SOURCE: &str = "fn main() {\n    let name = \"chaus\";\n    println!(\"{name}\");\n}\n";

fn open(source: &str, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pipeline.rs");
    std::fs::write(&path, source).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), Settings::default()));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn current_state(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> (DocumentRevision, Option<DocumentRevision>) {
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        (editor.document().revision(), editor.syntax_revision())
    })
}

#[test]
fn a_burst_of_edits_parses_the_latest_revision_once() {
    open(SOURCE, |editor, cx| {
        settle(cx);
        let before = cx.update(|_, cx| editor.read(cx).highlight_statistics());

        for _ in 0..100 {
            cx.simulate_input("x");
        }
        // No clock advance between edits: all 100 generations are pending when
        // the worker wakes, so only the newest may reach the parser.
        settle(cx);

        let (revision, spans_revision) = current_state(&editor, cx);
        assert_eq!(
            spans_revision,
            Some(revision),
            "the painted spans must belong to the final document revision"
        );
        let stats = cx.update(|_, cx| editor.read(cx).highlight_statistics());
        let parses = stats.incremental_parses + stats.full_parses;
        let parses_before = before.incremental_parses + before.full_parses;
        assert!(
            parses - parses_before <= 3,
            "100 edits must not schedule 100 parses; stats: {stats:?}"
        );
        assert!(
            stats.coalesced >= 90,
            "most generations should be skipped before parsing; stats: {stats:?}"
        );
        assert!(
            stats.published > before.published,
            "the latest parse must be published"
        );
        assert!(
            !cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()),
            "the final revision must still be highlighted"
        );
    });
}

#[test]
fn paused_edits_reuse_the_accepted_tree_and_match_a_full_parse() {
    open(SOURCE, |editor, cx| {
        settle(cx);
        let before = cx.update(|_, cx| editor.read(cx).highlight_statistics());

        cx.simulate_input("x");
        settle(cx);
        let after_first = cx.update(|_, cx| editor.read(cx).highlight_statistics());
        assert!(
            after_first.incremental_parses > before.incremental_parses,
            "a settled edit should reuse the accepted tree; stats: {after_first:?}"
        );

        cx.simulate_input("y");
        settle(cx);
        let after_second = cx.update(|_, cx| editor.read(cx).highlight_statistics());
        assert!(
            after_second.incremental_parses > after_first.incremental_parses,
            "the second settled edit should also reuse its tree; stats: {after_second:?}"
        );

        let (revision, language, rope, spans) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.document().revision(),
                editor.language(),
                editor.document().rope().clone(),
                editor.syntax_spans().to_vec(),
            )
        });
        let fresh = chaus_highlight::parse_snapshot(language, &rope, None, None, revision)
            .expect("fresh Rust parse");
        assert_eq!(
            spans,
            fresh.highlight.spans.as_slice(),
            "incremental results must equal a full parse of the same revision"
        );
    });
}

#[test]
fn undo_and_language_changes_reparse_and_plain_text_disables_colour() {
    open(SOURCE, |editor, cx| {
        settle(cx);
        let before = cx.update(|_, cx| editor.read(cx).highlight_statistics());

        cx.simulate_input("x");
        settle(cx);
        cx.simulate_keystrokes("ctrl-z");
        settle(cx);
        let (revision, spans_revision) = current_state(&editor, cx);
        assert_eq!(
            spans_revision,
            Some(revision),
            "undo must schedule a parse for the restored revision"
        );
        let after_undo = cx.update(|_, cx| editor.read(cx).highlight_statistics());
        assert!(
            after_undo.full_parses > before.full_parses,
            "history operations pass no incremental edit; stats: {after_undo:?}"
        );

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_language(LanguageId::PlainText, cx);
            });
        });
        settle(cx);
        let plain = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (editor.syntax_spans().is_empty(), editor.syntax_revision())
        });
        assert!(plain.0, "Plain Text must have no spans");
        assert_eq!(plain.1, Some(revision));

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_language(LanguageId::Rust, cx);
            });
        });
        settle(cx);
        let (revision, spans_revision) = current_state(&editor, cx);
        assert_eq!(spans_revision, Some(revision));
        assert!(
            !cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()),
            "switching back to Rust must highlight again"
        );
    });
}
