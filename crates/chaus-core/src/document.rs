//! The document: a Ropey text buffer, newline metadata, transactions and
//! undo/redo history.
//!
//! All edits go through [`Document::apply`]. The editor view never mutates the
//! Rope directly, so typing, paste, IME, Vim and formatters share one undo
//! model and one revision stream for caches and background parsers.

use std::cell::RefCell;
use std::ops::Range;
use std::time::{Duration, Instant};

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;

use crate::eol::{Eol, EolMap};
use crate::position::{
    ByteOffset, CharOffset, CharRange, DocumentRevision, LineIndex, Selection, Utf16Offset,
};

/// How long contiguous typing keeps coalescing into one undo entry.
const TYPING_COALESCE_WINDOW: Duration = Duration::from_millis(750);

/// The origin of an edit. History grouping and dirty tracking depend on it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCause {
    Typing,
    Delete,
    Paste,
    ImeCommit,
    Vim,
    Formatter,
    ReplaceAll,
    Indent,
}

/// One replacement in original-document coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: CharRange,
    pub insert: String,
}

impl TextEdit {
    #[must_use]
    pub fn new(range: CharRange, insert: impl Into<String>) -> Self {
        Self {
            range,
            insert: insert.into(),
        }
    }

    #[must_use]
    pub fn insertion(at: CharOffset, text: impl Into<String>) -> Self {
        Self::new(CharRange::empty(at), text)
    }

    #[must_use]
    pub fn deletion(range: CharRange) -> Self {
        Self::new(range, String::new())
    }

    #[must_use]
    pub fn inserted_chars(&self) -> usize {
        self.insert.chars().count()
    }
}

/// A set of disjoint edits applied as one user operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditTransaction {
    pub edits: Vec<TextEdit>,
    pub before_selection: Selection,
    pub after_selection: Selection,
    pub cause: EditCause,
}

impl EditTransaction {
    #[must_use]
    pub fn new(cause: EditCause, edit: TextEdit, before: Selection, after: Selection) -> Self {
        Self {
            edits: vec![edit],
            before_selection: before,
            after_selection: after,
            cause,
        }
    }

    /// A single insertion replacing `selection`.
    #[must_use]
    pub fn replace_selection(
        cause: EditCause,
        selection: Selection,
        text: impl Into<String>,
    ) -> Self {
        let text = text.into();
        let end = CharOffset(selection.range().start.0 + text.chars().count());
        Self::new(
            cause,
            TextEdit::new(selection.range(), text),
            selection,
            Selection::caret(end),
        )
    }
}

/// A row/UTF-8-byte-column point, the unit Tree-sitter consumes.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPoint {
    pub row: usize,
    pub column_bytes: usize,
}

/// The effect of one applied edit, in pre-edit and final post-edit
/// coordinates. Multiple edits in one transaction are rebased after all edits
/// so every range refers to the final document.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct AppliedEdit {
    pub old_range: CharRange,
    pub new_range: CharRange,
    pub old_byte_range: Range<usize>,
    pub new_byte_range: Range<usize>,
    pub old_start_point: TextPoint,
    pub old_end_point: TextPoint,
    pub new_end_point: TextPoint,
    pub removed_text: String,
    pub inserted_text: String,
}

/// Result of a transaction, used to invalidate caches and update parsers.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct EditResult {
    pub revision: DocumentRevision,
    pub edits: Vec<AppliedEdit>,
    pub affected_lines: LineRange,
}

/// A half-open span of hard lines.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineRange {
    pub start: LineIndex,
    pub end: LineIndex,
}

impl LineRange {
    #[must_use]
    pub fn new(start: LineIndex, end: LineIndex) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    #[must_use]
    pub fn contains(self, line: LineIndex) -> bool {
        self.start <= line && line < self.end
    }
}

/// Reasons a transaction may be rejected. The document is left untouched on
/// every error.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    OutOfBounds { offset: CharOffset, length: usize },
    OverlappingEdits { first: CharRange, second: CharRange },
    EmptyTransaction,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::OutOfBounds { offset, length } => {
                write!(
                    formatter,
                    "offset {offset} is beyond document length {length}"
                )
            }
            EditError::OverlappingEdits { first, second } => write!(
                formatter,
                "edits overlap: {}..{} and {}..{}",
                first.start, first.end, second.start, second.end
            ),
            EditError::EmptyTransaction => {
                write!(formatter, "transaction contains no text changes")
            }
        }
    }
}

impl std::error::Error for EditError {}

fn utf16_len(characters: impl Iterator<Item = char>) -> usize {
    characters.map(char::len_utf16).sum()
}

/// Per-line UTF-16 lengths with a lazily rebuilt prefix sum. Edits patch only
/// the affected lines; the prefix is recomputed on demand (O(lines), not
/// O(text)).
#[derive(Debug, Default)]
struct Utf16Index {
    line_lengths: Vec<usize>,
    prefix: Vec<usize>,
    prefix_valid: bool,
}

impl Utf16Index {
    fn rebuild(&mut self, rope: &Rope) {
        self.line_lengths.clear();
        for line in rope.lines() {
            self.line_lengths.push(utf16_len(line.chars()));
        }
        self.prefix_valid = false;
    }

    fn ensure_prefix(&mut self) {
        if self.prefix_valid {
            return;
        }
        self.prefix.clear();
        self.prefix.reserve(self.line_lengths.len() + 1);
        let mut total = 0;
        for len in &self.line_lengths {
            self.prefix.push(total);
            total += len;
        }
        self.prefix.push(total);
        self.prefix_valid = true;
    }

    fn char_to_utf16(&mut self, rope: &Rope, offset: CharOffset) -> Utf16Offset {
        self.ensure_prefix();
        let line = rope.char_to_line(offset.0.min(rope.len_chars()));
        let line_start = rope.line_to_char(line);
        let prefix = self.prefix.get(line).copied().unwrap_or(0);
        let mut units = prefix;
        for ch in rope.line(line).chars().take(offset.0 - line_start) {
            units += ch.len_utf16();
        }
        Utf16Offset(units)
    }

    fn utf16_to_char(&mut self, rope: &Rope, offset: Utf16Offset) -> CharOffset {
        self.ensure_prefix();
        let length = rope.len_lines();
        let total = self.prefix.last().copied().unwrap_or(0);
        let target = offset.0.min(total);
        if length == 0 {
            return CharOffset::ZERO;
        }
        // Several lines can start at the same UTF-16 offset when blank lines
        // precede the target. Pick the *first* matching line; otherwise a
        // lookup at offset zero would land after the leading newline.
        let first_at_or_after = self.prefix.partition_point(|start| *start < target);
        let line = match self.prefix.get(first_at_or_after) {
            Some(start) if *start == target => first_at_or_after,
            _ => first_at_or_after.saturating_sub(1),
        }
        .min(length - 1);
        let line_start = rope.line_to_char(line);
        let mut units = self.prefix.get(line).copied().unwrap_or(0);
        let mut chars = 0;
        for ch in rope.line(line).chars() {
            if units >= target {
                break;
            }
            let next = units + ch.len_utf16();
            if next > target {
                // Inside a surrogate pair: clamp to the scalar boundary.
                break;
            }
            units = next;
            chars += 1;
        }
        CharOffset(line_start + chars)
    }
}

#[derive(Debug)]
struct UndoStep {
    edit: TextEdit,
    removed: String,
    eol: EolOp,
}

#[derive(Debug)]
struct EolOp {
    break_index: usize,
    removed: Vec<Eol>,
    inserted: Vec<Eol>,
}

#[derive(Debug)]
struct HistoryEntry {
    /// Steps in the order they were applied (descending char offsets within
    /// each transaction, transactions in chronological order).
    steps: Vec<UndoStep>,
    before_selection: Selection,
    after_selection: Selection,
    cause: EditCause,
    time: Instant,
    /// Identity of the history state before and after this entry. State
    /// identities are unique, unlike undo-stack depths, so branching and
    /// coalescing cannot make a different state look saved.
    before_state: u64,
    after_state: u64,
}

impl HistoryEntry {
    fn undo(&self, document: &mut Document) {
        for step in self.steps.iter().rev() {
            document.apply_inverse(step);
        }
    }

    fn redo(&self, document: &mut Document) {
        for step in &self.steps {
            document.apply_forward(step);
        }
    }
}

/// A cheap, immutable snapshot of document text and newline metadata for a
/// background save. History and selection are deliberately excluded.
#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    rope: Rope,
    eol: EolMap,
    preferred_eol: Eol,
}

impl DocumentSnapshot {
    /// Reconstruct the original newline kinds without touching the live
    /// document.
    #[must_use]
    pub fn text_with_eol(&self) -> String {
        self.eol.expand(self.rope.chars(), self.preferred_eol)
    }
}

/// A text document with one window's editing state.
#[derive(Debug)]
pub struct Document {
    rope: Rope,
    revision: DocumentRevision,
    eol: EolMap,
    preferred_eol: Eol,
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    current_state: u64,
    saved_state: u64,
    next_state: u64,
    group_depth: usize,
    group_start_depth: usize,
    utf16: RefCell<Utf16Index>,
}

impl Document {
    /// Create an empty UTF-8 document with `\n` breaks.
    #[must_use]
    pub fn new() -> Self {
        Self::from_normalized_text(String::new(), EolMap::default(), Eol::Lf)
    }

    /// Create a document from text whose hard breaks have already been
    /// normalized to `\n`.
    ///
    /// # Panics
    /// Panics if `text` still contains `\r`, or if the EOL map does not have
    /// exactly one entry per hard break.
    #[must_use]
    pub fn from_normalized_text(text: String, eol: EolMap, preferred_eol: Eol) -> Self {
        assert!(
            !text.contains('\r'),
            "document text must be normalized to \\n before construction"
        );
        let break_count = text.chars().filter(|c| *c == '\n').count();
        assert_eq!(
            eol.len(),
            break_count,
            "EOL map must have one entry per hard break"
        );
        let rope = Rope::from_str(&text);
        let mut utf16 = Utf16Index::default();
        utf16.rebuild(&rope);
        Self {
            rope,
            revision: DocumentRevision::ZERO,
            eol,
            preferred_eol,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_state: 0,
            saved_state: 0,
            next_state: 1,
            group_depth: 0,
            group_start_depth: 0,
            utf16: RefCell::new(utf16),
        }
    }

    /// Build from arbitrary newline text, recording each original break kind.
    #[must_use]
    pub fn from_source_text(source: &str) -> Self {
        let preferred = Eol::detect(source);
        let (text, eol) = EolMap::from_source(source);
        Self::from_normalized_text(text, eol, preferred)
    }

    #[must_use]
    pub fn revision(&self) -> DocumentRevision {
        self.revision
    }

    #[must_use]
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    #[must_use]
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The full document text. Intended for save/snapshot paths, not paint.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    #[must_use]
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    #[must_use]
    pub fn eol_map(&self) -> &EolMap {
        &self.eol
    }

    #[must_use]
    pub fn preferred_eol(&self) -> Eol {
        self.preferred_eol
    }

    pub fn set_preferred_eol(&mut self, eol: Eol) {
        self.preferred_eol = eol;
    }

    /// Text with the original newline kinds reconstructed, for save.
    ///
    /// This expands directly from the rope chunks so saving does not first
    /// build and then copy a second full-document string.
    #[must_use]
    pub fn text_with_eol(&self) -> String {
        self.eol.expand(self.rope.chars(), self.preferred_eol)
    }

    #[must_use]
    pub fn slice(&self, range: CharRange) -> String {
        let start = range.start.0.min(self.rope.len_chars());
        let end = range.end.0.min(self.rope.len_chars());
        self.rope.slice(start..end).to_string()
    }

    /// Find a literal string from `from`, wrapping at the document edge.
    ///
    /// Forward search includes a match starting at `from`; backward search
    /// requires the match to start before it. Callers advancing past the
    /// current match pass the end (forward) or start (backward) of that match.
    #[must_use]
    pub fn find_literal(
        &self,
        needle: &str,
        from: CharOffset,
        forward: bool,
        case_sensitive: bool,
    ) -> Option<CharOffset> {
        if needle.is_empty() {
            return None;
        }
        let text = self.text();
        let from_byte = self.char_to_byte(from).0.min(text.len());
        if case_sensitive {
            let found = if forward {
                text[from_byte..]
                    .find(needle)
                    .map(|offset| from_byte + offset)
                    .or_else(|| text.find(needle))
            } else {
                text[..from_byte]
                    .rfind(needle)
                    .or_else(|| text.rfind(needle))
            };
            return found.map(|byte| CharOffset(self.rope().byte_to_char(byte)));
        }

        let needle_chars: Vec<char> = needle.chars().collect();
        let matches_at = |byte: usize| {
            let mut candidates = text[byte..].chars();
            needle_chars.iter().all(|needle| {
                candidates
                    .next()
                    .is_some_and(|candidate| candidate.to_lowercase().eq(needle.to_lowercase()))
            })
        };
        let found = if forward {
            text[from_byte..]
                .char_indices()
                .find(|(offset, _)| matches_at(from_byte + offset))
                .map(|(offset, _)| from_byte + offset)
                .or_else(|| {
                    text.char_indices()
                        .find(|(offset, _)| matches_at(*offset))
                        .map(|(offset, _)| offset)
                })
        } else {
            text[..from_byte]
                .char_indices()
                .rev()
                .find(|(offset, _)| matches_at(*offset))
                .map(|(offset, _)| offset)
                .or_else(|| {
                    text.char_indices()
                        .rev()
                        .find(|(offset, _)| matches_at(*offset))
                        .map(|(offset, _)| offset)
                })
        };
        found.map(|byte| CharOffset(self.rope().byte_to_char(byte)))
    }

    /// Every non-overlapping literal match in document order.
    #[must_use]
    pub fn literal_ranges(&self, needle: &str, case_sensitive: bool) -> Vec<CharRange> {
        if needle.is_empty() {
            return Vec::new();
        }
        let text = self.text();
        let needle_chars: Vec<char> = needle.chars().collect();
        let matches_at = |byte: usize| {
            let mut candidates = text[byte..].chars();
            needle_chars.iter().all(|needle| {
                if case_sensitive {
                    candidates.next() == Some(*needle)
                } else {
                    candidates
                        .next()
                        .is_some_and(|candidate| candidate.to_lowercase().eq(needle.to_lowercase()))
                }
            })
        };
        let mut ranges = Vec::new();
        let mut offset = 0;
        while offset < text.len() {
            if matches_at(offset) {
                let start = CharOffset(self.rope().byte_to_char(offset));
                let end = CharOffset(start.0 + needle_chars.len());
                ranges.push(CharRange::new(start, end));
                offset += text[offset..]
                    .chars()
                    .take(needle_chars.len())
                    .map(char::len_utf8)
                    .sum::<usize>();
            } else {
                offset += text[offset..].chars().next().map_or(1, char::len_utf8);
            }
        }
        ranges
    }

    /// Text of a line, excluding its trailing hard break.
    #[must_use]
    pub fn line_text(&self, line: LineIndex) -> String {
        let start = self.rope.line_to_char(line.0);
        let end = self.line_content_end(line);
        self.rope.slice(start..end.0).to_string()
    }

    #[must_use]
    pub fn line_content_end(&self, line: LineIndex) -> CharOffset {
        let slice = self.rope.line(line.0);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }
        CharOffset(self.rope.line_to_char(line.0) + len)
    }

    #[must_use]
    pub fn line_start(&self, line: LineIndex) -> CharOffset {
        CharOffset(self.rope.line_to_char(line.0))
    }

    #[must_use]
    pub fn line_of(&self, offset: CharOffset) -> LineIndex {
        LineIndex(self.rope.char_to_line(offset.0.min(self.len_chars())))
    }

    #[must_use]
    pub fn char_to_byte(&self, offset: CharOffset) -> ByteOffset {
        ByteOffset(self.rope.char_to_byte(offset.0.min(self.len_chars())))
    }

    #[must_use]
    pub fn char_to_utf16(&self, offset: CharOffset) -> Utf16Offset {
        self.utf16.borrow_mut().char_to_utf16(&self.rope, offset)
    }

    #[must_use]
    pub fn utf16_to_char(&self, offset: Utf16Offset) -> CharOffset {
        self.utf16.borrow_mut().utf16_to_char(&self.rope, offset)
    }

    #[must_use]
    pub fn char_range_to_utf16(&self, range: CharRange) -> Range<usize> {
        let start = self.char_to_utf16(range.start).0;
        let end = self.char_to_utf16(range.end).0;
        start..end
    }

    #[must_use]
    pub fn utf16_range_to_char(&self, range: Range<usize>) -> CharRange {
        let start = self.utf16_to_char(Utf16Offset(range.start));
        let end = self.utf16_to_char(Utf16Offset(range.end));
        CharRange::new(start, end)
    }

    /// Previous grapheme boundary strictly before `offset`.
    #[must_use]
    pub fn prev_grapheme_boundary(&self, offset: CharOffset) -> CharOffset {
        let offset = offset.min(CharOffset(self.len_chars()));
        if offset.0 == 0 {
            return CharOffset::ZERO;
        }
        let line = self.line_of(offset);
        let line_start = self.line_start(line);
        if offset <= line_start {
            return line_start.saturating_sub(1);
        }
        let content_end = self.line_content_end(line);
        let text = self.rope.slice(line_start.0..content_end.0).to_string();
        let mut previous = line_start;
        let mut char_index = 0;
        for grapheme in text.graphemes(true) {
            let start = CharOffset(line_start.0 + char_index);
            if start >= offset {
                break;
            }
            previous = start;
            char_index += grapheme.chars().count();
        }
        previous
    }

    /// Next grapheme boundary strictly after `offset`.
    #[must_use]
    pub fn next_grapheme_boundary(&self, offset: CharOffset) -> CharOffset {
        let length = self.len_chars();
        let offset = offset.min(CharOffset(length));
        if offset.0 == length {
            return offset;
        }
        let line = self.line_of(offset);
        let line_start = self.line_start(line);
        let content_end = self.line_content_end(line);
        if offset >= content_end {
            return CharOffset((offset.0 + 1).min(length));
        }
        let text = self.rope.slice(line_start.0..content_end.0).to_string();
        let mut char_index = 0;
        for grapheme in text.graphemes(true) {
            let end = CharOffset(line_start.0 + char_index + grapheme.chars().count());
            if end > offset {
                return end;
            }
            char_index += grapheme.chars().count();
        }
        content_end
    }

    /// Line/UTF-8-byte-column point for Tree-sitter.
    #[must_use]
    pub fn text_point(&self, offset: CharOffset) -> TextPoint {
        let offset = offset.min(CharOffset(self.len_chars()));
        let line = self.line_of(offset);
        let line_start = self.line_start(line);
        TextPoint {
            row: line.0,
            column_bytes: self.rope.char_to_byte(offset.0) - self.rope.char_to_byte(line_start.0),
        }
    }

    /// Apply one validated transaction. Text, EOL metadata and history change
    /// together; a rejected transaction leaves the document untouched.
    pub fn apply(&mut self, transaction: EditTransaction) -> Result<EditResult, EditError> {
        let length = self.len_chars();
        let EditTransaction {
            mut edits,
            before_selection,
            after_selection,
            cause,
        } = transaction;
        edits.retain(|edit| !(edit.range.is_empty() && edit.insert.is_empty()));
        if edits.is_empty() {
            return Err(EditError::EmptyTransaction);
        }
        edits.sort_by_key(|edit| edit.range.start);
        for window in edits.windows(2) {
            if window[0].range.end > window[1].range.start {
                return Err(EditError::OverlappingEdits {
                    first: window[0].range,
                    second: window[1].range,
                });
            }
        }
        for edit in &edits {
            if edit.range.end.0 > length {
                return Err(EditError::OutOfBounds {
                    offset: edit.range.end,
                    length,
                });
            }
        }

        let was_contiguous_typing = self.can_coalesce(cause, &edits, before_selection);
        let grouped = self.group_depth > 0 && self.undo_stack.len() > self.group_start_depth;
        let mut steps = Vec::with_capacity(edits.len());
        let mut applied = Vec::with_capacity(edits.len());
        // Descending application keeps all lower original offsets valid.
        for edit in edits.iter().rev() {
            let old_start_line = self.line_of(edit.range.start);
            let old_end_line = self.line_of(edit.range.end);
            let old_start_point = self.text_point(edit.range.start);
            let old_end_point = self.text_point(edit.range.end);
            let old_byte_start = self.rope.char_to_byte(edit.range.start.0);
            let old_byte_end = self.rope.char_to_byte(edit.range.end.0);
            let removed = self.slice(edit.range);
            let break_index = self.rope.char_to_line(edit.range.start.0);
            let removed_breaks: Vec<Eol> = {
                let count = removed.chars().filter(|c| *c == '\n').count();
                self.eol
                    .kinds()
                    .iter()
                    .skip(break_index)
                    .take(count)
                    .copied()
                    .collect()
            };
            let inserted_breaks: Vec<Eol> = edit
                .insert
                .chars()
                .filter(|c| *c == '\n')
                .map(|_| self.preferred_eol)
                .collect();

            self.rope.remove(edit.range.start.0..edit.range.end.0);
            if !edit.insert.is_empty() {
                self.rope.insert(edit.range.start.0, &edit.insert);
            }
            self.eol
                .splice(break_index, removed_breaks.len(), inserted_breaks.clone());

            let inserted_chars = edit.inserted_chars();
            let new_start_line = self.line_of(edit.range.start);
            let new_end_offset =
                CharOffset((edit.range.start.0 + inserted_chars).min(self.len_chars()));
            let new_end_line = self.line_of(new_end_offset);
            let new_end_point = self.text_point(new_end_offset);
            self.patch_utf16_lines(old_start_line, old_end_line, new_start_line, new_end_line);

            steps.push(UndoStep {
                edit: edit.clone(),
                removed: removed.clone(),
                eol: EolOp {
                    break_index,
                    removed: removed_breaks,
                    inserted: inserted_breaks,
                },
            });
            applied.push(AppliedEdit {
                old_range: edit.range,
                new_range: CharRange::new(edit.range.start, new_end_offset),
                old_byte_range: old_byte_start..old_byte_end,
                new_byte_range: old_byte_start..old_byte_start + edit.insert.len(),
                old_start_point,
                old_end_point,
                new_end_point,
                removed_text: removed,
                inserted_text: edit.insert.clone(),
            });
        }

        // `steps` stay in application order (descending offsets); undo walks
        // them backwards.
        applied.reverse();
        // Ranges recorded while applying refer to intermediate documents when
        // a transaction has several edits. Rebase them onto the final text in
        // ascending offset order, accumulating each edit's length delta.
        let mut shift: isize = 0;
        let mut affected_start = LineIndex(usize::MAX);
        let mut affected_end = LineIndex::ZERO;
        for applied_edit in &mut applied {
            let inserted = applied_edit.inserted_text.chars().count();
            let removed = applied_edit.old_range.end.0 - applied_edit.old_range.start.0;
            let start = CharOffset((applied_edit.old_range.start.0 as isize + shift) as usize);
            let end = CharOffset(start.0 + inserted);
            applied_edit.new_range = CharRange::new(start, end);
            applied_edit.new_byte_range =
                self.rope.char_to_byte(start.0)..self.rope.char_to_byte(end.0);
            applied_edit.new_end_point = self.text_point(end);
            affected_start = affected_start.min(self.line_of(start));
            affected_end = affected_end.max(LineIndex(self.line_of(end).0 + 1));
            shift += inserted as isize - removed as isize;
        }
        self.revision = self.revision.next();

        let after_state = self.fresh_state();
        let entry = HistoryEntry {
            steps,
            before_selection,
            after_selection,
            cause,
            time: Instant::now(),
            before_state: self.current_state,
            after_state,
        };
        self.redo_stack.clear();
        let append_to_last = (grouped || was_contiguous_typing)
            && self
                .undo_stack
                .last()
                .is_some_and(|last| last.after_state != self.saved_state);
        if append_to_last {
            if let Some(last) = self.undo_stack.last_mut() {
                last.steps.extend(entry.steps);
                last.after_selection = entry.after_selection;
                last.time = entry.time;
                last.after_state = after_state;
            }
        } else {
            self.undo_stack.push(entry);
        }
        self.current_state = after_state;

        let affected_lines = LineRange::new(
            affected_start,
            LineIndex(affected_end.0.max(affected_start.0 + 1)),
        );
        Ok(EditResult {
            revision: self.revision,
            edits: applied,
            affected_lines,
        })
    }

    fn can_coalesce(&self, cause: EditCause, edits: &[TextEdit], before: Selection) -> bool {
        if cause != EditCause::Typing || edits.len() != 1 {
            return false;
        }
        let edit = &edits[0];
        if edit.insert.contains('\n') || !edit.range.is_empty() {
            return false;
        }
        let Some(last) = self.undo_stack.last() else {
            return false;
        };
        last.cause == EditCause::Typing
            && last.after_state != self.saved_state
            && last.after_selection.is_empty()
            && last.after_selection.head == edit.range.start
            && last.after_selection == before
            && last.time.elapsed() < TYPING_COALESCE_WINDOW
    }

    fn patch_utf16_lines(
        &mut self,
        old_start_line: LineIndex,
        old_end_line: LineIndex,
        new_start_line: LineIndex,
        new_end_line: LineIndex,
    ) {
        let mut utf16 = self.utf16.borrow_mut();
        let new_line_count = self.rope.len_lines();
        let last_new = new_end_line.0.min(new_line_count.saturating_sub(1));
        let new_lengths: Vec<usize> = (new_start_line.0..=last_new)
            .map(|line| utf16_len(self.rope.line(line).chars()))
            .collect();
        let remove_end = (old_end_line.0 + 1).min(utf16.line_lengths.len());
        let remove_start = old_start_line.0.min(remove_end);
        utf16
            .line_lengths
            .splice(remove_start..remove_end, new_lengths);
        utf16.prefix_valid = false;
    }

    fn apply_inverse(&mut self, step: &UndoStep) {
        let start = step.edit.range.start;
        let old_start_line = self.line_of(start);
        let inserted_end = CharOffset((start.0 + step.edit.inserted_chars()).min(self.len_chars()));
        let old_end_line = self.line_of(inserted_end);
        self.rope.remove(start.0..inserted_end.0);
        if !step.removed.is_empty() {
            self.rope.insert(start.0, &step.removed);
        }
        self.eol.splice(
            step.eol.break_index,
            step.eol.inserted.len(),
            step.eol.removed.iter().copied(),
        );
        let new_end = CharOffset((start.0 + step.removed.chars().count()).min(self.len_chars()));
        self.patch_utf16_lines(
            old_start_line,
            old_end_line,
            self.line_of(start),
            self.line_of(new_end),
        );
    }

    fn apply_forward(&mut self, step: &UndoStep) {
        let start = step.edit.range.start;
        let old_start_line = self.line_of(start);
        let removed_end =
            CharOffset((start.0 + step.removed.chars().count()).min(self.len_chars()));
        let old_end_line = self.line_of(removed_end);
        self.rope.remove(start.0..step.edit.range.end.0);
        if !step.edit.insert.is_empty() {
            self.rope.insert(start.0, &step.edit.insert);
        }
        self.eol.splice(
            step.eol.break_index,
            step.eol.removed.len(),
            step.eol.inserted.iter().copied(),
        );
        let new_end = CharOffset((start.0 + step.edit.inserted_chars()).min(self.len_chars()));
        self.patch_utf16_lines(
            old_start_line,
            old_end_line,
            self.line_of(start),
            self.line_of(new_end),
        );
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Option<Selection> {
        let entry = self.undo_stack.pop()?;
        entry.undo(self);
        self.revision = self.revision.next();
        let selection = entry.before_selection.clamp(self.len_chars());
        self.current_state = entry.before_state;
        self.redo_stack.push(entry);
        Some(selection)
    }

    pub fn redo(&mut self) -> Option<Selection> {
        let entry = self.redo_stack.pop()?;
        entry.redo(self);
        self.revision = self.revision.next();
        let selection = entry.after_selection.clamp(self.len_chars());
        self.current_state = entry.after_state;
        self.undo_stack.push(entry);
        Some(selection)
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.current_state != self.saved_state
    }

    /// Mark the current history position as the on-disk baseline.
    pub fn mark_saved(&mut self) {
        self.saved_state = self.current_state;
    }

    /// Mark a previously captured history state as the on-disk baseline. A
    /// background save uses this so edits made while it was in flight stay
    /// dirty.
    pub fn mark_saved_at(&mut self, state: u64) {
        self.saved_state = state;
    }

    /// Mark the current content as unsaved. Recovery uses this after restoring
    /// a snapshot so closing the window still prompts to save.
    pub fn mark_unsaved(&mut self) {
        self.saved_state = self.fresh_state();
    }

    /// Opaque identity of the current history state, for a late save result.
    #[must_use]
    pub fn state_token(&self) -> u64 {
        self.current_state
    }

    /// Capture text and newline metadata for a background save. Cloning the
    /// Rope is cheap and does not copy its text.
    #[must_use]
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            rope: self.rope.clone(),
            eol: self.eol.clone(),
            preferred_eol: self.preferred_eol,
        }
    }

    fn fresh_state(&mut self) -> u64 {
        let state = self.next_state;
        self.next_state = self.next_state.wrapping_add(1);
        state
    }

    /// Begin an explicit user operation. Transactions applied while a group is
    /// open share one undo entry, so a Vim change and the text typed into the
    /// resulting Insert session undo together. Groups nest.
    pub fn begin_user_operation(&mut self) {
        if self.group_depth == 0 {
            self.group_start_depth = self.undo_stack.len();
        }
        self.group_depth += 1;
    }

    /// Close the innermost user operation. The outermost close returns history
    /// to normal per-transaction grouping.
    pub fn end_user_operation(&mut self) {
        self.group_depth = self.group_depth.saturating_sub(1);
        if self.group_depth == 0 {
            self.group_start_depth = 0;
        }
    }

    /// Abandon any open user operation without merging further transactions.
    /// Used when Vim mode is disabled or the document is replaced.
    pub fn cancel_user_operation(&mut self) {
        self.group_depth = 0;
        self.group_start_depth = 0;
    }

    /// Replace the whole document (file reload). Clears history and marks the
    /// new content clean.
    pub fn reset(&mut self, text: String, eol: EolMap, preferred_eol: Eol) {
        let replacement = Self::from_normalized_text(text, eol, preferred_eol);
        self.rope = replacement.rope;
        self.eol = replacement.eol;
        self.preferred_eol = replacement.preferred_eol;
        self.utf16 = replacement.utf16;
        self.undo_stack.clear();
        self.redo_stack.clear();
        let state = self.fresh_state();
        self.current_state = state;
        self.saved_state = state;
        self.group_depth = 0;
        self.group_start_depth = 0;
        self.revision = self.revision.next();
    }

    /// Count of undo entries (for diagnostics/tests).
    #[must_use]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
