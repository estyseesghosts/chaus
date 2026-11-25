//! Newline metadata.
//!
//! The Rope stores canonical `\n` hard breaks. The original on-disk newline
//! kind of every break is kept in a parallel map so mixed-EOL files round-trip
//! without a second text buffer. Inserted breaks adopt the document's
//! preferred kind; untouched breaks keep theirs. The map is updated in the
//! same transaction as the Rope (and unwound by undo/redo).
//!
//! Entries live in fixed-capacity chunks behind `Arc`. A snapshot clones only
//! the chunk handles, and an edit uses a copy-on-write rebuild of the chunks it
//! touches, so neither operation scales with the total number of line breaks.

use std::sync::Arc;

/// The newline sequence of one hard break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Eol {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    /// Detect the dominant newline kind in raw file text.
    #[must_use]
    pub fn detect(text: &str) -> Eol {
        let mut crlf = 0usize;
        let mut lf = 0usize;
        let mut cr = 0usize;
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        crlf += 1;
                        i += 1;
                    } else {
                        cr += 1;
                    }
                }
                b'\n' => lf += 1,
                _ => {}
            }
            i += 1;
        }
        if crlf >= lf && crlf >= cr && crlf > 0 {
            Eol::CrLf
        } else if cr > lf && cr > 0 {
            Eol::Cr
        } else {
            Eol::Lf
        }
    }
}

/// Entries per chunk. Chunk handles are shared by snapshots, and edits rebuild
/// only the chunks whose logical range they touch.
const CHUNK_SIZE: usize = 512;

/// One entry per hard break in the document, in document order.
///
/// Cloning is `O(chunks)` (just the handles), and a splice rebuilds only the
/// affected chunks, so a live background snapshot is never invalidated by an
/// edit to the document it came from.
#[derive(Clone, Debug)]
pub struct EolMap {
    chunks: Vec<Arc<Vec<Eol>>>,
    starts: Vec<usize>,
    len: usize,
}

impl Default for EolMap {
    fn default() -> Self {
        Self {
            chunks: vec![Arc::new(Vec::new())],
            starts: vec![0],
            len: 0,
        }
    }
}

impl PartialEq for EolMap {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && (0..self.len).all(|index| self.eol_at(index) == other.eol_at(index))
    }
}

impl Eq for EolMap {}

impl EolMap {
    #[must_use]
    pub fn new(kinds: Vec<Eol>) -> Self {
        let chunks = if kinds.is_empty() {
            vec![Arc::new(Vec::new())]
        } else {
            kinds
                .chunks(CHUNK_SIZE)
                .map(|part| Arc::new(part.to_vec()))
                .collect()
        };
        let mut map = Self {
            chunks,
            starts: Vec::new(),
            len: 0,
        };
        map.reindex();
        map
    }

    /// Rebuild the logical index after a structural change to `chunks`.
    fn reindex(&mut self) {
        self.starts.clear();
        self.starts.reserve(self.chunks.len());
        let mut total = 0usize;
        for chunk in &self.chunks {
            self.starts.push(total);
            total += chunk.len();
        }
        self.len = total;
    }

    /// Map a logical break index to `(chunk, offset)` in `O(log chunks)`.
    fn locate(&self, index: usize) -> (usize, usize) {
        let chunk = self
            .starts
            .partition_point(|&start| start <= index)
            .saturating_sub(1);
        (chunk, index - self.starts[chunk])
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn eol_at(&self, break_index: usize) -> Option<Eol> {
        if break_index >= self.len {
            return None;
        }
        let (chunk, offset) = self.locate(break_index);
        self.chunks[chunk].get(offset).copied()
    }

    /// Number of shared chunks. Snapshot cost is proportional to this value,
    /// not to [`Self::len`].
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Upper bound on heap bytes held by shared chunks and their headers.
    /// Snapshot-sensitive callers can use this to observe that cloning stays
    /// proportional to the chunk count.
    #[must_use]
    pub fn estimated_heap_bytes(&self) -> usize {
        let payload = self.len * std::mem::size_of::<Eol>();
        let handle = std::mem::size_of::<Vec<Eol>>() + 2 * std::mem::size_of::<usize>();
        payload + self.chunks.len() * handle
    }

    /// Copy `count` entries starting at `start` into a contiguous vector.
    #[must_use]
    pub fn kinds_range(&self, start: usize, count: usize) -> Vec<Eol> {
        let end = start.saturating_add(count).min(self.len);
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        for index in start..end {
            if let Some(eol) = self.eol_at(index) {
                out.push(eol);
            }
        }
        out
    }

    /// Materialize every entry as a contiguous vector.
    ///
    /// Snapshot-sensitive paths should prefer [`Self::eol_at`],
    /// [`Self::kinds_range`] or [`Self::expand`] instead.
    #[must_use]
    pub fn kinds(&self) -> Vec<Eol> {
        let mut out = Vec::with_capacity(self.len);
        for chunk in &self.chunks {
            out.extend_from_slice(chunk);
        }
        out
    }

    /// Replace `remove` entries starting at `start` with `insert`.
    ///
    /// Only the chunks overlapping the edited range are rebuilt; the rest
    /// remain shared with any snapshot that captured them earlier.
    pub fn splice(&mut self, start: usize, remove: usize, insert: impl IntoIterator<Item = Eol>) {
        let start = start.min(self.len);
        let end = start.saturating_add(remove).min(self.len);
        let insert: Vec<Eol> = insert.into_iter().collect();
        let insert_len = insert.len();
        if start == end && insert_len == 0 {
            return;
        }

        let (start_chunk, start_offset) = self.locate(start);
        let (end_chunk, end_offset) = self.locate(end);
        let end_chunk_exclusive = if end_offset == 0 {
            end_chunk
        } else {
            end_chunk + 1
        };

        let mut flat = Vec::new();
        for chunk in &self.chunks[start_chunk..end_chunk_exclusive] {
            flat.extend_from_slice(chunk);
        }
        let flat_end = start_offset + (end - start);
        flat.splice(start_offset..flat_end, insert);

        let replacement: Vec<Arc<Vec<Eol>>> = if flat.is_empty() {
            vec![Arc::new(Vec::new())]
        } else {
            flat.chunks(CHUNK_SIZE)
                .map(|part| Arc::new(part.to_vec()))
                .collect()
        };
        self.chunks
            .splice(start_chunk..end_chunk_exclusive, replacement);
        self.reindex();
    }

    pub fn push(&mut self, eol: Eol) {
        let start = self.len;
        self.splice(start, 0, std::iter::once(eol));
    }

    /// Rebuild the map for canonical text that has already been normalized to
    /// `\n`, using `source` offsets (pre-normalization text) to recover each
    /// break's original kind.
    #[must_use]
    pub fn from_source(source: &str) -> (String, EolMap) {
        let mut normalized = String::with_capacity(source.len());
        let mut kinds = Vec::new();
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                        kinds.push(Eol::CrLf);
                    } else {
                        kinds.push(Eol::Cr);
                    }
                    normalized.push('\n');
                }
                '\n' => {
                    kinds.push(Eol::Lf);
                    normalized.push('\n');
                }
                other => normalized.push(other),
            }
        }
        (normalized, EolMap::new(kinds))
    }

    /// Reconstruct raw text for save: `text` is canonical (`\n` breaks).
    /// `break_index` advances per `\n` encountered.
    #[must_use]
    pub fn apply_to(&self, text: &str, fallback: Eol) -> String {
        self.expand(text.chars(), fallback)
    }

    /// Reconstruct raw text from any character source, without first
    /// materializing the canonical text as a single `String`.
    #[must_use]
    pub fn expand(&self, characters: impl Iterator<Item = char>, fallback: Eol) -> String {
        let mut out = String::new();
        let mut break_ix = 0;
        for ch in characters {
            if ch == '\n' {
                out.push_str(self.eol_at(break_ix).unwrap_or(fallback).as_str());
                break_ix += 1;
            } else {
                out.push(ch);
            }
        }
        out
    }
}
