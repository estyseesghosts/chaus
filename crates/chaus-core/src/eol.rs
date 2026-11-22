//! Newline metadata.
//!
//! The Rope stores canonical `\n` hard breaks. The original on-disk newline
//! kind of every break is kept in a parallel map so mixed-EOL files round-trip
//! without a second text buffer. Inserted breaks adopt the document's
//! preferred kind; untouched breaks keep theirs. The map is updated in the
//! same transaction as the Rope (and unwound by undo/redo).

/// The newline sequence of one hard break.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Eol {
    #[default]
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    /// Detect the dominant newline kind in raw file text.
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

/// One entry per hard break in the document, in document order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EolMap {
    kinds: Vec<Eol>,
}

impl EolMap {
    pub fn new(kinds: Vec<Eol>) -> Self {
        Self { kinds }
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn eol_at(&self, break_index: usize) -> Option<Eol> {
        self.kinds.get(break_index).copied()
    }

    pub fn kinds(&self) -> &[Eol] {
        &self.kinds
    }

    /// Replace `remove` entries starting at `start` with `insert`.
    pub fn splice(&mut self, start: usize, remove: usize, insert: impl IntoIterator<Item = Eol>) {
        let start = start.min(self.kinds.len());
        let end = (start + remove).min(self.kinds.len());
        self.kinds.splice(start..end, insert);
    }

    pub fn push(&mut self, eol: Eol) {
        self.kinds.push(eol);
    }

    /// Rebuild the map for canonical text that has already been normalized to
    /// `\n`, using `source` offsets (pre-normalization text) to recover each
    /// break's original kind.
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
    pub fn apply_to(&self, text: &str, fallback: Eol) -> String {
        let mut out = String::with_capacity(text.len() + self.len());
        let mut break_ix = 0;
        for ch in text.chars() {
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
