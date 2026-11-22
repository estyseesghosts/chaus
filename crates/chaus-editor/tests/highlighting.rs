//! Background syntax highlighting: revision tagging, overrides and fallback.

use chaus_core::position::CharOffset;
use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_highlight::{LanguageId, TokenKind};
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
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }
}

fn open(path: &std::path::Path, test: impl FnOnce(Entity<Editor>, &mut VisualTestContext)) {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let loaded = chaus_io::load(path).unwrap();
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

fn fixture(name: &str, source: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(name), source).unwrap();
    directory
}

#[test]
fn a_rust_file_is_detected_and_highlighted_after_the_background_parse() {
    let directory = fixture("main.rs", "fn main() { let text = \"hi\"; }\n");
    open(&directory.path().join("main.rs"), |editor, cx| {
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).language()),
            LanguageId::Rust
        );
        assert!(
            cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()),
            "nothing is paintable before the worker finishes"
        );
        settle(cx);
        let spans = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.syntax_spans().len(),
                editor.syntax_spans().to_vec(),
                editor.document().len_chars(),
                editor.syntax_revision(),
                editor.document().revision(),
            )
        });
        assert!(spans.0 > 0, "Rust must produce spans");
        assert_eq!(spans.3, Some(spans.4), "spans match the document revision");
        assert!(
            spans.1.iter().any(|span| span.kind == TokenKind::Keyword),
            "fn is a keyword: {:?}",
            spans.1
        );
        assert!(spans.1.iter().all(|span| span.range.end <= spans.2));
    });
}

#[test]
fn stale_spans_are_never_painted_after_an_edit() {
    let directory = fixture("main.rs", "fn main() {}\n");
    open(&directory.path().join("main.rs"), |editor, cx| {
        settle(cx);
        assert!(!cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()));
        cx.simulate_input("let value = 42;\n");
        assert!(
            cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()),
            "spans from the previous revision must be hidden immediately"
        );
        settle(cx);
        let (spans, revision, current, length) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.syntax_spans().to_vec(),
                editor.syntax_revision(),
                editor.document().revision(),
                editor.document().len_chars(),
            )
        });
        assert_eq!(revision, Some(current));
        assert!(spans.iter().all(|span| span.range.end <= length));
        assert!(!spans.is_empty());
    });
}

#[test]
fn the_plain_text_override_stops_and_resumes_highlighting() {
    let directory = fixture("notes.txt", "fn main() { let text = \"hi\"; }\n");
    open(&directory.path().join("notes.txt"), |editor, cx| {
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).language()),
            LanguageId::PlainText
        );
        assert!(cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()));

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.set_language(LanguageId::Rust, cx));
        });
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).language()),
            LanguageId::Rust
        );
        assert!(cx.update(|_, cx| !editor.read(cx).syntax_spans().is_empty()));

        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.clear_language_override(cx));
        });
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).language()),
            LanguageId::PlainText
        );
        assert!(cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()));
    });
}

#[test]
fn undo_redo_and_unicode_keep_spans_inside_the_document() {
    let directory = fixture("main.rs", "// é\nfn main() { }\n");
    open(&directory.path().join("main.rs"), |editor, cx| {
        settle(cx);
        let prefix = CharOffset("// é\n".chars().count());
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_selection(chaus_core::position::Selection::caret(prefix), cx);
            });
        });
        cx.simulate_input("let x = 1;\n");
        settle(cx);
        cx.simulate_keystrokes("ctrl-z");
        settle(cx);
        let (spans, length) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.syntax_spans().to_vec(),
                editor.document().len_chars(),
            )
        });
        assert!(
            spans.iter().all(|span| span.range.end <= length),
            "undo must not leave out-of-range spans"
        );
        assert!(!spans.is_empty());

        cx.simulate_keystrokes("ctrl-shift-z");
        settle(cx);
        let spans = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.syntax_spans().to_vec(),
                editor.document().len_chars(),
            )
        });
        assert!(spans.0.iter().all(|span| span.range.end <= spans.1));
    });
}

#[test]
fn an_oversized_file_stays_editable_without_highlighting() {
    let huge = format!(
        "fn main() {{}}\n// {}\n",
        "x".repeat(chaus_highlight::MAX_HIGHLIGHT_BYTES)
    );
    let directory = fixture("huge.rs", &huge);
    open(&directory.path().join("huge.rs"), |editor, cx| {
        settle(cx);
        assert!(cx.update(|_, cx| editor.read(cx).syntax_spans().is_empty()));
        let status = cx.update(|_, cx| editor.read(cx).status_text().map(str::to_string));
        assert!(
            status
                .as_ref()
                .is_some_and(|status| status.contains("Highlighting off")),
            "the fallback is reported: {status:?}"
        );
        cx.simulate_input("let live = true;\n");
        settle(cx);
        assert!(cx.update(|_, cx| editor.read(cx).document().text().contains("live")));
    });
}
