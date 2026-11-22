//! Cumulative visual-row counts per hard line.
//!
//! A Fenwick tree over the per-line row counts answers "how many rows precede
//! this line" and "which line owns this visual row" in `O(log lines)`, and a
//! point update touches only that line's count. Inserting or deleting hard
//! lines splices the count vector and rebuilds the tree, which is `O(lines)`
//! integer work and never re-measures Unicode wrapping for untouched lines.

use chaus_core::position::LineIndex;

#[derive(Debug, Default)]
pub(crate) struct RowIndex {
    values: Vec<usize>,
    /// One-based Fenwick tree; `tree[index]` sums a power-of-two range.
    tree: Vec<usize>,
}

impl RowIndex {
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    /// Visual rows before `line`.
    pub(crate) fn prefix(&self, line: usize) -> usize {
        let mut index = line.min(self.values.len());
        let mut total = 0usize;
        while index > 0 {
            total += self.tree[index];
            index &= index - 1;
        }
        total
    }

    pub(crate) fn total(&self) -> usize {
        self.prefix(self.values.len())
    }

    /// Replace every count. Used by a full rebuild and by a splice.
    pub(crate) fn rebuild(&mut self, values: Vec<usize>) {
        self.values = values;
        let count = self.values.len();
        self.tree.clear();
        self.tree.resize(count + 1, 0);
        for (line, rows) in self.values.iter().copied().enumerate() {
            let mut index = line + 1;
            while index <= count {
                self.tree[index] += rows;
                index += index.isolate_lowest_one();
            }
        }
    }

    /// Change one line's row count.
    pub(crate) fn set(&mut self, line: LineIndex, rows: usize) {
        let Some(old) = self.values.get_mut(line.0) else {
            return;
        };
        if *old == rows {
            return;
        }
        let delta = rows as i64 - *old as i64;
        *old = rows;
        let count = self.values.len();
        let mut index = line.0 + 1;
        while index <= count {
            self.tree[index] = (self.tree[index] as i64 + delta).max(0) as usize;
            index += index.isolate_lowest_one();
        }
    }

    /// Replace the counts for the lines an edit inserted or removed.
    ///
    /// `start` and `old_end` are in the old document; `new_rows` supplies the
    /// counts for `start..` in the new document. Untouched lines keep their
    /// measured counts, so only the edited range is re-wrapped.
    pub(crate) fn splice(
        &mut self,
        start: usize,
        old_end: usize,
        new_rows: impl IntoIterator<Item = usize>,
    ) {
        let start = start.min(self.values.len());
        let old_end = old_end.clamp(start, self.values.len());
        self.values.splice(start..old_end, new_rows);
        let values = std::mem::take(&mut self.values);
        self.rebuild(values);
    }

    /// The hard line that owns visual row `row`, clamped to the document.
    pub(crate) fn line_at_row(&self, row: usize) -> LineIndex {
        if self.values.is_empty() {
            return LineIndex(0);
        }
        let mut index = 0usize;
        let mut sum = 0usize;
        // Largest power of two not above the line count.
        let mut step = 1usize << (usize::BITS - 1 - self.values.len().leading_zeros());
        while step > 0 {
            let next = index + step;
            if next <= self.values.len() && sum + self.tree[next] <= row {
                index = next;
                sum += self.tree[next];
            }
            step >>= 1;
        }
        LineIndex(index.min(self.values.len() - 1))
    }
}
