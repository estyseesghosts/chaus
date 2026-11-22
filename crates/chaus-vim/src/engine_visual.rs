//! Visual mode and the command line.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};

use crate::engine::{CommandKind, Engine, Outcome};
use crate::key::Key;
use crate::mode::Mode;
use crate::motion;
use crate::operator::{self, Operator};

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
            Key::Char(character)
                if character.is_ascii_digit() && (character != '0' || self.count > 0) =>
            {
                self.push_digits(character as usize - '0' as usize);
                Outcome::Handled
            }
            Key::Char('v') => {
                let head = selection.head;
                self.set_mode(Mode::Normal);
                self.set_visual_anchor(None);
                *selection = Selection::caret(head);
                self.reset_count();
                Outcome::Handled
            }
            Key::Char('V') => {
                self.set_mode(Mode::VisualLine);
                Outcome::Handled
            }
            Key::Char('o') => {
                let anchor = self.visual_anchor().unwrap_or(selection.head);
                self.set_visual_anchor(Some(selection.head));
                *selection = Selection::new(selection.head, anchor);
                Outcome::Handled
            }
            Key::Char('d' | 'x') => self.finish_visual(Operator::Delete, document, selection),
            Key::Char('c') => self.finish_visual(Operator::Change, document, selection),
            Key::Char('y') => self.finish_visual(Operator::Yank, document, selection),
            Key::Char(':') => {
                self.set_mode(Mode::CommandLine);
                self.set_command_kind(CommandKind::Ex);
                self.take_command();
                Outcome::Handled
            }
            Key::Char(
                character @ ('h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | '0' | '^' | '$' | 'G'),
            ) => {
                let count = self.count();
                let mut head = selection.head;
                for _ in 0..count {
                    head = match character {
                        'h' => document.prev_grapheme_boundary(head),
                        'l' => document.next_grapheme_boundary(head),
                        'j' => motion::vertical(document, head, 1),
                        'k' => motion::vertical(document, head, -1),
                        'w' => motion::next_word_start(document.rope(), head),
                        'b' => motion::previous_word_start(document.rope(), head),
                        'e' => motion::word_end(document.rope(), head),
                        '0' => {
                            let line = document.line_of(head);
                            document.line_start(line)
                        }
                        '^' => {
                            let line = document.line_of(head);
                            motion::first_non_space(document, line)
                        }
                        '$' => {
                            let line = document.line_of(head);
                            document.line_content_end(line)
                        }
                        'G' => {
                            let line = LineIndex(document.len_lines().saturating_sub(1));
                            motion::first_non_space(document, line)
                        }
                        _ => head,
                    };
                }
                let anchor = self.visual_anchor().unwrap_or(selection.anchor);
                *selection = Selection::new(anchor, head);
                self.reset_count();
                Outcome::Handled
            }
            Key::Char(character) => {
                self.message(format!("visual command '{character}' is not implemented"));
                Outcome::Handled
            }
            Key::Enter | Key::Backspace | Key::Tab | Key::Control(_) => Outcome::Handled,
        }
    }

    fn finish_visual(
        &mut self,
        operator: Operator,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let visual_line = self.mode() == Mode::VisualLine;
        let mut range = if visual_line {
            let from = document.line_of(selection.range().start);
            let to = document.line_of(selection.range().end);
            let start = from.0.min(to.0);
            let count = from.0.max(to.0) - start + 1;
            motion::linewise_range(document, LineIndex(start), count)
        } else {
            selection.range()
        };
        if !visual_line && range.start < range.end && range.end.0 < document.len_chars() {
            range.end = document.next_grapheme_boundary(range.end);
        }
        let range = if operator == Operator::Change && visual_line {
            let end_line = document.line_of(CharOffset(range.end.0.saturating_sub(1)));
            CharRange::new(range.start, document.line_content_end(end_line))
        } else {
            range
        };
        let before = *selection;
        let applied = operator::apply(
            operator,
            document,
            before,
            range,
            visual_line,
            self.register_mut(),
        );
        self.set_visual_anchor(None);
        self.reset_count();
        match applied {
            Some(applied) => {
                *selection = Selection::caret(motion::clamp_normal(document, applied.caret));
                self.set_changed();
                if applied.enter_insert {
                    self.set_mode(Mode::Insert);
                    self.insert_buffer().clear();
                } else {
                    self.set_mode(Mode::Normal);
                }
                Outcome::Changed
            }
            None => {
                self.set_mode(Mode::Normal);
                self.message("edit was rejected");
                Outcome::Handled
            }
        }
    }

    pub(crate) fn command_line_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match key {
            Key::Escape => {
                self.take_command();
                self.set_mode(Mode::Normal);
                Outcome::Handled
            }
            Key::Backspace => {
                self.command_mut().pop();
                Outcome::Handled
            }
            Key::Enter => {
                let command = self.take_command();
                let kind = match self.command_kind() {
                    CommandKind::Search { forward } => CommandKind::Search { forward: *forward },
                    CommandKind::Ex => CommandKind::Ex,
                };
                self.set_mode(Mode::Normal);
                match kind {
                    CommandKind::Search { forward } => {
                        if command.is_empty() {
                            return Outcome::Handled;
                        }
                        if let Some(offset) =
                            motion::find_literal(document, selection.head, &command, forward)
                        {
                            *selection = Selection::caret(offset);
                            self.set_last_search(Some((command, forward)));
                        } else {
                            self.message(format!("pattern not found: {command}"));
                        }
                        Outcome::Handled
                    }
                    CommandKind::Ex => Outcome::Command(command),
                }
            }
            Key::Char(character) => {
                if self.command_mut().chars().count() < 512 {
                    self.command_mut().push(character);
                }
                Outcome::Handled
            }
            Key::Tab => {
                self.command_mut().push('\t');
                Outcome::Handled
            }
            Key::Control(_) => Outcome::Handled,
        }
    }
}
