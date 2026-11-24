//! Presentation of the temporary Vim `:`, `/` and `?` input line.

use gpui::{
    div, px, relative, Div, InteractiveElement as _, ParentElement as _, Stateful, Styled as _,
    Window,
};
use silvestris::core::tokens::Theme;

use crate::editor::Editor;

/// Characters drawn beside the caret so it is never at the clipped edge.
const CARET_MARGIN: usize = 8;

impl Editor {
    /// The input line, drawn over the bottom-left corner only while it is
    /// open. Long input is clipped around the caret so the caret stays
    /// visible; the line never wraps over the document.
    pub(crate) fn render_command_line(
        &self,
        window: &Window,
        theme: &Theme,
    ) -> Option<Stateful<Div>> {
        if !self.vim {
            return None;
        }
        let view = self.vim_engine.command_line()?;
        let width = f32::from(window.viewport_size().width);
        // Approximate characters that fit in 75% of the window at 12px.
        let budget = ((width * 0.75) / 7.0).max(24.0) as usize;
        let before_count = view.before_caret.chars().count();
        let before = if before_count + CARET_MARGIN > budget {
            let keep = budget.saturating_sub(CARET_MARGIN + 1);
            let tail: String = view
                .before_caret
                .chars()
                .skip(before_count.saturating_sub(keep))
                .collect();
            format!("\u{2026}{tail}")
        } else {
            view.before_caret.to_owned()
        };
        let remaining = budget.saturating_sub(before.chars().count());
        let after = if view.after_caret.chars().count() > remaining {
            let head: String = view
                .after_caret
                .chars()
                .take(remaining.saturating_sub(1))
                .collect();
            format!("{head}\u{2026}")
        } else {
            view.after_caret.to_owned()
        };
        let text = silvestris::convert::hsla(theme.colors.text);
        let mut line = div()
            .id("chaus-command-line")
            .debug_selector(|| "chaus-command-line".to_string())
            .absolute()
            .bottom(px(6.))
            .left(px(10.))
            .max_w(relative(0.75))
            .overflow_hidden()
            .flex()
            .items_center()
            .whitespace_nowrap()
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .bg(silvestris::convert::hsla(theme.colors.group_fill))
            .text_color(text)
            .text_size(px(12.))
            .child(format!("{}{before}", view.prefix.symbol()));
        if let Some(preedit) = view.preedit {
            line = line.child(
                div()
                    .text_color(silvestris::convert::hsla(theme.colors.text_secondary))
                    .child(preedit.to_owned()),
            );
        }
        Some(line.child(div().w(px(1.)).h(px(14.)).bg(text)).child(after))
    }
}
