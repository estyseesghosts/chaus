//! Normal-mode grammar, motions, operators and Insert transitions.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};

use crate::engine::{CommandKind, Engine, LastChange, Outcome, Pending};
use crate::key::Key;
use crate::mode::Mode;
use crate::motion;
use crate::operator::{self, Operator};

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
                    self.set_changed();
                    Outcome::Changed
                } else {
                    Outcome::Handled
                }
            }
            Key::Char(character) => self.normal_char(character, document, selection),
            Key::Enter | Key::Backspace | Key::Tab | Key::Control(_) => Outcome::Handled,
        }
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
            'h' | 'j' | 'k' | 'l' | 'w' | 'b' | 'e' | '0' | '^' | '$' | 'G' => {
                self.move_caret(character, document, selection);
                Outcome::Handled
            }
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
                    self.set_changed();
                    Outcome::Changed
                } else {
                    Outcome::Handled
                }
            }
            '/' => self.start_command_line(CommandKind::Search { forward: true }),
            '?' => self.start_command_line(CommandKind::Search { forward: false }),
            ':' => self.start_command_line(CommandKind::Ex),
            'n' | 'N' => self.repeat_search(character == 'N', document, selection),
            '.' => self.repeat_last_change(document, selection),
            'f' => {
                self.set_pending(Some(Pending::Find));
                Outcome::Handled
            }
            ';' => {
                if let Some(character) = self.last_find() {
                    self.find_character(character, document, selection);
                }
                Outcome::Handled
            }
            _ => {
                self.message(format!("unknown command '{character}'"));
                self.reset_count();
                Outcome::Handled
            }
        }
    }

    fn pending_key(
        &mut self,
        pending: Pending,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        match pending {
            Pending::Operator(operator) => {
                if Operator::from_key(character) == Some(operator) {
                    return self.apply_linewise(operator, document, selection);
                }
                if character == 'i' || character == 'a' {
                    self.set_pending(Some(Pending::TextObject {
                        operator,
                        outer: character == 'a',
                    }));
                    return Outcome::Handled;
                }
                if let Some((range, linewise)) =
                    self.motion_range(document, selection.head, character)
                {
                    let keys = vec![Key::Char(operator.key()), Key::Char(character)];
                    return self.apply_operator(
                        operator,
                        document,
                        selection,
                        range,
                        linewise,
                        Some(keys),
                    );
                }
                self.reset_count();
                self.message("operator needs a motion");
                Outcome::Handled
            }
            Pending::G => {
                if character == 'g' {
                    let line = CharOffset::ZERO;
                    let line = document.line_of(line);
                    let target = if self.count > 0 {
                        LineIndex((self.count - 1).min(document.len_lines().saturating_sub(1)))
                    } else {
                        line
                    };
                    let caret = motion::first_non_space(document, target);
                    *selection = Selection::caret(motion::clamp_normal(document, caret));
                    self.reset_count();
                } else {
                    self.reset_count();
                    self.message("expected 'g' after 'g'");
                }
                Outcome::Handled
            }
            Pending::Find => {
                self.find_character(character, document, selection);
                Outcome::Handled
            }
            Pending::Replace => self.replace_characters(character, document, selection),
            Pending::TextObject { operator, outer } => {
                let range = match character {
                    'w' => Some(motion::word_object(document, selection.head, outer)),
                    '"' => motion::quoted_object(document, selection.head, '"', outer),
                    '\'' => motion::quoted_object(document, selection.head, '\'', outer),
                    '(' | ')' | 'b' => {
                        motion::bracketed_object(document, selection.head, '(', ')', outer)
                    }
                    '[' | ']' => {
                        motion::bracketed_object(document, selection.head, '[', ']', outer)
                    }
                    '{' | '}' => {
                        motion::bracketed_object(document, selection.head, '{', '}', outer)
                    }
                    _ => None,
                };
                match range {
                    Some(range) => {
                        let keys = vec![
                            Key::Char(operator.key()),
                            Key::Char(if outer { 'a' } else { 'i' }),
                            Key::Char(character),
                        ];
                        self.apply_operator(operator, document, selection, range, false, Some(keys))
                    }
                    None => {
                        self.reset_count();
                        self.message("no matching text object");
                        Outcome::Handled
                    }
                }
            }
        }
    }

    fn move_caret(&mut self, motion_key: char, document: &mut Document, selection: &mut Selection) {
        let count = self.count();
        if motion_key == 'G' {
            let line = if self.count > 0 {
                LineIndex((self.count - 1).min(document.len_lines().saturating_sub(1)))
            } else {
                LineIndex(document.len_lines().saturating_sub(1))
            };
            let caret = motion::first_non_space(document, line);
            *selection = Selection::caret(motion::clamp_normal(document, caret));
            self.reset_count();
            return;
        }
        let mut caret = selection.head;
        for _ in 0..count {
            caret = match motion_key {
                'h' => document.prev_grapheme_boundary(caret),
                'l' => document.next_grapheme_boundary(caret),
                'j' => motion::vertical(document, caret, 1),
                'k' => motion::vertical(document, caret, -1),
                'w' => motion::next_word_start(document.rope(), caret),
                'b' => motion::previous_word_start(document.rope(), caret),
                'e' => motion::word_end(document.rope(), caret),
                '0' => {
                    let line = document.line_of(caret);
                    document.line_start(line)
                }
                '^' => {
                    let line = document.line_of(caret);
                    motion::first_non_space(document, line)
                }
                '$' => {
                    let line = document.line_of(caret);
                    document.line_content_end(line)
                }
                _ => caret,
            };
        }
        *selection = Selection::caret(motion::clamp_normal(document, caret));
        self.reset_count();
    }

    fn motion_range(
        &self,
        document: &Document,
        caret: CharOffset,
        motion_key: char,
    ) -> Option<(CharRange, bool)> {
        let count = self.count();
        let mut target = caret;
        let mut inclusive = false;
        let mut linewise = false;
        match motion_key {
            'h' => {
                for _ in 0..count {
                    target = document.prev_grapheme_boundary(target);
                }
            }
            'l' => {
                for _ in 0..count {
                    target = document.next_grapheme_boundary(target);
                }
                inclusive = true;
            }
            'w' => {
                for _ in 0..count {
                    target = motion::next_word_start(document.rope(), target);
                }
            }
            'b' => {
                for _ in 0..count {
                    target = motion::previous_word_start(document.rope(), target);
                }
            }
            'e' => {
                for _ in 0..count {
                    target = motion::word_end(document.rope(), target);
                }
                inclusive = true;
            }
            '0' => {
                let line = document.line_of(caret);
                target = document.line_start(line);
            }
            '^' => {
                let line = document.line_of(caret);
                target = motion::first_non_space(document, line);
            }
            '$' => {
                let line = document.line_of(caret);
                target = document.line_content_end(line);
            }
            'j' | 'k' => {
                linewise = true;
                target = motion::vertical(
                    document,
                    caret,
                    if motion_key == 'j' {
                        count as isize
                    } else {
                        -(count as isize)
                    },
                );
            }
            'G' => {
                linewise = true;
                let line = if self.count > 0 {
                    LineIndex((self.count - 1).min(document.len_lines().saturating_sub(1)))
                } else {
                    LineIndex(document.len_lines().saturating_sub(1))
                };
                target = motion::first_non_space(document, line);
            }
            _ => return None,
        }
        if linewise {
            let from_line = document.line_of(caret);
            let to_line = document.line_of(target);
            let start = from_line.0.min(to_line.0);
            let end = from_line.0.max(to_line.0);
            return Some((
                motion::linewise_range(document, LineIndex(start), end - start + 1),
                true,
            ));
        }
        let start = caret.min(target);
        let mut end = caret.max(target);
        // An inclusive motion includes the character under the caret, but a
        // motion that stops at the line content end must not swallow the
        // newline (`dl` is `x`, not a line join).
        if inclusive && end.0 < document.len_chars() && document.rope().char(end.0) != '\n' {
            end = document.next_grapheme_boundary(end);
        }
        Some((CharRange::new(start, end), false))
    }

    fn apply_linewise(
        &mut self,
        operator: Operator,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let count = self.count();
        let line = document.line_of(selection.head);
        let mut range = motion::linewise_range(document, line, count);
        if operator == Operator::Change {
            let last = (line.0 + count - 1).min(document.len_lines().saturating_sub(1));
            range = CharRange::new(range.start, document.line_content_end(LineIndex(last)));
        }
        let keys = vec![Key::Char(operator.key()), Key::Char(operator.key())];
        self.apply_operator(operator, document, selection, range, true, Some(keys))
    }

    /// Apply a validated operator range. `keys` is the Normal-mode command
    /// that produced it, used by `.`; visual selections pass `None` because
    /// their range cannot be replayed from a new caret.
    fn apply_operator(
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
                    self.set_changed();
                    Outcome::Changed
                }
            }
            None => {
                if operator == Operator::Change {
                    document.end_user_operation();
                }
                self.message("edit was rejected");
                Outcome::Handled
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
                self.set_changed();
                Outcome::Changed
            }
            Err(_) => {
                document.end_user_operation();
                self.message("could not open a line");
                Outcome::Handled
            }
        }
    }

    fn delete_characters(&mut self, document: &mut Document, selection: &mut Selection) -> Outcome {
        let count = self.count();
        let line = document.line_of(selection.head);
        let content_end = document.line_content_end(line);
        let mut end = selection.head;
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
        let range = CharRange::new(selection.head, end);
        self.apply_operator(
            Operator::Delete,
            document,
            selection,
            range,
            false,
            Some(vec![Key::Char('x')]),
        )
    }

    fn replace_characters(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        if character == '\n' {
            self.message("'r' cannot insert a line break");
            self.reset_count();
            return Outcome::Handled;
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
            self.message("not enough characters on the line for 'r'");
            self.reset_count();
            return Outcome::Handled;
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
                self.set_changed();
                Outcome::Changed
            }
            Err(_) => Outcome::Handled,
        }
    }

    fn find_character(
        &mut self,
        character: char,
        document: &mut Document,
        selection: &mut Selection,
    ) {
        let line = document.line_of(selection.head);
        let start = document.line_start(line);
        let text = document.line_text(line);
        let local = selection.head.0.saturating_sub(start.0);
        let characters: Vec<char> = text.chars().collect();
        let mut remaining = self.count();
        let mut found = None;
        for (index, candidate) in characters.iter().enumerate().skip(local + 1) {
            if *candidate == character {
                remaining -= 1;
                if remaining == 0 {
                    found = Some(index);
                    break;
                }
            }
        }
        self.set_last_find(Some(character));
        self.reset_count();
        if let Some(index) = found {
            *selection = Selection::caret(CharOffset(start.0 + index));
        } else {
            self.message(format!("'{character}' not found on this line"));
        }
    }

    fn start_command_line(&mut self, kind: CommandKind) -> Outcome {
        self.set_mode(Mode::CommandLine);
        self.set_command_kind(kind);
        self.take_command();
        self.reset_count();
        Outcome::Handled
    }

    fn repeat_search(
        &mut self,
        reverse: bool,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some((needle, forward)) = self.last_search().cloned() else {
            self.message("no previous search");
            return Outcome::Handled;
        };
        let forward = if reverse { !forward } else { forward };
        if let Some(offset) = motion::find_literal(document, selection.head, &needle, forward) {
            *selection = Selection::caret(offset);
        } else {
            self.message(format!("pattern not found: {needle}"));
        }
        Outcome::Handled
    }

    fn repeat_last_change(
        &mut self,
        document: &mut Document,
        selection: &mut Selection,
    ) -> Outcome {
        let Some(change) = self.last_change().cloned() else {
            self.message("nothing to repeat");
            return Outcome::Handled;
        };
        // Vim replaces the original count when `.` has a count of its own;
        // strip any recorded digits and prepend the requested count.
        let override_count = (self.count > 0).then(|| self.count());
        self.reset_count();
        let LastChange { keys, inserted } = change;
        let keys = if let Some(count) = override_count {
            with_count_override(keys, count)
        } else {
            keys
        };
        self.begin_replay();
        for key in keys {
            let outcome = self.dispatch_direct(key, document, selection);
            if matches!(outcome, Outcome::Message(_)) {
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
                    self.set_changed();
                }
            }
            self.finish_insert(document, selection);
        }
        self.end_replay();
        Outcome::Handled
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
            self.message("register is empty");
            self.reset_count();
            return Outcome::Handled;
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
                self.set_changed();
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
