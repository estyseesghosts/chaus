//! Cursor motions and text objects, expressed in Rope character indices.
//!
//! Normal-mode carets rest *on* a scalar; an empty line allows the line start.
//! Nothing here mutates the document.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, CharRange, LineIndex};
use ropey::Rope;

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
    if let Some(character) = character_at(rope, CharOffset(index)) {
        if is_word_char(character) {
            while index < length && character_at(rope, CharOffset(index)).is_some_and(is_word_char)
            {
                index += 1;
            }
        } else if !character.is_whitespace() {
            while index < length
                && character_at(rope, CharOffset(index))
                    .is_some_and(|next| !is_word_char(next) && !next.is_whitespace())
            {
                index += 1;
            }
        }
    }
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    CharOffset(index)
}

/// End of the current or next word (`e`).
pub(crate) fn word_end(rope: &Rope, from: CharOffset) -> CharOffset {
    let length = rope.len_chars();
    let mut index = from.0.min(length);
    if index + 1 < length
        && character_at(rope, CharOffset(index)).is_some_and(is_word_char)
        && character_at(rope, CharOffset(index + 1)).is_some_and(is_word_char)
    {
        index += 1;
    }
    while index < length && character_at(rope, CharOffset(index)).is_some_and(char::is_whitespace) {
        index += 1;
    }
    while index + 1 < length
        && character_at(rope, CharOffset(index + 1)).is_some_and(|next| {
            character_at(rope, CharOffset(index)).is_some_and(|current| {
                is_word_char(current) == is_word_char(next) && !next.is_whitespace()
            })
        })
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
    let word = character_at(rope, CharOffset(index - 1)).is_some_and(is_word_char);
    while index > 0
        && character_at(rope, CharOffset(index - 1))
            .is_some_and(|character| !character.is_whitespace() && is_word_char(character) == word)
    {
        index -= 1;
    }
    CharOffset(index)
}

pub(crate) fn first_non_space(document: &Document, line: LineIndex) -> CharOffset {
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
    let text = document.text();
    let characters: Vec<char> = text.chars().collect();
    let mut index = offset.0.min(characters.len().saturating_sub(1));
    let mut depth = 0usize;
    let mut open_index = None;
    loop {
        if characters.get(index) == Some(&close) {
            depth += 1;
        } else if characters.get(index) == Some(&open) {
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
    if characters.get(open_index) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    let mut index = open_index;
    loop {
        if characters.get(index) == Some(&open) {
            depth += 1;
        } else if characters.get(index) == Some(&close) {
            depth -= 1;
            if depth == 0 {
                let start = if outer { open_index } else { open_index + 1 };
                let end = if outer { index + 1 } else { index };
                return Some(CharRange::new(CharOffset(start), CharOffset(end)));
            }
        }
        index += 1;
        if index >= characters.len() {
            return None;
        }
    }
}

/// First literal occurrence at or after `from`, wrapping to the start.
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
        let start_byte = document.char_to_byte(from).0.min(text.len());
        let found = text[start_byte..]
            .find(needle)
            .map(|offset| start_byte + offset)
            .or_else(|| text.find(needle));
        found.map(|byte| CharOffset(document.rope().byte_to_char(byte)))
    } else {
        let end_byte = document.char_to_byte(from).0.min(text.len());
        let found = text[..end_byte]
            .rfind(needle)
            .or_else(|| text.rfind(needle));
        found.map(|byte| CharOffset(document.rope().byte_to_char(byte)))
    }
}
