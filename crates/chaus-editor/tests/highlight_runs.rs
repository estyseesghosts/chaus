//! Painted GPUI run lengths must match the layout's displayed bytes.
//!
//! `chaus-layout` expands tabs and measures graphemes; `editor_paint` must not
//! re-derive those offsets from Unicode scalars. These tests cross the two
//! layers through the real editor so an emoji or combining sequence before a
//! tab cannot silently shift every later run.

use chaus_core::position::LineIndex;
use chaus_core::settings::{Settings, WrapMode};
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
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        draw(cx);
    }
}

fn open<F>(source: &str, settings: Settings, test: F)
where
    F: FnOnce(Entity<Editor>, &mut VisualTestContext),
{
    let mut app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runs.rs");
    std::fs::write(&path, source).unwrap();
    let loaded = chaus_io::load(&path).unwrap();
    let (harness, cx) = app.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        Harness { editor }
    });
    let editor = cx.update(|_, app| harness.read(app).editor.clone());
    draw(cx);
    test(editor, cx);
}

fn assert_runs_match_display(editor: &Entity<Editor>, cx: &mut VisualTestContext, line: LineIndex) {
    let (segments, runs) = cx.update(|_, cx| {
        let editor = editor.read(cx);
        let segments = editor.visual_segments(line);
        let runs: Vec<_> = (0..segments.len())
            .map(|index| {
                editor
                    .syntax_run_lengths(line, index)
                    .expect("every visual segment has runs")
            })
            .collect();
        (segments, runs)
    });
    for (segment, runs) in segments.iter().zip(runs.iter()) {
        let total: usize = runs.iter().map(|(len, _)| *len).sum();
        assert_eq!(
            total,
            segment.display.len(),
            "run lengths must cover {:?} exactly",
            segment.display
        );
        let mut boundary = 0usize;
        for (len, _) in runs {
            assert!(
                segment.display.is_char_boundary(boundary),
                "run boundary {boundary} splits UTF-8 in {:?}",
                segment.display
            );
            boundary += len;
        }
        assert!(segment.display.is_char_boundary(boundary));
    }
}

#[test]
fn emoji_and_combining_marks_before_tabs_do_not_desynchronize_runs() {
    let source = "👩‍💻\tlet x = 1;\ne\u{301}\tlet y = 2;\n漢字\tlet z = 3;\n\t\tlet w = 4;\n";
    open(source, Settings::default(), |editor, cx| {
        settle(cx);
        for line in 0..4 {
            assert_runs_match_display(&editor, cx, LineIndex(line));
        }

        let (segments, runs, spans) = cx.update(|_, cx| {
            let editor = editor.read(cx);
            (
                editor.visual_segments(LineIndex(0)),
                editor.syntax_run_lengths(LineIndex(0), 0).unwrap(),
                editor.syntax_spans().to_vec(),
            )
        });
        assert!(
            !spans.is_empty(),
            "the Rust fixture must actually be highlighted"
        );
        assert!(
            runs.iter()
                .any(|(_, kind)| *kind == Some(chaus_highlight::TokenKind::Keyword)),
            "`let` must be coloured; runs were {runs:?}"
        );
        let space_after_tab = "👩‍💻  ".len();
        assert_eq!(
            runs[0].0, space_after_tab,
            "the first run must end where `let` begins in the expanded row: {runs:?}"
        );
        assert_eq!(
            runs[1],
            (3, Some(chaus_highlight::TokenKind::Keyword)),
            "`let` is three ASCII bytes"
        );
        assert_eq!(
            segments[0].display.len(),
            source.lines().next().unwrap().len() + 1
        );
    });
}

#[test]
fn every_wrapped_row_of_a_long_line_has_exact_runs() {
    let paragraph = "alpha 👩‍💻 beta gamma delta epsilon zeta eta theta iota kappa lambda ";
    let source = paragraph.repeat(20);
    let mut settings = Settings::default();
    settings.wrap = WrapMode::Word;
    open(&source, settings, |editor, cx| {
        settle(cx);
        let line_count = cx.update(|_, cx| editor.read(cx).document().len_lines());
        for line in 0..line_count {
            assert_runs_match_display(&editor, cx, LineIndex(line));
        }
        let segment_count = cx.update(|_, cx| editor.read(cx).visual_segments(LineIndex(0)).len());
        assert!(
            segment_count > 1,
            "the fixture must wrap over several visual rows, got {segment_count}"
        );
    });
}
