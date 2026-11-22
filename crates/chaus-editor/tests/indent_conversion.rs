//! Spaces-to-tabs conversion must preserve visual width, including remainders.

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext, Window,
};

struct Harness {
    editor: Entity<Editor>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone().into_any_element()
    }
}

fn initialize(app: &mut TestAppContext) {
    app.update(|cx| {
        silvestris::test_support::initialize(
            silvestris::ThemePreference::explicit(silvestris::ThemeId::AQUA_LIGHT),
            cx,
        );
        chaus_editor::bind_keys(cx);
    });
}

fn width(prefix: &str, tab_stop: usize) -> usize {
    let mut column = 0;
    for ch in prefix.chars() {
        if ch == '\t' {
            column += tab_stop - (column % tab_stop);
        } else {
            column += 1;
        }
    }
    column
}

fn leading_ws(line: &str) -> String {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect()
}

#[test]
fn conversion_keeps_visual_width_and_nonindent_text_for_every_prefix() {
    for tab_stop in [2usize, 4, 8] {
        let prefixes = [
            String::new(),
            " ".to_string(),
            "  ".to_string(),
            "     ".to_string(),
            "  \t ".to_string(),
            "\t  ".to_string(),
            format!("{}{}", " ".repeat(2 * tab_stop + 2), "\t "),
            "          ".to_string(),
        ];
        let original: Vec<String> = prefixes
            .iter()
            .enumerate()
            .map(|(index, prefix)| format!("{prefix}text{index} ünïcode"))
            .collect();
        let text = original.join("\n");
        let mut app = TestAppContext::single();
        initialize(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("indent.txt");
        std::fs::write(&path, &text).unwrap();
        let loaded = chaus_io::load(&path).unwrap();
        let (harness, cx) = app.add_window_view(|window, cx| {
            let mut settings = Settings::default();
            settings.tab_stop = tab_stop;
            settings.expand_tabs = false;
            let editor = cx.new(|cx| Editor::new(cx, Some(loaded), settings));
            let focus = editor.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            Harness { editor }
        });
        let editor = cx.update(|_, app| harness.read(app).editor.clone());
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.convert_indentation_to_tabs(cx));
        });

        let converted = cx.update(|_, cx| editor.read(cx).document().text());
        let lines: Vec<&str> = converted.split('\n').collect();
        assert_eq!(lines.len(), original.len());
        for (index, (before, after)) in original.iter().zip(lines.iter()).enumerate() {
            let before_ws = leading_ws(before);
            let after_ws = leading_ws(after);
            assert_eq!(
                width(&before_ws, tab_stop),
                width(&after_ws, tab_stop),
                "line {index} changed visual indentation: {before:?} -> {after:?} at stop {tab_stop}"
            );
            assert_eq!(
                before.trim_start_matches([' ', '\t']),
                after.trim_start_matches([' ', '\t']),
                "line {index} changed nonindent text"
            );
        }

        // One undo restores every line byte-for-byte.
        cx.simulate_keystrokes("ctrl-z");
        let restored = cx.update(|_, cx| editor.read(cx).document().text());
        assert_eq!(restored, text, "conversion must be one undo transaction");
    }
}
