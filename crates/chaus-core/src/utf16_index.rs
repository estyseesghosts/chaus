//! Per-line UTF-16 lengths with a Fenwick tree for prefix queries.
//!
//! Point updates from ordinary typing are `O(log lines)` and prefix queries are
//! `O(log lines)`, so a conversion after an edit near the start of a 100k-line
//! document no longer walks every preceding line. A splice that changes the
//! line count rebuilds the tree from the already-patched length vector; the
//! text itself is never rescanned by that rebuild.

use ropey::Rope;

use crate::position::{CharOffset, Utf16Offset};

pub(crate) fn utf16_len(characters: impl Iterator<Item = char>) -> usize {
    characters.map(char::len_utf16).sum()
}

#[derive(Debug, Default)]
pub(crate) struct Utf16Index {
    pub(crate) line_lengths: Vec<usize>,
    tree: Vec<usize>,
}

impl Utf16Index {
    pub(crate) fn rebuild(&mut self, rope: &Rope) {
        self.line_lengths.clear();
        for line in rope.lines() {
            self.line_lengths.push(utf16_len(line.chars()));
        }
        self.rebuild_tree();
    }

    pub(crate) fn rebuild_tree(&mut self) {
        let count = self.line_lengths.len();
        self.tree.clear();
        self.tree.resize(count + 1, 0);
        for (line, length) in self.line_lengths.iter().copied().enumerate() {
            let mut index = line + 1;
            while index <= count {
                self.tree[index] += length;
                index += index.isolate_lowest_one();
            }
        }
    }

    pub(crate) fn set_length(&mut self, line: usize, length: usize) {
        let Some(old) = self.line_lengths.get(line).copied() else {
            return;
        };
        if old == length {
            return;
        }
        self.line_lengths[line] = length;
        let count = self.line_lengths.len();
        let mut index = line + 1;
        while index <= count {
            if length >= old {
                self.tree[index] += length - old;
            } else {
                self.tree[index] -= old - length;
            }
            index += index.isolate_lowest_one();
        }
    }

    /// Sum of UTF-16 lengths for lines `0..line`.
    fn prefix(&self, line: usize) -> usize {
        let mut index = line.min(self.line_lengths.len());
        let mut total = 0usize;
        while index > 0 {
            total += self.tree[index];
            index &= index - 1;
        }
        total
    }

    fn total(&self) -> usize {
        self.prefix(self.line_lengths.len())
    }

    /// First line index whose prefix is at least `target`.
    ///
    /// For `target == 0` this is line zero; otherwise the result is the
    /// smallest count whose cumulative UTF-16 length reaches the target.
    /// `target` must not exceed [`Self::total`].
    fn first_line_at_or_after(&self, target: usize) -> usize {
        if target == 0 {
            return 0;
        }
        let count = self.line_lengths.len();
        let mut index = 0usize;
        let mut remaining = target;
        let mut step = 1usize;
        while step <= count {
            step = step.saturating_mul(2);
        }
        step >>= 1;
        while step > 0 {
            let next = index + step;
            if next <= count && self.tree[next] < remaining {
                index = next;
                remaining -= self.tree[next];
            }
            step >>= 1;
        }
        (index + 1).min(count)
    }

    pub(crate) fn char_to_utf16(&mut self, rope: &Rope, offset: CharOffset) -> Utf16Offset {
        let offset = offset.min(CharOffset(rope.len_chars()));
        let line = rope.char_to_line(offset.0);
        let line_start = rope.line_to_char(line);
        let mut units = self.prefix(line);
        for ch in rope.line(line).chars().take(offset.0 - line_start) {
            units += ch.len_utf16();
        }
        Utf16Offset(units)
    }

    pub(crate) fn utf16_to_char(&mut self, rope: &Rope, offset: Utf16Offset) -> CharOffset {
        let length = rope.len_lines();
        if length == 0 {
            return CharOffset::ZERO;
        }
        let target = offset.0.min(self.total());
        // Several lines can start at the same UTF-16 offset when blank lines
        // precede the target. Pick the *first* matching line; otherwise a
        // lookup at offset zero would land after the leading newline.
        let first_at_or_after = self.first_line_at_or_after(target);
        let line = match self.prefix(first_at_or_after) {
            start if start == target => first_at_or_after,
            _ => first_at_or_after.saturating_sub(1),
        }
        .min(length - 1);
        let line_start = rope.line_to_char(line);
        let mut units = self.prefix(line);
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
