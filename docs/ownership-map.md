# Ownership map after the cleanup pass

This is the current map of mutable lifecycles. It supersedes the provisional
`EditorState { document_handle, selection(s), preferred_x, scroll, wrap, ... }`
sketch in [editor.md](editor.md). That sketch described the intended split; the
types below are the split that exists in the tree.

## Document and text

`chaus-core` owns the live text. `Document::apply()` is the only mutation
authority for the Rope and its EOL map; undo/redo, IME commits, Vim operators,
paste and formatter results all go through it. The edit vocabulary lives in
`edit.rs`, `Utf16Index` in `utf16_index.rs`, the undo/redo stacks and coalescing
state in `history.rs`, and immutable background-work data in `snapshot.rs`.
`History` never touches the Rope: `Document` applies the replay data it returns.

Coordinates are newtypes at module boundaries: Rope scalars (`LineCharOffset`),
expanded display columns (`VisualColumn`), row-local caret preference
(`RowColumn`), visual rows (`WrapRowIndex`), display bytes (`DisplayByteOffset`),
wrap width (`WrapWidthColumns`), and line-relative/document byte offsets in
`chaus-core::position`. Raw `usize` appears only where Ropey, tree-sitter or GPUI
requires it.

## Text layout

`chaus-layout/src/lib.rs` is a facade over flat leaves: `units.rs`,
`line_source.rs`, `rows.rs`, `checkpoints.rs`, `display.rs` and `source.rs`.
`LineSource::window()` takes Unicode scalar offsets and converts them to byte
offsets explicitly. Wrapped rows use `WrapCheckpoints`; unwrapped horizontal
mapping uses the separate `ColumnCheckpoints` and `unwrapped_window()`. The
unwrapped path returns one row without scanning the line, and painting receives
only a visible source window plus fixed overscan. `LayoutWorkCounters` reports
wrapped and unwrapped source reads separately. Find-bar width is editor UI
geometry (`chaus-editor::geometry::find_bar_width`), not layout policy.

## Editor

`Editor` keeps `Document`, `Selection`, `Settings` and its primary `FocusHandle`
directly. The remaining state belongs to private owners in flat leaf modules:
`ViewportState`, `PointerState`, `TextInputState`, `FileSession`,
`RecoveryState`, `VimState`, `BlinkState`, `FormatState`, `MenuState`,
`HighlightState` and `StatusState`. `Option<FindBar>` and the close/conflict
prompt focus handles stay on `Editor` because they have no independent
lifecycle. Each owner's fields are private to its module; `Editor` coordinates
cross-subsystem transitions and `after_document_mutation()` is the one
mutation-completion path: it invalidates file overwrite decisions, refreshes
rows, schedules recovery and highlighting, adjusts caret/scroll and notifies.
Sibling adapters call narrow methods instead of reaching into another owner's
primitive fields. `Editor::render()` is composition plus one action-binding
adapter. Any active modal sheet removes the editor key context at the dispatch
boundary through `Editor::key_context()`; `modal_sheet_open()` is the single
predicate for "the document must not receive editing input". Recovery keeps
its interceptor because it can arrive after focus exists.

## Files, formatter and recovery

`chaus-io` is a facade over `loaded_file.rs`, `file_meta.rs`, `conflict.rs`,
`save.rs`, `atomic_replace.rs`, `path.rs` and `recovery.rs`. It owns document
file lifecycle and crash snapshots. Snapshot storage stays here because it
reuses the document loader and the same platform replacement contract; copying
those contracts into a separate crate was explicitly rejected. Formatter
execution is independent and lives in the std-only `chaus-format` crate.

## Settings

`SettingsGlobal` remains the authoritative live value applied to open windows.
`SettingsStore` is the only owner of runtime `_chausrc`/`.chausrc` interaction:
queue generations, in-flight identity, pending snapshot, error state, reload
barriers and shutdown state. Preferences subscribes to it and derives its
status during render; it performs no blocking filesystem read. Startup loading
happens before GPUI exists and its result is stored in the persistence owner.
Shutdown is serialized through the store: no older writer can commit after the
final snapshot.

## Vim

`chaus-vim::Engine` keeps mode, pending operator, count, register and visual
anchor together. Mapping, command-line, repeat and search state are private
substates with narrow methods. `normal_char()` remains the Normal-mode dispatch
table and `motion::resolve()` remains an explicit motion table; no trait-based
state-machine framework was introduced.
