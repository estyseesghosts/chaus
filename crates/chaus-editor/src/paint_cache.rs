//! Bounded cache for shaped visible rows.
//!
//! A repaint that does not change text (caret blink, selection, a transient
//! status message) reuses the GPUI shaped lines for the rows that are already
//! on screen. Any change to the text revision, highlight generation, wrap
//! geometry, font or theme clears the cache, because every one of those
//! changes glyphs, advances or colours.

use std::collections::{HashMap, VecDeque};

use chaus_core::position::DocumentRevision;
use chaus_layout::WrapPolicy;
use gpui::ShapedLine;
use silvestris::ThemeId;

/// Everything that affects how a row is shaped and coloured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaintKey {
    pub(crate) revision: DocumentRevision,
    pub(crate) highlight: u64,
    pub(crate) columns: Option<usize>,
    pub(crate) policy: WrapPolicy,
    pub(crate) tab_stop: usize,
    pub(crate) font_size_bits: u32,
    pub(crate) font_family: String,
    pub(crate) theme: ThemeId,
    /// Source scalar where a painted window starts. Wrapped rows use their
    /// segment start; unwrapped painting uses the visible horizontal window.
    pub(crate) offset: usize,
}

/// A row is identified by its hard line and its segment within that line.
type RowId = (usize, usize);

#[derive(Default)]
pub(crate) struct ShapedRowCache {
    key: Option<PaintKey>,
    entries: HashMap<RowId, ShapedLine>,
    /// Insertion order, oldest first, so eviction is bounded.
    order: VecDeque<RowId>,
}

impl ShapedRowCache {
    /// Rows kept across repaints. A viewport shows far fewer; the bound also
    /// keeps scrolling from growing the cache without limit.
    const CAPACITY: usize = 256;

    fn prepare(&mut self, key: &PaintKey) {
        if self.key.as_ref() == Some(key) {
            return;
        }
        self.key = Some(key.clone());
        self.entries.clear();
        self.order.clear();
    }

    pub(crate) fn get(
        &mut self,
        key: &PaintKey,
        line: usize,
        segment: usize,
    ) -> Option<ShapedLine> {
        self.prepare(key);
        let id = (line, segment);
        let shaped = self.entries.get(&id).cloned()?;
        // The paint loop walks rows in order, so the front is the least
        // recently drawn; keep recently used rows at the back.
        if let Some(position) = self.order.iter().position(|entry| *entry == id) {
            self.order.remove(position);
            self.order.push_back(id);
        }
        Some(shaped)
    }

    pub(crate) fn insert(
        &mut self,
        key: &PaintKey,
        line: usize,
        segment: usize,
        shaped: ShapedLine,
    ) {
        self.prepare(key);
        let id = (line, segment);
        if self.entries.insert(id, shaped).is_none() {
            self.order.push_back(id);
        }
        while self.entries.len() > Self::CAPACITY {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}
