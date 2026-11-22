# Ropey document store and position correctness

## Selection

Use [`ropey::Rope`](https://docs.rs/ropey/latest/ropey/struct.Rope.html) for editable UTF-8 text. It stores text as a balanced rope, with practical chunk/line/character APIs. The **unit of Ropey `insert/remove/slice` positions is a Unicode scalar (`char`) index**, not a UTF-8 byte index, visual column, grapheme or UTF-16 code unit. Treat that distinction as a type-system constraint.

Ropey is a **storage engine**, not an undo manager, display renderer, selection engine or encoding detector. Build those layers explicitly; do not expect Ropey to solve them.

## Explicit types

Create positional newtypes to prevent accidental cross-unit arithmetic:

```rust
struct CharOffset(usize);       // Ropey character index, canonical edits
struct ByteOffset(usize);       // UTF-8, Tree-sitter offset
struct Utf16Offset(usize);      // GPUI/native text input index
struct LineIndex(usize);        // hard line index (0-based)
struct GraphemeColumn(usize);   // caret boundary in logical line
struct VisualRow(usize);        // screen row after soft wrap
struct VisualX(f32);           // shaped text X, logical pixels
struct DocumentRevision(u64);
```

Use `CharRange`/`ByteRange` equivalents rather than untyped `Range<usize>` in domain-facing public APIs. `Rope::char_to_byte`, `byte_to_char`, `char_to_line`, `line_to_char` bridge the standard units. Only construct byte offsets at verified UTF-8 boundaries. For line/column, account for both hard line separators and trailing newline semantics.

## Cursor, graphemes, and UTF-16

Canonical document edits use scalar indices. **Visible caret stops use grapheme cluster boundaries**, because `e + ◌́` and emoji joined with ZWJ should be deleted/navigated as user-perceived characters. Use `unicode-segmentation` on a bounded span around the cursor; line breaks and long clusters require care. Keep selections/edits normalized to legal stops unless a native IME explicitly requests an intermediary composition range.

Platform IME/accessibility APIs often use UTF-16 offsets. Define:

```text
rope_char_to_utf16(doc, char_offset) -> Utf16Offset
utf16_to_rope_char(doc, utf16_offset, affinity) -> CharOffset
```

Naively walking from offset 0 on every caret movement is O(document size). Store a lazily updated per-line prefix of UTF-16 lengths, then scan relevant Rope chunks/line slices. Track `affinity` for an index in the middle of a surrogate pair; never split a scalar. Invalidate prefix cache after insert/delete on affected and subsequent lines; use cumulative indexed metadata or incremental updates for scalable lookup.

**Line endings:** Ropey has line APIs and recognizes common newline sequences. The file layer must independently preserve original on-disk EOL metadata; the text buffer should define whether it normalizes CRLF on import. Recommended: canonical in-memory `\n` plus file-layer EOL reconstruction on save **only when per-line mixed endings can be preserved**; if mixed line endings are not preserved, don't normalize them silently—retain the original CRLF/CR text and teach navigation/layout to recognize it. See [file-io.md](file-io.md). Choose and test one policy before M2.

## Edit/change representation

Use a transaction API such as:

```rust
struct TextEdit { delete: CharRange, insert: String }
struct EditTransaction {
    edits: Vec<TextEdit>, // no overlap; validated in original coordinates
    before_selection: Selection,
    after_selection: Selection,
    cause: EditCause,
}
struct EditResult {
    revision: DocumentRevision,
    old_byte_range: ByteRange,
    new_byte_range: ByteRange,
    old_start_point: TsPoint,
    old_end_point: TsPoint,
    new_end_point: TsPoint,
    affected_lines: LineRange,
}
```

Before mutation, calculate affected byte indices and Tree-sitter points; replacements may contain multiple lines and multi-byte Unicode. Apply disjoint edits in descending offset order so earlier offsets remain valid. After mutation, compute new caret/selection and edited line span; derive Tree-sitter `InputEdit` data for each change **in the same order it is actually applied**. Treat formatter output and replace-all as a single undo transaction even if it contains multiple edits.

Prefer a bounded history of inverse edit deltas over snapshots of the whole Rope. Coalesce nearby user typing using time + cursor continuity + cause, without merging edits across a mode switch, cursor jump, paste, file reload or explicit undo boundary. Redo cleared when a new edit branches from the undone state. Track save checkpoint by transaction identity/revision, not merely a boolean toggled on the first keystroke; undo to saved content should clear the dirty indicator where feasible.

## Rope access patterns

- Use `rope.line(line_idx)`, `.lines()`, `.chunks()`, `.slice(char_range)` to consume only relevant slices. Convert a visible line to `String` only for its local shaping/query consumption, not the entire document.
- `rope.len_chars()` and `rope.len_lines()` are useful for bounds and viewport indexes. Do **not** assume trailing newline produces no final empty line.
- For rendering, maintain an indexed mapping from hard lines to wrapped visual-row counts; keep lazy estimates outside the viewport. A viewport jump to line 100,000 must not sequentially shape 100,000 lines.
- Save via `Rope::write_to(...)` or iterate chunks into a transcoding/EOL writer rather than allocating one giant string. A separate original-byte representation may be required for unsupported/invalid encodings; never silently insert replacement characters.
- Background highlighting receives immutable snapshots (`Rope::clone()` is cheap copy-on-write, but measure memory pressure) and a revision; results apply only to matching revision.

## Essential property/fuzz tests

1. Apply random Unicode inserts/deletes (including CRLF and grapheme joins) to Rope and a small reference `String`; compare resulting text after every operation.
2. For random legal char indices, check `byte_to_char(char_to_byte(i)) == i`; for UTF-16 positions check lossless round trips at scalar boundaries, and defined clamping inside surrogate pairs.
3. Verify soft-wrapped visual positions map to exact logical char positions and back for tabs, ligatures, combining marks and varying viewport sizes.
4. Insert/delete around EOF, empty rope, newline boundaries, long lines and Unicode clusters; no panic or invalid range.
5. Undo all random transactions and redo all; resulting text and selected ranges must equal captured checkpoints. Include edits that change line count and nested commands.
6. Profile random edits near start of a 100k-line document; no full-buffer scans as a hidden side effect.

## Do not

- Use byte offsets to call Ropey's char-indexed editing APIs.
- Use Rust `.chars().count()` on the complete document to compute a cursor position.
- Infer a user's cursor movement boundaries from scalar values without grapheme segmentation.
- Normalize every input document's EOL/BOM silently; treat load/save policy as a separate testable layer.
- Store text twice indefinitely as `Rope` and a synchronized giant `String` for rendering.

## References

[Rope API](https://docs.rs/ropey/latest/ropey/struct.Rope.html), [RopeSlice API](https://docs.rs/ropey/latest/ropey/struct.RopeSlice.html), [Ropey design/units](https://docs.rs/ropey/latest/ropey/), [Unicode Segmentation crate](https://docs.rs/unicode-segmentation/latest/unicode_segmentation/).
