//! Cursor motions and text objects, expressed in Rope character indices.
//!
//! Normal mode, operator-pending mode and Visual mode all resolve a
//! [`Motion`] through [`resolve`]. The resolver returns both the caret target
//! and the half-open range an operator should use, so the three modes cannot
//! drift apart. Normal-mode carets rest *on* a scalar; an empty line allows the
//! line start. Nothing here mutates the document.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, CharRange, LineIndex, Selection};
use ropey::Rope;

/// A character-find direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindDirection {
    Forward,
    Backward,
}

impl FindDirection {
    /// The direction `;` or `,` repeats.
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }
}

/// The last `f`/`F`/`t`/`T`, as `;` and `,` need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FindState {
    pub(crate) character: char,
    pub(crate) direction: FindDirection,
    pub(crate) till: bool,
}

impl FindState {
    /// The same character with the direction reversed, for `,`.
    pub(crate) fn reversed(self) -> Self {
        Self {
            direction: self.direction.toggled(),
            ..self
        }
    }
}

/// A semantic motion, independent of the key sequence that produced it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordStart,
    WordStartBig,
    WordEnd,
    WordEndBig,
    WordBack,
    WordBackBig,
    WordEndBack,
    LineStart,
    LineFirstNonBlank,
    LineEnd,
    FirstLine,
    LastLine,
    Find(FindState),
    MatchPair,
}

impl Motion {
    /// Map a one-key Normal motion. `g`-prefixed and find motions are parsed
    /// by the engine, which owns the pending state.
    pub(crate) fn from_key(character: char) -> Option<Self> {
        match character {
            'h' => Some(Self::Left),
            'j' => Some(Self::Down),
            'k' => Some(Self::Up),
            'l' => Some(Self::Right),
            'w' => Some(Self::WordStart),
            'W' => Some(Self::WordStartBig),
            'b' => Some(Self::WordBack),
            'B' => Some(Self::WordBackBig),
            'e' => Some(Self::WordEnd),
            'E' => Some(Self::WordEndBig),
            '0' => Some(Self::LineStart),
            '^' => Some(Self::LineFirstNonBlank),
            '$' => Some(Self::LineEnd),
            'G' => Some(Self::LastLine),
            '%' => Some(Self::MatchPair),
            _ => None,
        }
    }

    /// Map the second key of a `g` motion.
    pub(crate) fn from_g(character: char) -> Option<Self> {
        match character {
            'g' => Some(Self::FirstLine),
            'e' => Some(Self::WordEndBack),
            _ => None,
        }
    }
}

/// What a motion resolved to for the caret and for an operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedMotion {
    /// Half-open text range an operator applies to. Linewise motions cover
    /// whole hard lines, including the trailing newline when one exists.
    pub(crate) range: CharRange,
    /// Where Normal and Visual mode put the caret.
    pub(crate) target: CharOffset,
    pub(crate) linewise: bool,
    /// Whether the character at the far end of the range belongs to the
    /// motion (`e`, `f`, `%`).
    pub(crate) inclusive: bool,
    /// Effective count after the engine's count multiplication.
    pub(crate) count: usize,
}

/// Resolve `motion` from `caret`. `explicit_count` distinguishes a plain `G`
/// (last line) from `1G` (line one); other motions only need `count`.
pub(crate) fn resolve(
    document: &Document,
    caret: CharOffset,
    motion: Motion,
    count: usize,
    explicit_count: bool,
) -> Option<ResolvedMotion> {
    let count = count.max(1);
    let caret = clamp_normal(document, caret);
    let result = |target: CharOffset, linewise: bool, inclusive: bool, extend_end: bool| {
        if linewise {
            let from = document.line_of(caret);
            let to = document.line_of(target);
            let first = from.0.min(to.0);
            let span = from.0.max(to.0) - first + 1;
            Some(ResolvedMotion {
                range: linewise_range(document, LineIndex(first), span),
                target,
                linewise: true,
                inclusive,
                count,
            })
        } else {
            Some(charwise_result(
                document, caret, target, inclusive, extend_end, count,
            ))
        }
    };

    match motion {
        Motion::Left => {
            let start = document.line_start(document.line_of(caret));
            let mut target = caret;
            for _ in 0..count {
                if target <= start {
                    break;
                }
                target = document.prev_grapheme_boundary(target);
            }
            result(target, false, false, false)
        }
        Motion::Right => {
            let end = document.line_content_end(document.line_of(caret));
            let mut target = caret;
            for _ in 0..count {
                if target >= end {
                    break;
                }
                target = document.next_grapheme_boundary(target).min(end);
            }
            result(target, false, false, false)
        }
        Motion::Down | Motion::Up => {
            let delta = if motion == Motion::Down {
                count as isize
            } else {
                -(count as isize)
            };
            let target = vertical(document, caret, delta);
            result(target, true, false, false)
        }
        Motion::WordStart => {
            let mut target = caret;
            for _ in 0..count {
                target = next_word_start(document.rope(), target);
            }
            result(target, false, false, false)
        }
        Motion::WordStartBig => {
            let mut target = caret;
            for _ in 0..count {
                target = next_big_word_start(document.rope(), target);
            }
            result(target, false, false, false)
        }
        Motion::WordEnd => {
            let mut target = caret;
            for _ in 0..count {
                target = word_end(document.rope(), target);
            }
            result(target, false, true, true)
        }
        Motion::WordEndBig => {
            let mut target = caret;
            for _ in 0..count {
                target = word_end_big(document.rope(), target);
            }
            result(target, false, true, true)
        }
        Motion::WordBack => {
            let mut target = caret;
            for _ in 0..count {
                target = previous_word_start(document.rope(), target);
            }
            result(target, false, false, false)
        }
        Motion::WordBackBig => {
            let mut target = caret;
            for _ in 0..count {
                target = previous_big_word_start(document.rope(), target);
            }
            result(target, false, false, false)
        }
        Motion::WordEndBack => {
            let mut target = caret;
            for _ in 0..count {
                target = word_end_back(document.rope(), target);
            }
            result(target, false, true, false)
        }
        Motion::LineStart => {
            let target = document.line_start(document.line_of(caret));
            result(target, false, false, false)
        }
        Motion::LineFirstNonBlank => {
            let target = first_non_space(document, document.line_of(caret));
            result(target, false, false, false)
        }
        Motion::LineEnd => {
            let line = document.line_of(caret);
            let last = (line.0 + count - 1).min(document.len_lines().saturating_sub(1));
            let target = document.line_content_end(LineIndex(last));
            result(target, false, false, false)
        }
        Motion::FirstLine => {
            let line = if explicit_count {
                (count - 1).min(document.len_lines().saturating_sub(1))
            } else {
                0
            };
            let target = first_non_space(document, LineIndex(line));
            result(target, true, false, false)
        }
        Motion::LastLine => {
            let line = if explicit_count {
                (count - 1).min(document.len_lines().saturating_sub(1))
            } else {
                document.len_lines().saturating_sub(1)
            };
            let target = first_non_space(document, LineIndex(line));
            result(target, true, false, false)
        }
        Motion::Find(state) => {
            let mut occurrence = FindOccurrence::current(caret, state);
            let mut target = None;
            for _ in 0..count {
                target = Some(occurrence.next(document)?);
            }
            let target = target?;
            result(target, false, true, target >= caret)
        }
        Motion::MatchPair => {
            let target = matching_delimiter(document, caret)?;
            result(target, false, true, true)
        }
    }
}

fn charwise_result(
    document: &Document,
    caret: CharOffset,
    target: CharOffset,
    inclusive: bool,
    extend_end: bool,
    count: usize,
) -> ResolvedMotion {
    let start = caret.min(target);
    let mut end = caret.max(target);
    // An inclusive motion covers the character under its far end. A backward
    // inclusive motion already includes its target when the range starts
    // there; extending the other end would swallow the caret character too.
    if extend_end
        && inclusive
        && end.0 < document.len_chars()
        && document.rope().char(end.0) != '\n'
    {
        end = document.next_grapheme_boundary(end);
    }
    ResolvedMotion {
        range: CharRange::new(start, end),
        target,
        linewise: false,
        inclusive,
        count,
    }
}

/// The hard lines a Visual selection covers. A non-empty selection ending
/// exactly at a line start does not include that next line.
pub(crate) fn selected_lines(document: &Document, selection: Selection) -> (LineIndex, LineIndex) {
    let range = selection.range();
    let first = document.line_of(range.start);
    if range.is_empty() {
        return (first, first);
    }
    let last = CharOffset(range.end.0.saturating_sub(1));
    (first, document.line_of(last))
}

pub(crate) fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn character_at(rope: &Rope, offset: CharOffset) -> Option<char> {
    if offset.0 < rope.len_chars() {
        Some(rope.char(offset.0))
    } else {
        None
    }
}

/// Two runs Vim distinguishes for lowercase word motions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunClass {
    Word,
    Punctuation,
}

fn run_class(character: char) -> Option<RunClass> {
    if character.is_whitespace() {
        None
    } else if is_word_char(character) {
        Some(RunClass::Word)
    } else {
        Some(RunClass::Punctuation)
    }
}

/// Keep a Normal-mode caret on a character, never past the line's content.
pub(crate) fn clamp_normal(document: &Document, offset: CharOffset) -> CharOffset {
    let offset = offset.min(CharOffset(document.len_chars()));
    let line = document.line_of(offset);
    let start = document.line_start(line);
    let end = document.line_content_end(line);
    if start == end {
        return start;
    }
    if offset >= end {
        return document.prev_grapheme_boundary(end);
    }
    offset
}

/// Skip to the start of the next Vim-style word.
pub(crate) fn next_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    if let Some(class) = character_at(rope, CharOffset(index)).and_then(run_class) {
        while index < length
            && character_at(rope, CharOffset(index)).and_then(run_class) == Some(class)
        {
            index += 1;
        }
    }
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    CharOffset(index)
}

/// Skip to the start of the next WORD (a non-whitespace run).
pub(crate) fn next_big_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    if character_at(rope, CharOffset(index)).is_some_and(|character| !character.is_whitespace()) {
        while index < length
            && character_at(rope, CharOffset(index))
                .is_some_and(|character| !character.is_whitespace())
        {
            index += 1;
        }
    }
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    CharOffset(index)
}

/// End of the current or next word (`e`). From the end of one run the motion
/// advances to the end of the next non-whitespace run, even when the class
/// changes (`foo` to `.`).
pub(crate) fn word_end(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    if index + 1 < length
        && !character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
        && character_at(rope, CharOffset(index + 1)).is_some_and(|next| !next.is_whitespace())
        && run_class(character_at(rope, CharOffset(index)).unwrap_or(' '))
            != run_class(character_at(rope, CharOffset(index + 1)).unwrap_or(' '))
    {
        index += 1;
    }
    while index + 1 < length
        && character_at(rope, CharOffset(index + 1)).is_some_and(|next| {
            character_at(rope, CharOffset(index)).is_some_and(|current| {
                run_class(current) == run_class(next) && !next.is_whitespace()
            })
        })
    {
        index += 1;
    }
    CharOffset(index.min(length.saturating_sub(1)))
}

/// End of the current or next WORD (`E`).
pub(crate) fn word_end_big(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    while index + 1 < length
        && character_at(rope, CharOffset(index + 1)).is_some_and(|next| !next.is_whitespace())
        && !character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace)
    {
        index += 1;
    }
    CharOffset(index.min(length.saturating_sub(1)))
}

/// Start of the previous word (`b`).
pub(crate) fn previous_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let mut index = from.0.min(rope.len_chars());
    while index > 0 && character_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace) {
        index -= 1;
    }
    if index == 0 {
        return CharOffset(0);
    }
    let class = character_at(rope, CharOffset(index - 1)).and_then(run_class);
    while index > 0
        && character_at(rope, CharOffset(index - 1)).and_then(run_class) == class
        && !character_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    CharOffset(index)
}

/// Start of the previous WORD (`B`).
pub(crate) fn previous_big_word_start(rope: &Rope, from: CharOffset) -> CharOffset {
    let mut index = from.0.min(rope.len_chars());
    while index > 0 && character_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace) {
        index -= 1;
    }
    while index > 0
        && character_at(rope, CharOffset(index - 1))
            .is_some_and(|character| !character.is_whitespace())
    {
        index -= 1;
    }
    CharOffset(index)
}

/// Backward to the end of the previous word (`ge`). From the middle of a run
/// the current run is skipped first, so the motion always moves backward.
pub(crate) fn word_end_back(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    if length == 0 {
        return CharOffset::ZERO;
    }
    let mut index = from.0.min(length - 1);
    if index == 0 {
        return CharOffset::ZERO;
    }
    if character_at(rope, CharOffset(index)).is_some_and(|character| !character.is_whitespace()) {
        let class = character_at(rope, CharOffset(index)).and_then(run_class);
        while index > 0
            && character_at(rope, CharOffset(index - 1)).and_then(run_class) == class
            && !character_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
        {
            index -= 1;
        }
    }
    if index == 0 {
        return CharOffset::ZERO;
    }
    index -= 1;
    while index > 0 && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index -= 1;
    }
    if character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        return CharOffset(index);
    }
    let class = character_at(rope, CharOffset(index)).and_then(run_class);
    while index > 0
        && character_at(rope, CharOffset(index - 1)).and_then(run_class) == class
        && !character_at(rope, CharOffset(index - 1)).is_some_and(char::is_whitespace)
    {
        index -= 1;
    }
    let mut end = index;
    while end + 1 < length
        && character_at(rope, CharOffset(end + 1)).and_then(run_class) == class
        && !character_at(rope, CharOffset(end + 1)).is_some_and(char::is_whitespace)
    {
        end += 1;
    }
    CharOffset(end)
}

pub fn first_non_space(document: &Document, line: LineIndex) -> CharOffset {
    let text = document.line_text(line);
    let offset = text
        .char_indices()
        .find(|(_, character)| *character != ' ' && *character != '\t')
        .map(|(byte, _)| text[..byte].chars().count())
        .unwrap_or(0);
    CharOffset(document.line_start(line).0 + offset)
}

/// Move a caret vertically, keeping its character column.
pub(crate) fn vertical(document: &Document, offset: CharOffset, delta: isize) -> CharOffset {
    let line = document.line_of(offset);
    let column = offset.0.saturating_sub(document.line_start(line).0);
    let target =
        (line.0 as isize + delta).clamp(0, document.len_lines().saturating_sub(1) as isize);
    let target = LineIndex(target as usize);
    let start = document.line_start(target);
    let end = document.line_content_end(target);
    let maximum = end.0.saturating_sub(start.0);
    CharOffset(start.0 + column.min(maximum))
}

/// Whole hard lines, including the trailing newline when one exists.
pub(crate) fn linewise_range(document: &Document, line: LineIndex, count: usize) -> CharRange {
    let last = (line.0 + count.max(1) - 1).min(document.len_lines().saturating_sub(1));
    let start = document.line_start(line);
    let end = if last + 1 < document.len_lines() {
        document.line_start(LineIndex(last + 1))
    } else {
        document.line_content_end(LineIndex(last))
    };
    CharRange::new(start, end)
}

/// A word text object (`iw`/`aw`).
pub(crate) fn word_object(document: &Document, offset: CharOffset, outer: bool) -> CharRange {
    let rope = document.rope();
    let length = document.len_chars();
    if length == 0 {
        return CharRange::empty(offset);
    }
    let mut start = offset.0.min(length - 1);
    let word = character_at(rope, CharOffset(start)).is_some_and(is_word_char);
    while start > 0
        && character_at(rope, CharOffset(start - 1))
            .is_some_and(|character| !character.is_whitespace() && is_word_char(character) == word)
    {
        start -= 1;
    }
    let mut end = offset.0.min(length - 1);
    while end < length
        && character_at(rope, CharOffset(end))
            .is_some_and(|character| !character.is_whitespace() && is_word_char(character) == word)
    {
        end += 1;
    }
    if outer {
        while end < length
            && character_at(rope, CharOffset(end)).is_some_and(char::is_whitespace)
            && character_at(rope, CharOffset(end)) != Some('\n')
        {
            end += 1;
        }
    }
    CharRange::new(CharOffset(start), CharOffset(end))
}

/// A quoted text object. Delimiters must be on one line.
pub(crate) fn quoted_object(
    document: &Document,
    offset: CharOffset,
    quote: char,
    outer: bool,
) -> Option<CharRange> {
    let line = document.line_of(offset);
    let start = document.line_start(line);
    let text = document.line_text(line);
    let local = offset.0.saturating_sub(start.0);
    let characters: Vec<char> = text.chars().collect();
    let mut open = None;
    let mut close = None;
    for (index, character) in characters.iter().enumerate() {
        if *character != quote {
            continue;
        }
        if index <= local
            && characters[..index]
                .iter()
                .filter(|ch| **ch == quote)
                .count()
                % 2
                == 0
        {
            open = Some(index);
        } else if index > local {
            close = Some(index);
            break;
        }
    }
    match (open, close) {
        (Some(open), Some(close)) => {
            let range_start = if outer { open } else { open + 1 };
            let range_end = if outer { close + 1 } else { close };
            Some(CharRange::new(
                CharOffset(start.0 + range_start),
                CharOffset(start.0 + range_end),
            ))
        }
        _ => None,
    }
}

/// A bracket text object, honoring nesting.
pub(crate) fn bracketed_object(
    document: &Document,
    offset: CharOffset,
    open: char,
    close: char,
    outer: bool,
) -> Option<CharRange> {
    let rope = document.rope();
    let length = rope.len_chars();
    let character_at = |index: usize| (index < length).then(|| rope.char(index));
    let mut index = offset.0.min(length.saturating_sub(1));
    let mut depth = 0usize;
    let mut open_index = None;
    loop {
        if character_at(index) == Some(close) {
            depth += 1;
        } else if character_at(index) == Some(open) {
            if depth == 0 {
                open_index = Some(index);
                break;
            }
            depth -= 1;
        }
        if index == 0 {
            break;
        }
        index -= 1;
    }
    let open_index = open_index?;
    let mut depth = 0usize;
    let mut index = open_index;
    loop {
        if character_at(index) == Some(open) {
            depth += 1;
        } else if character_at(index) == Some(close) {
            depth -= 1;
            if depth == 0 {
                let start = if outer { open_index } else { open_index + 1 };
                let end = if outer { index + 1 } else { index };
                return Some(CharRange::new(CharOffset(start), CharOffset(end)));
            }
        }
        index += 1;
        if index >= length {
            return None;
        }
    }
}

/// Next literal occurrence strictly after (or before) `from`, wrapping at the
/// document edge. Skipping the caret is what makes `n` advance instead of
/// finding the current match again.
pub(crate) fn find_literal(
    document: &Document,
    from: CharOffset,
    needle: &str,
    forward: bool,
) -> Option<CharOffset> {
    if needle.is_empty() {
        return None;
    }
    let text = document.text();
    if forward {
        let start = document.next_grapheme_boundary(from);
        let start_byte = document.char_to_byte(start).0.min(text.len());
        text[start_byte..]
            .find(needle)
            .map(|offset| start_byte + offset)
            .or_else(|| text.find(needle))
            .map(|byte| CharOffset(document.rope().byte_to_char(byte)))
    } else {
        let end = document.prev_grapheme_boundary(from);
        let end_byte = document.char_to_byte(end).0.min(text.len());
        text[..end_byte]
            .rfind(needle)
            .or_else(|| text.rfind(needle))
            .map(|byte| CharOffset(document.rope().byte_to_char(byte)))
    }
}

/// The word run under or nearest to the caret, for `*` and `#`.
pub(crate) fn word_near(document: &Document, offset: CharOffset) -> Option<(String, CharOffset)> {
    let length = document.len_chars();
    if length == 0 {
        return None;
    }
    let rope = document.rope();
    let index = offset.0.min(length - 1);
    if character_at(rope, CharOffset(index)).is_some_and(is_word_char) {
        return Some(word_run_at(document, index));
    }
    let mut forward = index;
    while forward < length {
        if character_at(rope, CharOffset(forward)).is_some_and(is_word_char) {
            return Some(word_run_at(document, forward));
        }
        forward += 1;
    }
    let mut backward = index;
    while backward > 0 {
        backward -= 1;
        if character_at(rope, CharOffset(backward)).is_some_and(is_word_char) {
            return Some(word_run_at(document, backward));
        }
    }
    None
}

fn word_run_at(document: &Document, index: usize) -> (String, CharOffset) {
    let rope = document.rope();
    let length = rope.len_chars();
    let mut start = index;
    let mut end = index;
    while start > 0 && character_at(rope, CharOffset(start - 1)).is_some_and(is_word_char) {
        start -= 1;
    }
    while end < length && character_at(rope, CharOffset(end)).is_some_and(is_word_char) {
        end += 1;
    }
    let text = document.slice(CharRange::new(CharOffset(start), CharOffset(end)));
    (text, CharOffset(start))
}

/// A bounded `%` matcher for `()`, `[]` and `{}`.
const MAX_DELIMITER_SCAN: usize = 1 << 20;
const MAX_DELIMITER_DEPTH: usize = 1024;

fn matching_delimiter(document: &Document, caret: CharOffset) -> Option<CharOffset> {
    let rope = document.rope();
    let length = rope.len_chars();
    if length == 0 {
        return None;
    }
    let at = |index: usize| (index < length).then(|| rope.char(index));
    let delimiter = |character: char| matches!(character, '(' | ')' | '[' | ']' | '{' | '}');
    let pair = |character: char| match character {
        '(' => Some(('(', ')')),
        ')' => Some(('(', ')')),
        '[' => Some(('[', ']')),
        ']' => Some(('[', ']')),
        '{' => Some(('{', '}')),
        '}' => Some(('{', '}')),
        _ => None,
    };

    let start = caret.0.min(length - 1);
    let mut anchor = None;
    if at(start).is_some_and(delimiter) {
        anchor = Some(start);
    } else {
        let line = document.line_of(caret);
        let line_start = document.line_start(line).0;
        let line_end = document.line_content_end(line).0;
        for index in start..line_end {
            if at(index).is_some_and(delimiter) {
                anchor = Some(index);
                break;
            }
        }
        if anchor.is_none() {
            for index in (line_start..start).rev() {
                if at(index).is_some_and(delimiter) {
                    anchor = Some(index);
                    break;
                }
            }
        }
    }
    let anchor = anchor?;
    let (open, close) = pair(at(anchor)?)?;
    let opening = at(anchor) == Some(open);
    if opening {
        let mut depth = 0usize;
        let limit = (anchor + MAX_DELIMITER_SCAN).min(length);
        for index in anchor..limit {
            match at(index) {
                Some(character) if character == open => {
                    depth += 1;
                    if depth > MAX_DELIMITER_DEPTH {
                        return None;
                    }
                }
                Some(character) if character == close => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(CharOffset(index));
                    }
                }
                _ => {}
            }
        }
        None
    } else {
        let mut depth = 0usize;
        let low = anchor.saturating_sub(MAX_DELIMITER_SCAN);
        for index in (low..=anchor).rev() {
            match at(index) {
                Some(character) if character == close => {
                    depth += 1;
                    if depth > MAX_DELIMITER_DEPTH {
                        return None;
                    }
                }
                Some(character) if character == open => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(CharOffset(index));
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// Stateful find used by the motion resolver. Owns the next search position so
/// a count can advance through several occurrences on the same line.
#[derive(Clone, Copy, Debug)]
struct FindOccurrence {
    character: char,
    direction: FindDirection,
    till: bool,
    position: CharOffset,
}

impl FindOccurrence {
    fn current(caret: CharOffset, state: FindState) -> Self {
        Self {
            character: state.character,
            direction: state.direction,
            till: state.till,
            position: caret,
        }
    }

    fn next(&mut self, document: &Document) -> Option<CharOffset> {
        let line = document.line_of(self.position);
        let start = document.line_start(line).0;
        let end = document.line_content_end(line).0;
        let local = self
            .position
            .0
            .saturating_sub(start)
            .min(end.saturating_sub(start));
        let text = document.line_text(line);
        let characters: Vec<char> = text.chars().collect();
        let found = match self.direction {
            FindDirection::Forward => characters
                .iter()
                .enumerate()
                .skip(local + 1)
                .find(|(_, candidate)| **candidate == self.character)
                .map(|(index, _)| index),
            FindDirection::Backward => {
                if local == 0 {
                    None
                } else {
                    characters[..local]
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, candidate)| **candidate == self.character)
                        .map(|(index, _)| index)
                }
            }
        }?;
        let target = if self.till {
            match self.direction {
                FindDirection::Forward => found.checked_sub(1)?,
                FindDirection::Backward => found + 1,
            }
        } else {
            found
        };
        self.position = CharOffset(start + found);
        Some(CharOffset(start + target))
    }
}
