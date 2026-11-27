//! Normal-mode grammar, motions, operators and Insert transitions.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};

use crate::command_line::CommandPrefix;
use crate::engine::{merge_outcome, Engine, Outcome, Pending};
use crate::ex_command::ExCommand;
use crate::key::Key;
use crate::mode::Mode;
use crate::motion::{self, FindDirection, FindState, Motion};
use crate::operator::{self, Operator};
use crate::viewport::{ViewportCommand, ViewportIntent};

impl Engine {
    pub(crate) fn normal_key(
        &mut self,
        key: Key,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match key {
            Key::Escape => {
                self.clear_pending();
                self.set_mode(Mode::Normal);
                self.set_visual_anchor(None);
                Outcome::Handled
            }
            Key::Control('r') => {
                self.clear_pending();
                if let Some(after) = document.redo() {
                    *selection = after;
                    Outcome::Changed
                } else {
                    Outcome::Handled
                }
            }
            Key::Control('d') => self.half_page(true),
            Key::Control('u') => self.half_page(false),
            Key::Char(character) => self.normal_char(character, document, selection),
            _ => Outcome::Handled,
        }
    }

    pub(crate) fn half_page(&mut self, down: bool) -> Outcome {
        // A half-page command is never a continuation of `z`, `f` or an
        // operator; cancel the prefix instead of leaving it armed.
        self.set_pending(None);
        let count = self.count();
        self.reset_count();
        Outcome::Viewport(ViewportIntent::new(
            if down {
                ViewportCommand::HalfPageDown
            } else {
                ViewportCommand::HalfPageUp
            },
            count,
        ))
    }

    fn normal_char(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        // A leading `0` is the line-start motion, not a count digit. It is a
        // digit only after another count digit has already been entered.
        if character.is_ascii_digit()
            && (character != '0' || self.count > 0 || self.motion_count > 0)
        {
            self.push_digits(character as usize - '0' as usize);
            return Outcome::Handled;
        }

        if let Some(pending) = self.take_pending() {
            return self.pending_key(pending, character, document, selection);
        }

        match character {
            'g' => {
                self.set_pending(Some(Pending::G));
                Outcome::Handled
            }
            'd' | 'c' | 'y' => {
                if let Some(operator) = Operator::from_key(character) {
                    self.set_pending(Some(Pending::Operator(operator)));
                }
                Outcome::Handled
            }
            'f' | 'F' | 't' | 'T' => {
                self.set_pending(Some(Pending::Find {
                    operator: None,
                    direction: find_direction(character),
                    till: matches!(character, 't' | 'T'),
                }));
                Outcome::Handled
            }
            ';' | ',' => self.repeat_find(character == ',', document, selection),
            'i' => self.enter_insert(document, selection, selection.head, 'i'),
            'a' => {
                let head = selection.head;
                let line = document.line_of(head);
                let end = document.line_content_end(line);
                let target = if head < end {
                    document.next_grapheme_boundary(head)
                } else {
                    end
                };
                self.enter_insert(document, selection, target, 'a')
            }
            'I' => {
                let line = document.line_of(selection.head);
                let target = motion::first_non_space(document, line);
                self.enter_insert(document, selection, target, 'I')
            }
            'A' => {
                let line = document.line_of(selection.head);
                let target = document.line_content_end(line);
                self.enter_insert(document, selection, target, 'A')
            }
            'o' => self.open_line(document, selection, true),
            'O' => self.open_line(document, selection, false),
            'v' => {
                self.set_mode(Mode::VisualChar);
                self.set_visual_anchor(Some(selection.head));
                *selection = Selection::new(selection.head, selection.head);
                self.reset_count();
                Outcome::Handled
            }
            'V' => {
                self.set_mode(Mode::VisualLine);
                self.set_visual_anchor(Some(selection.head));
                *selection = Selection::new(selection.head, selection.head);
                self.reset_count();
                Outcome::Handled
            }
            'x' => self.delete_characters(document, selection),
            'X' => self.delete_backward(document, selection),
            's' => self.substitute_characters(document, selection),
            'D' => self.operator_to_line_end(Operator::Delete, 'D', document, selection),
            'C' => self.operator_to_line_end(Operator::Change, 'C', document, selection),
            'S' => self.apply_linewise(Operator::Change, document, selection, vec![Key::Char('S')]),
            'Y' => self.apply_linewise(Operator::Yank, document, selection, vec![Key::Char('Y')]),
            'r' => {
                self.set_pending(Some(Pending::Replace));
                Outcome::Handled
            }
            'p' => self.paste(document, selection, true),
            'P' => self.paste(document, selection, false),
            'u' => {
                self.clear_pending();
                if let Some(after) = document.undo() {
                    *selection = after;
                    Outcome::Changed
                } else {
                    Outcome::Handled
                }
            }
            '/' => self.start_command_line(CommandPrefix::SearchForward),
            '?' => self.start_command_line(CommandPrefix::SearchBackward),
            ':' => self.start_command_line(CommandPrefix::Ex),
            'n' | 'N' => self.repeat_search(character == 'N', document, selection),
            '.' => self.repeat_last_change(document, selection),
            'J' => self.join_lines(document, selection, false),
            '*' | '#' => self.search_word(character == '#', document, selection),
            '>' => {
                self.set_pending(Some(Pending::Indent { outdent: false }));
                Outcome::Handled
            }
            '<' => {
                self.set_pending(Some(Pending::Indent { outdent: true }));
                Outcome::Handled
            }
            'z' => {
                self.set_pending(Some(Pending::Scroll));
                Outcome::Handled
            }
            'Z' => {
                self.set_pending(Some(Pending::Z));
                Outcome::Handled
            }
            character => {
                if let Some(motion) = Motion::from_key(character) {
                    self.move_motion(motion, document, selection)
                } else {
                    self.reset_count();
                    Outcome::Message(format!("unknown command '{character}'"))
                }
            }
        }
    }

    pub(crate) fn pending_key(
        &mut self,
        pending: Pending,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match pending {
            Pending::Operator(operator) => {
                self.operator_pending_key(operator, character, document, selection)
            }
            Pending::OperatorG(operator) => {
                if let Some(motion) = Motion::from_g(character) {
                    let keys = vec![
                        Key::Char(operator.key()),
                        Key::Char('g'),
                        Key::Char(character),
                    ];
                    return self.apply_operator_motion(operator, motion, document, selection, keys);
                }
                self.finish_pending_error(format!(
                    "expected a 'g' motion, not '{character}' after 'g'"
                ))
            }
            Pending::G => self.pending_g(character, document, selection),
            Pending::Find {
                operator,
                direction,
                till,
            } => {
                let state = FindState {
                    character,
                    direction,
                    till,
                };
                self.apply_find(state, operator, document, selection)
            }
            Pending::Replace => self.replace_characters(character, document, selection),
            Pending::TextObject { operator, outer } => {
                self.apply_text_object(operator, outer, character, document, selection)
            }
            Pending::Indent { outdent } => {
                let expected = if outdent { '<' } else { '>' };
                if character != expected {
                    return self.finish_pending_error(format!(
                        "expected '{expected}' after indentation prefix"
                    ));
                }
                self.indent_current_lines(outdent, document, selection)
            }
            Pending::Z => match character {
                'Z' => {
                    self.clear_pending();
                    Outcome::Ex(ExCommand::Exit)
                }
                'Q' => {
                    self.clear_pending();
                    Outcome::Ex(ExCommand::Quit { force: true })
                }
                other => self.finish_pending_error(format!("unknown Z command 'Z{other}'")),
            },
            Pending::Scroll => {
                let command = match character {
                    'z' => Some(ViewportCommand::Center),
                    't' => Some(ViewportCommand::Top),
                    'b' => Some(ViewportCommand::Bottom),
                    _ => None,
                };
                match command {
                    Some(command) => {
                        self.reset_count();
                        Outcome::Viewport(ViewportIntent::new(command, 1))
                    }
                    None => self.finish_pending_error(format!("unknown z command 'z{character}'")),
                }
            }
        }
    }

    fn pending_g(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        if let Some(motion) = Motion::from_g(character) {
            if matches!(self.mode(), Mode::VisualChar | Mode::VisualLine) {
                return self.move_visual_motion(motion, document, selection);
            }
            return self.move_motion(motion, document, selection);
        }
        if character == 'J' {
            return self.join_lines(document, selection, true);
        }
        self.finish_pending_error(format!(
            "expected 'g', 'e' or 'J' after 'g', not '{character}'"
        ))
    }

    fn operator_pending_key(
        &mut self,
        operator: Operator,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        if Operator::from_key(character) == Some(operator) {
            let key = Key::Char(operator.key());
            return self.apply_linewise(operator, document, selection, vec![key, key]);
        }
        if character == 'i' || character == 'a' {
            self.set_pending(Some(Pending::TextObject {
                operator: Some(operator),
                outer: character == 'a',
            }));
            return Outcome::Handled;
        }
        if character == 'g' {
            self.set_pending(Some(Pending::OperatorG(operator)));
            return Outcome::Handled;
        }
        if matches!(character, 'f' | 'F' | 't' | 'T') {
            self.set_pending(Some(Pending::Find {
                operator: Some(operator),
                direction: find_direction(character),
                till: matches!(character, 't' | 'T'),
            }));
            return Outcome::Handled;
        }
        if matches!(character, ';' | ',') {
            let Some(state) = self.last_find() else {
                return self.finish_pending_error("no previous find to repeat");
            };
            let state = if character == ',' {
                state.reversed()
            } else {
                state
            };
            let keys = vec![Key::Char(operator.key()), Key::Char(character)];
            return self.apply_operator_motion(
                operator,
                Motion::Find(state),
                document,
                selection,
                keys,
            );
        }
        if operator == Operator::Change && matches!(character, 'w' | 'W') {
            return self.change_word(operator, character == 'W', document, selection);
        }
        if let Some(motion) = Motion::from_key(character) {
            let keys = vec![Key::Char(operator.key()), Key::Char(character)];
            return self.apply_operator_motion(operator, motion, document, selection, keys);
        }
        self.finish_pending_error("operator needs a motion")
    }

    fn finish_pending_error(&mut self, message: impl Into<String>) -> Outcome {
        self.reset_count();
        Outcome::Message(message.into())
    }

    fn move_motion(
        &mut self,
        motion: Motion,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        let explicit_count = self.explicit_count();
        self.reset_count();
        let Some(resolved) =
            motion::resolve(document, selection.head, motion, count, explicit_count)
        else {
            return Outcome::Message(motion_failure(motion));
        };
        *selection = Selection::caret(motion::clamp_normal(document, resolved.target));
        Outcome::Handled
    }

    pub(crate) fn move_visual_motion(
        &mut self,
        motion: Motion,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        let explicit_count = self.explicit_count();
        self.reset_count();
        let Some(resolved) =
            motion::resolve(document, selection.head, motion, count, explicit_count)
        else {
            return Outcome::Message(motion_failure(motion));
        };
        let anchor = self.visual_anchor().unwrap_or(selection.anchor);
        *selection = Selection::new(anchor, motion::clamp_normal(document, resolved.target));
        Outcome::Handled
    }

    fn apply_operator_motion(
        &mut self,
        operator: Operator,
        motion: Motion,
        document: &mut Document,
        selection: &mut Selection,
        keys: Vec<Key>,
    ) -> Outcome {
        let count = self.count();
        let explicit_count = self.explicit_count();
        let Some(resolved) =
            motion::resolve(document, selection.head, motion, count, explicit_count)
        else {
            return self.finish_pending_error(motion_failure(motion));
        };
        self.apply_operator(
            operator,
            document,
            selection,
            resolved.range,
            resolved.linewise,
            Some(keys),
        )
    }

    /// `cw`/`cW` change through the end of the current word instead of
    /// swallowing the whitespace that `dw` deletes.
    fn change_word(
        &mut self,
        operator: Operator,
        big: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        let explicit_count = self.explicit_count();
        let motion = if big {
            Motion::WordStartBig
        } else {
            Motion::WordStart
        };
        let Some(resolved) =
            motion::resolve(document, selection.head, motion, count, explicit_count)
        else {
            return self.finish_pending_error("operator needs a motion");
        };
        let mut range = resolved.range;
        let head = selection.head;
        let starts_in_word =
            head.0 < document.len_chars() && !document.rope().char(head.0).is_whitespace();
        if starts_in_word {
            let mut end = range.end;
            while end > head
                && document
                    .rope()
                    .char(end.0.saturating_sub(1))
                    .is_whitespace()
            {
                end = document.prev_grapheme_boundary(end);
            }
            range = CharRange::new(head, end);
        }
        let key = if big { 'W' } else { 'w' };
        self.apply_operator(
            operator,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char('c'), Key::Char(key)]),
        )
    }

    fn apply_find(
        &mut self,
        state: FindState,
        operator: Option<Operator>,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        self.set_last_find(Some(state));
        let motion = Motion::Find(state);
        match operator {
            Some(operator) => {
                let keys = vec![Key::Char(operator.key()), Key::Char(find_key(state))];
                self.apply_operator_motion(operator, motion, document, selection, keys)
            }
            None if matches!(self.mode(), Mode::VisualChar | Mode::VisualLine) => {
                self.move_visual_motion(motion, document, selection)
            }
            None => self.move_motion(motion, document, selection),
        }
    }

    fn repeat_find(
        &mut self,
        reversed: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some(state) = self.last_find() else {
            return Outcome::Message("no previous find".to_owned());
        };
        let state = if reversed { state.reversed() } else { state };
        self.apply_find(state, None, document, selection)
    }

    fn apply_text_object(
        &mut self,
        operator: Option<Operator>,
        outer: bool,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let range = match character {
            'w' => Some(motion::word_object(document, selection.head, outer)),
            '"' => motion::quoted_object(document, selection.head, '"', outer),
            '\'' => motion::quoted_object(document, selection.head, '\'', outer),
            '(' | ')' | 'b' => motion::bracketed_object(document, selection.head, '(', ')', outer),
            '[' | ']' => motion::bracketed_object(document, selection.head, '[', ']', outer),
            '{' | '}' => motion::bracketed_object(document, selection.head, '{', '}', outer),
            _ => None,
        };
        match range {
            Some(range) => match operator {
                Some(operator) => {
                    let keys = vec![
                        Key::Char(operator.key()),
                        Key::Char(if outer { 'a' } else { 'i' }),
                        Key::Char(character),
                    ];
                    self.apply_operator(operator, document, selection, range, false, Some(keys))
                }
                None if matches!(self.mode(), Mode::VisualChar | Mode::VisualLine) => {
                    let anchor = range.start;
                    let head = if range.is_empty() {
                        range.start
                    } else {
                        document.prev_grapheme_boundary(range.end)
                    };
                    self.set_visual_anchor(Some(anchor));
                    *selection = Selection::new(anchor, head);
                    self.reset_count();
                    Outcome::Handled
                }
                None => Outcome::Handled,
            },
            None => self.finish_pending_error("no matching text object"),
        }
    }

    fn apply_linewise(
        &mut self,
        operator: Operator,
        document: &mut Document,
        selection: &mut Selection,
        keys: Vec<Key>,
    ) -> Outcome {
        let count = self.count();
        let line = document.line_of(selection.head);
        let mut range = motion::linewise_range(document, line, count);
        if operator == Operator::Change {
            let last = (line.0 + count - 1).min(document.len_lines().saturating_sub(1));
            range = CharRange::new(range.start, document.line_content_end(LineIndex(last)));
        }
        self.apply_operator(operator, document, selection, range, true, Some(keys))
    }

    /// Apply a validated operator range. `keys` is the Normal-mode command
    /// that produced it, used by `.`; visual selections pass `None` because
    /// their range cannot be replayed from a new caret.
    pub(crate) fn apply_operator(
        &mut self,
        operator: Operator,
        document: &mut Document,
        selection: &mut Selection,
        range: CharRange,
        linewise: bool,
        keys: Option<Vec<Key>>,
    ) -> Outcome {
        // A Change and everything typed into the resulting Insert session are
        // one user operation, so one undo removes the whole change.
        if operator == Operator::Change {
            document.begin_user_operation();
        }
        if range.is_empty() {
            if operator == Operator::Change {
                let keys = keys.map(|keys| self.counted_keys(keys)).unwrap_or_default();
                self.reset_count();
                *selection = Selection::caret(range.start);
                self.begin_insert(range.start, keys, true, 1);
                self.set_mode(Mode::Insert);
                return Outcome::Handled;
            }
            self.reset_count();
            return Outcome::Handled;
        }
        let mut range = range;
        if operator == Operator::Change && linewise {
            let start_line = document.line_of(range.start);
            let end_line = document.line_of(CharOffset(range.end.0.saturating_sub(1)));
            range = CharRange::new(
                motion::first_non_space(document, start_line),
                document.line_content_end(end_line),
            );
        }
        let keys = match operator {
            Operator::Yank => None,
            Operator::Delete | Operator::Change => keys.map(|keys| self.counted_keys(keys)),
        };
        let before = *selection;
        let applied = operator::apply(
            operator,
            document,
            before,
            range,
            linewise,
            self.register_mut(),
        );
        self.reset_count();
        match applied {
            Some(applied) => {
                // Change enters Insert mode, where the caret may legally sit
                // after the line's last character; only Normal-mode carets are
                // clamped onto a character.
                *selection = if applied.enter_insert {
                    Selection::caret(applied.caret)
                } else {
                    Selection::caret(motion::clamp_normal(document, applied.caret))
                };
                if let Some(keys) = keys {
                    if applied.enter_insert {
                        self.begin_insert(applied.caret, keys, true, 1);
                    } else {
                        self.record_change(keys, None);
                    }
                } else if applied.enter_insert {
                    self.begin_insert(applied.caret, Vec::new(), false, 1);
                }
                if applied.enter_insert {
                    self.set_mode(Mode::Insert);
                }
                if operator == Operator::Yank {
                    Outcome::Handled
                } else {
                    Outcome::Changed
                }
            }
            None => {
                if operator == Operator::Change {
                    document.end_user_operation();
                }
                Outcome::Message("edit was rejected".to_owned())
            }
        }
    }

    fn enter_insert(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
        offset: CharOffset,
        command: char,
    ) -> Outcome {
        let repeat = self.count();
        let keys = self.counted_keys(vec![Key::Char(command)]);
        document.begin_user_operation();
        *selection = Selection::caret(offset);
        self.set_mode(Mode::Insert);
        self.begin_insert(offset, keys, false, repeat);
        self.set_visual_anchor(None);
        self.clear_pending();
        Outcome::Handled
    }

    fn open_line(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
        below: bool,
    ) -> Outcome {
        let line = document.line_of(selection.head);
        let indent: String = document
            .line_text(line)
            .chars()
            .take_while(|character| *character == ' ' || *character == '\t')
            .collect();
        let indent_length = indent.chars().count();
        let before = *selection;
        let (at, insert, caret) = if below {
            let end = document.line_content_end(line);
            (
                end,
                format!("\n{indent}"),
                CharOffset(end.0 + 1 + indent_length),
            )
        } else {
            let start = document.line_start(line);
            (
                start,
                format!("{indent}\n"),
                CharOffset(start.0 + indent_length),
            )
        };
        document.begin_user_operation();
        let transaction = EditTransaction::new(
            EditCause::Vim,
            TextEdit::insertion(at, insert),
            before,
            Selection::caret(caret),
        );
        match document.apply(transaction) {
            Ok(_) => {
                *selection = Selection::caret(caret);
                self.set_mode(Mode::Insert);
                let command = if below { 'o' } else { 'O' };
                self.begin_insert(caret, vec![Key::Char(command)], true, 1);
                Outcome::Changed
            }
            Err(_) => {
                document.end_user_operation();
                Outcome::Message("could not open a line".to_owned())
            }
        }
    }

    fn delete_characters(&mut self, document: &mut Document, selection: &mut Selection) -> Outcome {
        let range = character_range(document, selection.head, self.count());
        self.apply_operator(
            Operator::Delete,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char('x')]),
        )
    }

    fn delete_backward(&mut self, document: &mut Document, selection: &mut Selection) -> Outcome {
        let line_start = document.line_start(document.line_of(selection.head));
        let mut start = selection.head;
        for _ in 0..self.count() {
            if start <= line_start {
                break;
            }
            start = document.prev_grapheme_boundary(start);
        }
        let range = CharRange::new(start, selection.head);
        self.apply_operator(
            Operator::Delete,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char('X')]),
        )
    }

    fn substitute_characters(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let range = character_range(document, selection.head, self.count());
        self.apply_operator(
            Operator::Change,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char('s')]),
        )
    }

    fn operator_to_line_end(
        &mut self,
        operator: Operator,
        key: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        let line = document.line_of(selection.head);
        let last = (line.0 + count - 1).min(document.len_lines().saturating_sub(1));
        let end = document.line_content_end(LineIndex(last));
        let range = CharRange::new(selection.head, end);
        self.apply_operator(
            operator,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char(key)]),
        )
    }

    fn replace_characters(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        if character == '\n' {
            self.reset_count();
            return Outcome::Message("'r' cannot insert a line break".to_owned());
        }
        let count = self.count();
        let line = document.line_of(selection.head);
        let content_end = document.line_content_end(line);
        let mut end = selection.head;
        let mut replaced = 0;
        while replaced < count && end < content_end {
            let next = document.next_grapheme_boundary(end);
            if next > content_end {
                break;
            }
            end = next;
            replaced += 1;
        }
        if replaced < count {
            self.reset_count();
            return Outcome::Message("not enough characters on the line for 'r'".to_owned());
        }
        let range = CharRange::new(selection.head, end);
        if range.is_empty() {
            self.reset_count();
            return Outcome::Handled;
        }
        let text: String = std::iter::repeat_n(character, replaced).collect();
        let before = *selection;
        let after = Selection::caret(range.start);
        let transaction =
            EditTransaction::new(EditCause::Vim, TextEdit::new(range, text), before, after);
        let keys = self.counted_keys(vec![Key::Char('r'), Key::Char(character)]);
        self.reset_count();
        match document.apply(transaction) {
            Ok(_) => {
                *selection = after;
                self.record_change(keys, None);
                Outcome::Changed
            }
            Err(_) => Outcome::Handled,
        }
    }

    fn join_lines(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
        without_space: bool,
    ) -> Outcome {
        let count = self.count();
        let first = document.line_of(selection.head);
        // Vim joins `[count]` lines with a minimum of two: plain `J` joins the
        // current line with the next one, `2J` is the same, and `3J` joins
        // three lines.
        let lines = count.max(2);
        let last = (first.0 + lines - 1).min(document.len_lines().saturating_sub(1));
        if last == first.0 {
            self.reset_count();
            return Outcome::Message("no next line to join".to_owned());
        }
        let start = document.line_content_end(first);
        let end = document.line_content_end(LineIndex(last));
        let mut replacement = String::new();
        // The character immediately before the join, including a newline for
        // an empty first line, decides whether a separating space is needed.
        let mut last_character = (start.0 > 0).then(|| document.rope().char(start.0 - 1));
        for line in first.0 + 1..=last {
            let text = document.line_text(LineIndex(line));
            let body = if without_space {
                text.as_str()
            } else {
                text.trim_start_matches([' ', '\t'])
            };
            if !without_space
                && !body.is_empty()
                // Vim does not insert a space before a line that starts with
                // `)`; see `:help J`.
                && !body.starts_with(')')
                && last_character.is_some_and(|character| {
                    character != ' ' && character != '\t' && character != '\n'
                })
            {
                replacement.push(' ');
                last_character = Some(' ');
            }
            replacement.push_str(body);
            if let Some(character) = body.chars().last() {
                last_character = Some(character);
            }
        }
        let before = *selection;
        let caret = CharOffset(start.0);
        let transaction = EditTransaction::new(
            EditCause::Vim,
            TextEdit::new(CharRange::new(start, end), replacement),
            before,
            Selection::caret(caret),
        );
        let keys = if without_space {
            vec![Key::Char('g'), Key::Char('J')]
        } else {
            vec![Key::Char('J')]
        };
        let keys = self.counted_keys(keys);
        self.reset_count();
        match document.apply(transaction) {
            Ok(_) => {
                *selection = Selection::caret(motion::clamp_normal(document, caret));
                self.record_change(keys, None);
                Outcome::Changed
            }
            Err(_) => Outcome::Message("could not join the lines".to_owned()),
        }
    }

    fn search_word(
        &mut self,
        reverse: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        self.reset_count();
        let Some((word, start)) = motion::word_near(document, selection.head) else {
            return Outcome::Message("no word under or next to the cursor".to_owned());
        };
        let forward = !reverse;
        self.set_last_search(Some((word.clone(), forward)));
        let mut position = if forward {
            CharOffset(start.0 + word.chars().count().saturating_sub(1))
        } else {
            start
        };
        let mut found = None;
        for _ in 0..count {
            let Some(offset) = motion::find_literal(document, position, &word, forward) else {
                break;
            };
            position = offset;
            found = Some(offset);
        }
        match found {
            Some(offset) => {
                *selection = Selection::caret(offset);
                Outcome::Handled
            }
            None => Outcome::Message(format!("pattern not found: {word}")),
        }
    }

    fn indent_current_lines(
        &mut self,
        outdent: bool,
        document: &Document,
        selection: &Selection,
    ) -> Outcome {
        let count = self.count();
        let first = document.line_of(selection.head);
        let last = LineIndex((first.0 + count - 1).min(document.len_lines().saturating_sub(1)));
        let key = Key::Char(if outdent { '<' } else { '>' });
        let keys = self.counted_keys(vec![key, key]);
        self.reset_count();
        self.record_change(keys, None);
        self.set_visual_anchor(None);
        self.set_mode(Mode::Normal);
        Outcome::Indent {
            outdent,
            first_line: first,
            last_line: last,
        }
    }

    fn repeat_search(
        &mut self,
        reverse: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some((needle, forward)) = self.last_search().cloned() else {
            return Outcome::Message("no previous search".to_owned());
        };
        let forward = if reverse { !forward } else { forward };
        match motion::find_literal(document, selection.head, &needle, forward) {
            Some(offset) => {
                *selection = Selection::caret(offset);
                Outcome::Handled
            }
            None => Outcome::Message(format!("pattern not found: {needle}")),
        }
    }

    fn repeat_last_change(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some(change) = self.last_change().cloned() else {
            return Outcome::Message("nothing to repeat".to_owned());
        };
        // Vim replaces the original count when `.` has a count of its own;
        // strip any recorded digits and prepend the requested count.
        let override_count = (self.count > 0).then(|| self.count());
        self.reset_count();
        let (keys, inserted) = change.into_parts();
        let keys = if let Some(count) = override_count {
            with_count_override(keys, count)
        } else {
            keys
        };
        self.begin_replay();
        let mut result = Outcome::Handled;
        for key in keys {
            let outcome = self.dispatch_direct(key, document, selection);
            let rejected = matches!(outcome, Outcome::Message(_));
            result = merge_outcome(result, outcome);
            if rejected {
                break;
            }
        }
        if self.mode() == Mode::Insert {
            if let Some(text) = inserted {
                let before = *selection;
                let caret = CharOffset(selection.head.0 + text.chars().count());
                let transaction = EditTransaction::new(
                    EditCause::Vim,
                    TextEdit::insertion(selection.head, text),
                    before,
                    Selection::caret(caret),
                );
                if document.apply(transaction).is_ok() {
                    *selection = Selection::caret(caret);
                    result = merge_outcome(result, Outcome::Changed);
                }
            }
            self.finish_insert(document, selection);
        }
        self.end_replay();
        result
    }

    pub(crate) fn finish_insert(&mut self, document: &mut Document, selection: &mut Selection) {
        let mut move_back = false;
        if let Some((start, keys, always_record, repeat)) = self.take_insert_session() {
            let inserted = if selection.head >= start {
                document.slice(CharRange::new(start, selection.head))
            } else {
                String::new()
            };
            let has_text = !inserted.is_empty();
            if has_text && repeat > 1 {
                let extra = inserted.repeat(repeat - 1);
                let before = *selection;
                let caret = CharOffset(selection.head.0 + extra.chars().count());
                if document
                    .apply(EditTransaction::new(
                        EditCause::Vim,
                        TextEdit::insertion(selection.head, extra),
                        before,
                        Selection::caret(caret),
                    ))
                    .is_ok()
                {
                    *selection = Selection::caret(caret);
                }
            }
            // `a`/`A` place the Insert caret after an existing character, so
            // leaving without typing still steps back onto that character.
            let entered_after = matches!(keys.first(), Some(Key::Char('a' | 'A')));
            move_back = has_text || entered_after;
            if !keys.is_empty() && (always_record || has_text) {
                self.record_change(keys, has_text.then_some(inserted));
            }
        }
        if move_back {
            let line = document.line_of(selection.head);
            let start = document.line_start(line);
            if selection.head > start {
                let target = document.prev_grapheme_boundary(selection.head);
                *selection = Selection::caret(motion::clamp_normal(document, target));
            }
        }
        document.end_user_operation();
        self.set_mode(Mode::Normal);
        self.clear_pending();
    }

    pub(crate) fn paste(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
        after: bool,
    ) -> Outcome {
        if self.register().is_empty() {
            self.reset_count();
            return Outcome::Message("register is empty".to_owned());
        }
        let keys = self.counted_keys(vec![Key::Char(if after { 'p' } else { 'P' })]);
        let text = self.register().text().repeat(self.count());
        let linewise = self.register().is_linewise();
        let before = *selection;
        let (at, insert, caret) = if linewise {
            let line = document.line_of(selection.head);
            if after {
                let next = LineIndex(line.0 + 1);
                if next.0 < document.len_lines() {
                    (
                        document.line_start(next),
                        text.clone(),
                        document.line_start(next),
                    )
                } else {
                    let end = document.line_content_end(line);
                    (end, format!("\n{text}"), CharOffset(end.0 + 1))
                }
            } else {
                let start = document.line_start(line);
                (start, text.clone(), start)
            }
        } else {
            let at = if after {
                let line = document.line_of(selection.head);
                let end = document.line_content_end(line);
                if selection.head < end {
                    document.next_grapheme_boundary(selection.head)
                } else {
                    selection.head
                }
            } else {
                selection.head
            };
            let caret = CharOffset(at.0 + text.chars().count().saturating_sub(1));
            (at, text.clone(), caret)
        };
        let transaction = EditTransaction::new(
            EditCause::Vim,
            TextEdit::insertion(at, insert),
            before,
            Selection::caret(caret),
        );
        let result = match document.apply(transaction) {
            Ok(_) => {
                *selection = Selection::caret(motion::clamp_normal(document, caret));
                self.record_change(keys, None);
                Outcome::Changed
            }
            Err(_) => Outcome::Handled,
        };
        self.reset_count();
        result
    }

    pub(crate) fn clear_pending(&mut self) {
        self.take_pending();
        self.reset_count();
    }

    pub(crate) fn set_pending(&mut self, pending: Option<Pending>) {
        *self.pending() = pending;
    }
}

fn find_direction(character: char) -> FindDirection {
    match character {
        'f' | 't' => FindDirection::Forward,
        _ => FindDirection::Backward,
    }
}

fn find_key(state: FindState) -> char {
    match (state.direction, state.till) {
        (FindDirection::Forward, false) => 'f',
        (FindDirection::Forward, true) => 't',
        (FindDirection::Backward, false) => 'F',
        (FindDirection::Backward, true) => 'T',
    }
}

fn motion_failure(motion: Motion) -> String {
    match motion {
        Motion::Find(state) => format!("'{}' not found on this line", state.character),
        Motion::MatchPair => "no matching delimiter".to_owned(),
        _ => "motion is not available".to_owned(),
    }
}

/// The graphemes covered by `count` substitutions or deletions, stopping at
/// the line's content end.
fn character_range(document: &Document, head: CharOffset, count: usize) -> CharRange {
    let line = document.line_of(head);
    let content_end = document.line_content_end(line);
    let mut end = head;
    for _ in 0..count {
        if end >= content_end {
            break;
        }
        let next = document.next_grapheme_boundary(end);
        if next > content_end {
            break;
        }
        end = next;
    }
    CharRange::new(head, end)
}

/// Replace the leading count digits of a recorded command. Vim uses a count
/// before `.` in place of the original command's count.
fn with_count_override(keys: Vec<Key>, count: usize) -> Vec<Key> {
    let rest = keys
        .into_iter()
        .skip_while(|key| matches!(key, Key::Char(character) if character.is_ascii_digit()));
    let mut overridden: Vec<Key> = count.to_string().chars().map(Key::Char).collect();
    overridden.extend(rest);
    overridden
}
