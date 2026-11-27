# The editor: GPUI surface, input, commands, undo and performance

## Architecture / ownership

Build a **new GPUI multiline editor view**. Do not use GPUI Kit, Zed's editor subsystem, or Silvestris's single-line `TextField` as the multiline model. Silvestris handles frame/menus/prefs; the app owns the editor. GPUI's [pinned `EntityInputHandler` trait](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/src/input.rs), [input example](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/examples/input.rs) and Silvestris's [text field](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/controls/text_field.rs) are primary **reference implementations** for platform text services.

The sketch below is the original design intent. The implemented ownership map
after the cleanup pass is in [ownership-map.md](ownership-map.md); the
`EditorState` name in particular is no longer a type in the tree.

Recommended state:

```text
AppState { settings, open_windows, language_registry }
Document { rope, revision, file_metadata, dirty, history, disk_baseline }
EditorState { document_handle, selection(s), preferred_x, scroll, wrap,
              composition, focus_handle, layout_cache, vim_state, find_state }
EditorView  { GPUI element/render/paint/input adapters }

EditorIntent --> CommandDispatcher --> Document::apply(transaction)
              --> ChangeSet + revision --> caches/parse/paint invalidation
```

`Document` contains text and undo history, but **not pixel positions, GPUI handles or focus**. `EditorState` owns per-window cursor and transient input state. One document per window; keep logical model separate from view so fonts/themes can switch without resetting state.

## One edit transaction pipeline

Define typed operations, e.g. `Replace { char_range, text }`, `MoveCaret`, `SetSelection`, `Undo`, `Redo`. Every actual text edit runs through:

1. Validate all ranges against a single document revision, grapheme-safe cursor boundaries, read-only state, and overlap rules.
2. Apply a transaction to Ropey; record deleted text, inserted text, cursor/selection before and after, edit source (`Typing`, `Paste`, `IMECommit`, `Vim`, `Formatter`, `ReplaceAll`), and affected logical lines.
3. Increment a monotonically increasing document revision; emit a `ChangeSet` including **old/new UTF-8 byte ranges and Tree-sitter points** derived before/after mutation, not guessed after applying it.
4. Invalidate affected visual rows and syntax spans; notify GPUI/native text service of text and selection changes; schedule revision-tagged parser work.
5. Group undo by user intent: contiguous typing may coalesce until cursor movement/pause/focus change; paste/format/replace-all/one Vim operator are each one history entry. Cancelled preedit is not a document edit.

Selection is represented as anchor + head (not only `min..max`); retain direction for Shift+Arrow, dragging, Visual mode and copy. Normalize multi-selection only if implementing multi-cursor later; v0.1 should have one selection to limit complexity.

Transactions must be deterministic and independently unit-testable; GUI handlers should never splice Strings directly.

## GPUI input integration

Implement the pinned `gpui::EntityInputHandler` interface on an editor entity. In the custom editor `Element::paint`, register `Window::handle_input(&focus_handle, ElementInputHandler::new(bounds, editor_entity), cx)` only when the document view is eligible for text input. Check exact pinned API signatures.

The bridge must support:

- `selected_text_range` and `set_selected_text_range`: convert between **UTF-16 code units** and Ropey char indices, preserving selection direction where supported; text-service APIs are not char indices.
- `text_for_range`, `text_length_utf16`, `bounds_for_range`, `selection_bounds_for_range`, `character_index_for_point`: expose correct text and rects across hard/soft lines; clamp to legal boundaries and report adjusted ranges where expected.
- `replace_text_in_range`, `replace_and_mark_text_in_range`, `marked_text_range`, `unmark_text`: IME preedit/commit/cancel with proper composition overlay or documented transaction approach; do not treat interim composition as committed user typing.
- `paste`: preserve multiline plaintext (the Silvestris single-line field intentionally flattens it; **do not copy that behavior**). Normalize only where user settings/file format require.
- `accepts_text_input`: true during standard mode and Vim Insert/Replace/command input, **false for normal-mode letter commands** when not composing. Native IME composition takes priority in editable modes.

Input routing order: active native dialog/menu → focused settings/search/command field → editor IME composition → global non-conflicting command shortcuts (including Vim toggle) → document Vim interpreter or standard key actions → platform text insertion. GPUI key contexts must prevent the editor from hijacking other widgets. Confirm focus restoration on menu dismissal and after Save/Open dialogs.

**Shortcut policy:** Ctrl+Shift+V (Windows/Linux) toggles Vim; Ctrl/Cmd+S saves, Ctrl/Cmd+O opens, Ctrl/Cmd+N creates window, Ctrl/Cmd+F finds, standard copy/paste/undo defaults. Check the platform key equivalent API instead of treating `Ctrl` as universally correct. While Vim Normal is active, ordinary character keystrokes become Vim commands; `Ctrl+Shift+V` remains a global mode toggle.

## Text geometry, hit testing and rendering

Rendering pipeline for each frame:

1. Compute content viewport from `WindowFrame`: width/height, padding, scrollbar/hit region, font metrics, line height, DPI scale.
2. Determine **visible visual rows** including e.g. 2 screens' overscan; map visual rows → hard lines and wrap segments. Avoid shaping the entire document merely to find the viewport: keep incremental wrap-height per-line index, with coarse estimates corrected lazily.
3. Request `TextRun`s for visible text with syntax styles; shape through GPUI's text system, cache layouts using `(doc_revision/line_revision, font, size, wrap_width, tab width, theme/highlight_revision, DPI)`.
4. Paint selection backgrounds, current-line background (optional), glyphs, matching brace hints (optional), IME underlines, caret(s), and minimally styled gutters. Clip tightly to the document viewport.
5. Mouse screen point → visual row → shaped line X hit-test → UTF-8 offset within line → doc char index → nearest grapheme boundary. Use font shaping/hit-test information rather than fixed `x / char_width`, even for monospaced fonts (tabs, ligatures, combining glyphs).

Soft wrap changes **visual rows**, never adds `\n` to the Rope. Preserve preferred horizontal X while moving up/down across lines of differing lengths. Horizontal scrolling is used when wrap is disabled. Scrolling/drag selection must auto-scroll at viewport edges. On resize, recompute wrap segments while retaining the logical caret anchor and sensible viewport position.

**BiDi policy:** proper selection and hit-testing in RTL/mixed-direction text is hard. Use GPUI's shaping information when available and test it; if correct BiDi support cannot be provided in v0.1, document this limitation and do not claim complete Unicode editing parity. Never corrupt text when rendering is imperfect.

## Point-and-click behavior: required acceptance tests

- Single-click caret, drag extends selection, Shift+click extends anchor; double-click selects Unicode-aware word, triple-click selects hard line; right-click menu keeps selection and opens native-feeling Copy/Cut/Paste/Select All.
- Arrow, Ctrl+Arrow / platform equivalents, Home/End and Shift versions; Backspace/Delete remove grapheme clusters, not individual bytes; Tab indent/outdent with `Shift+Tab`; Enter retains indent.
- Word wrap on by default for notes; per-language or per-document override may turn it off for code. Line numbers off by default. Selection highlighting remains visible while focused; appropriately dim when inactive.
- Ctrl/Cmd+Z and redo work across typing/paste and mouse operations. A failed Save never clears dirty state.
- Find/search and replace: literal search first, case-sensitive toggle, next/previous, replace-one and replace-all one transaction; avoid regex semantics initially.
- Accessibility/text services: basic keyboard-only navigation and screen-reader text range/bounds where supported by the pinned GPUI build. Record platform limitations, don't pretend a visual label alone is accessible.

## Performance and profiling

Set **budgets to measure**, not guessed guarantees: cold start, open 10 MiB/100k lines, steady typing p50/p95 input-to-paint, scroll speed, wrap recalculation, peak memory, incremental parse duration. Instrument `Document::apply`, rewrap, line shaping, paint, query, save and formatter dispatch. Expose timings in debug builds/logs, not a permanent UI.

Use caches invalidated by local edits, not all-content redraws; background file I/O and parse; bounded work per frame for huge files and extremely long single lines; clear fallback to plain text if a parser fails. Keep a warm 1–10 MiB benchmark fixture and a pathological one-line megabyte fixture.

## Do not

- Do not infer offsets from rendered glyph count or assume tabs are one visual cell.
- Do not call `rope.to_string()` on each keystroke, paint all lines, or synchronous full-document reparse.
- Do not equate mouse selection with Vim Visual state; pointer interactions should use the same underlying selection operations in both modes, with explicit mode transitions.
- Do not register separate Save/Undo/Find callbacks for the menu, toolbar (none), context menu and keyboard.
- Do not destroy editor entities during theme changes, settings updates or highlighting refresh.
- Do not mutate the rope from a parser thread; background jobs receive immutable text snapshots and publish revision-tagged results.

## References

[Pinned GPUI input trait](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/src/input.rs); [pinned GPUI input example](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/examples/input.rs); [Silvestris input reference](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/controls/text_field.rs); [Silvestris command routing](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/docs/commands.md).
