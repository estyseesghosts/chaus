//! Visual mode.
//!
//! Movement and text objects go through the same resolver as Normal and
//! operator-pending mode. Visual-line finishes explicitly linewise; a
//! characterwise text object inside Visual-line does not change that.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, CharRange, Selection};

use crate::command_line::CommandPrefix;
use crate::engine::{Engine, Outcome, Pending};
use crate::key::Key;
use crate::mode::Mode;
use crate::motion::{self, FindDirection, Motion};
use crate::operator::Operator;

impl Engine {
    pub(crate) fn visual_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match key {
            Key::Escape => {
                let head = selection.head;
                self.set_mode(Mode::Normal);
                self.set_visual_anchor(None);
                *selection = Selection::caret(head);
                self.clear_pending();
                Outcome::Handled
            }
            Key::Control('d') => self.half_page(true),
            Key::Control('u') => self.half_page(false),
            Key::Char(character)
                if character.is_ascii_digit() && (character != '0' || self.count > 0) =>
            {
                self.push_digits(character as usize - '0' as usize);
                Outcome::Handled
            }
            Key::Char(character) => {
                if let Some(pending) = self.take_pending() {
                    return self.pending_key(pending, character, document, selection);
                }
                self.visual_char(character, document, selection)
            }
            _ => Outcome::Handled,
        }
    }

    fn visual_char(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match character {
            'v' => {
                let head = selection.head;
                self.set_mode(Mode::Normal);
                self.set_visual_anchor(None);
                *selection = Selection::caret(head);
                self.reset_count();
                Outcome::Handled
            }
            'V' => {
                if self.mode() == Mode::VisualLine {
                    let head = selection.head;
                    self.set_mode(Mode::Normal);
                    self.set_visual_anchor(None);
                    *selection = Selection::caret(head);
                    self.reset_count();
                } else {
                    self.set_mode(Mode::VisualLine);
                }
                Outcome::Handled
            }
            'o' => {
                let anchor = self.visual_anchor().unwrap_or(selection.head);
                self.set_visual_anchor(Some(selection.head));
                *selection = Selection::new(selection.head, anchor);
                Outcome::Handled
            }
            'd' | 'x' => self.finish_visual(Operator::Delete, document, selection),
            'c' => self.finish_visual(Operator::Change, document, selection),
            'y' => self.finish_visual(Operator::Yank, document, selection),
            ':' => self.start_command_line(CommandPrefix::Ex),
            '>' | '<' => self.indent_visual_lines(character == '<', document, selection),
            'i' | 'a' => {
                self.set_pending(Some(Pending::TextObject {
                    operator: None,
                    outer: character == 'a',
                }));
                Outcome::Handled
            }
            'f' | 'F' | 't' | 'T' => {
                self.set_pending(Some(Pending::Find {
                    operator: None,
                    direction: if matches!(character, 'f' | 't') {
                        FindDirection::Forward
                    } else {
                        FindDirection::Backward
                    },
                    till: matches!(character, 't' | 'T'),
                }));
                Outcome::Handled
            }
            ';' | ',' => self.visual_repeat_find(character == ',', document, selection),
            'g' => {
                self.set_pending(Some(Pending::G));
                Outcome::Handled
            }
            '%' => self.move_visual_motion(Motion::MatchPair, document, selection),
            character => {
                if let Some(motion) = Motion::from_key(character) {
                    self.move_visual_motion(motion, document, selection)
                } else {
                    Outcome::Message(format!("visual command '{character}' is not implemented"))
                }
            }
        }
    }

    fn visual_repeat_find(
        &mut self,
        reversed: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some(state) = self.last_find() else {
            return Outcome::Message("no previous find".to_owned());
        };
        let state = if reversed { state.reversed() } else { state };
        self.set_last_find(Some(state));
        self.move_visual_motion(Motion::Find(state), document, selection)
    }

    fn indent_visual_lines(
        &mut self,
        outdent: bool,
        document: &Document,
        selection: &Selection,
    ) -> Outcome {
        let (first, last) = visual_lines(document, *selection, self.mode() == Mode::VisualLine);
        self.set_visual_anchor(None);
        self.set_mode(Mode::Normal);
        self.reset_count();
        Outcome::Indent {
            outdent,
            first_line: first,
            last_line: last,
        }
    }

    fn finish_visual(
        &mut self,
        operator: Operator,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let visual_line = self.mode() == Mode::VisualLine;
        let range = if visual_line {
            let (first, last) = visual_lines(document, *selection, true);
            motion::linewise_range(document, first, last.0 - first.0 + 1)
        } else {
            let mut range = selection.range();
            if range.is_empty() && range.start.0 < document.len_chars() {
                let line = document.line_of(range.start);
                if range.start < document.line_content_end(line) {
                    range.end = document.next_grapheme_boundary(range.start);
                }
            } else if range.start < range.end && range.end.0 < document.len_chars() {
                range.end = document.next_grapheme_boundary(range.end);
            }
            range
        };
        let range = if operator == Operator::Change && visual_line {
            let end_line = document.line_of(CharOffset(range.end.0.saturating_sub(1)));
            CharRange::new(range.start, document.line_content_end(end_line))
        } else {
            range
        };
        // A visual range has no replayable Normal-mode command; leave `.`
        // pointing at the previous change.
        self.set_visual_anchor(None);
        self.reset_count();
        let outcome = self.apply_operator(operator, document, selection, range, visual_line, None);
        if self.mode() != Mode::Insert {
            self.set_mode(Mode::Normal);
        }
        outcome
    }
}

/// Hard lines covered by a Visual selection. Visual-line uses the anchor and
/// head lines directly, because a head at a line start still selects that
/// line. Characterwise follows the text range rule instead.
fn visual_lines(
    document: &Document,
    selection: Selection,
    visual_line: bool,
) -> (
    chaus_core::position::LineIndex,
    chaus_core::position::LineIndex,
) {
    if visual_line {
        let anchor = document.line_of(selection.anchor);
        let head = document.line_of(selection.head);
        (anchor.min(head), anchor.max(head))
    } else {
        motion::selected_lines(document, selection)
    }
}
