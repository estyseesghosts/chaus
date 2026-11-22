//! Minimal change spans between two texts.
//!
//! Whole-document rewrites (formatters, external tools) usually change a few
//! lines. Comparing from both ends turns that into one small replacement
//! instead of replacing every character: undo keeps only the changed text and
//! the view invalidates only the rows that moved.
//!
//! The scan is linear in the texts and counts characters, so it never splits a
//! UTF-8 scalar. It is deliberately not a general diff: only the common prefix
//! and suffix are removed, which is reliable, bounded and enough for the
//! formatter path.

use crate::position::{CharOffset, CharRange};

/// One replacement that turns the old text into the new text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    /// Characters of the old text that the replacement removes.
    pub old: CharRange,
    /// Text that takes the place of `old`. Empty for a pure deletion.
    pub insert: String,
}

impl TextChange {
    /// Length, in characters, of a document of `old_len` characters after the
    /// change is applied.
    #[must_use]
    pub fn new_len(&self, old_len: usize) -> usize {
        old_len - self.old.len() + self.insert.chars().count()
    }

    /// Characters the replacement inserts.
    #[must_use]
    pub fn inserted_len(&self) -> usize {
        self.insert.chars().count()
    }
}

/// The smallest prefix/suffix change that turns `old` into `new`.
///
/// Returns `None` when the texts are already equal. Adjacent equal text is
/// never part of `old` or `insert`, so applying the change can only touch the
/// characters that actually differ.
#[must_use]
pub fn common_change(old: &str, new: &str) -> Option<TextChange> {
    if old == new {
        return None;
    }

    let old_len = old.chars().count();
    let new_len = new.chars().count();
    let limit = old_len.min(new_len);

    let mut prefix = 0;
    let mut new_prefix_end = 0;
    for ((_, a), (new_byte, b)) in old.char_indices().zip(new.char_indices()) {
        if a != b {
            break;
        }
        prefix += 1;
        new_prefix_end = new_byte + b.len_utf8();
    }

    // The suffix may not reach back into the prefix; otherwise a text that is
    // one repeated character (`a` -> `aa`) would report a zero-width change.
    let mut suffix = 0;
    let mut new_suffix_start = new.len();
    if prefix < limit {
        for ((_, a), (new_byte, b)) in old.char_indices().rev().zip(new.char_indices().rev()) {
            if suffix >= limit - prefix || a != b {
                break;
            }
            suffix += 1;
            new_suffix_start = new_byte;
        }
    }

    Some(TextChange {
        old: CharRange::new(CharOffset(prefix), CharOffset(old_len - suffix)),
        insert: new[new_prefix_end..new_suffix_start].to_owned(),
    })
}
