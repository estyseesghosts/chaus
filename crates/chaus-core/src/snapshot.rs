//! Immutable text and newline data captured for background work.
//!
//! A snapshot is deliberately outside the document's mutation and history
//! paths: it shares Rope chunks and EOL map chunks, so a save or reflow job can
//! read a stable view while editing continues.

use ropey::Rope;

use crate::eol::{Eol, EolMap};

/// A cheap, immutable snapshot of document text and newline metadata for a
/// background save. History and selection are deliberately excluded.
#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    rope: Rope,
    eol: EolMap,
    preferred_eol: Eol,
}

impl DocumentSnapshot {
    pub(crate) fn new(rope: Rope, eol: EolMap, preferred_eol: Eol) -> Self {
        Self {
            rope,
            eol,
            preferred_eol,
        }
    }

    /// Reconstruct the original newline kinds without touching the live
    /// document.
    #[must_use]
    pub fn text_with_eol(&self) -> String {
        self.eol.expand(self.rope.chars(), self.preferred_eol)
    }

    /// Number of hard lines in the captured text.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Borrow one hard line, including its trailing `\n` when present.
    /// Background layout walks these slices through `chaus_layout`'s bounded
    /// source path instead of materializing every line up front.
    #[must_use]
    pub fn line_slice(&self, line: usize) -> ropey::RopeSlice<'_> {
        self.rope.line(line)
    }

    /// Number of shared newline chunks captured by this snapshot.
    #[must_use]
    pub fn eol_chunk_count(&self) -> usize {
        self.eol.chunk_count()
    }

    /// Upper bound on newline heap bytes captured by this snapshot.
    #[must_use]
    pub fn eol_heap_bytes(&self) -> usize {
        self.eol.estimated_heap_bytes()
    }
}
