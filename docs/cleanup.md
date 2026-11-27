Rechecked against **Chaus `master` at `8ba0c457`** and **style-guides at `62dae7cc`**. The previous findings still stand, with two refinements: `chaus-layout/src/lib.rs` is currently a leaf module, so it is not itself violating the “non-leaf modules only organize/re-export” rule; it is instead an oversized mixed-responsibility module. I also found two layout problems worth folding into this pass: `display_boundaries_source()` has confused window-relative/absolute offsets, and `find_bar_width()` is editor UI geometry living in the generic layout crate. `chaus-vim::Engine` is also a secondary god-state object and should be reduced after the editor cleanup.

The bounds for this pass are: **no UI redesign, no new editor features, no generic framework, no rewrite of Ropey/GPUI/Silvestris integration, no arbitrary line-count-driven splitting, and no changing working behavior except where fixing verified bugs.** Preserve `Document::apply()` as the single mutation authority, preserve the existing `FileOperation` request-identity work, preserve `after_document_mutation()` as the shared mutation-completion path, and do not split coherent functions such as `Document::apply()` or `motion::resolve()` merely because they are long.

1. **Freeze the current contracts with targeted regression tests before moving ownership around.**

   **Deliverables**
   - Add new tests that fail against the specific defects and risks we are about to touch. Do this before the structural refactor so the implementer cannot accidentally “clean up” behavior without noticing.
   - Add a close-sheet keyboard regression test that calls the real `chaus_editor::bind_keys()`, opens a dirty-close sheet, and proves `Backspace`, `Delete`, `Enter`, `Tab`, arrow keys, ordinary printable input, paste and Escape do not mutate the document behind it. The existing `close_sheet.rs` does not bind the editor keymap, so it does not reproduce the actual dispatch ordering.
   - Add the equivalent conflict-sheet test with real editor bindings.
   - Add `LineSource::Text::window()` tests for:
     - `"abcdef"` with non-zero scalar start/end.
     - `"aé漢👩‍💻z"` with scalar offsets around every multi-byte character.
     - start == end, start == len, and end == len.
     - Assert the exact returned substring and never merely its length.
   - Add a test for `display_boundaries_source()` using a **later** segment whose `char_start` is well beyond zero, with tabs and Unicode before it. Existing coverage mostly proves the first segment.
   - Add Rope-backed no-wrap tests using at least a 1 MiB hard line:
     - row count is one without whole-line materialization;
     - painting the beginning stays bounded;
     - painting after a deep horizontal scroll stays bounded;
     - mouse/caret lookup beyond character 4096 returns the correct source position;
     - selection beyond character 4096 remains correct;
     - tabs, CJK, combining marks and ZWJ emoji occur both before and around the deep target.
   - Add deterministic settings tests for:
     - old write in flight → newer setting queued → shutdown;
     - old write cannot finish after shutdown and restore stale settings;
     - write failure arriving asynchronously becomes visible to Preferences;
     - explicit Reload while a save is pending cannot later be overwritten by that older save.
   - Use gates/channels or GPUI's deterministic executor. **Do not use arbitrary sleeps.**
   - Keep these tests in new files where practical, following Chaus's own test rule.

   **Avoidables**
   - Do not change production architecture in this task.
   - Do not weaken existing tests because refactoring makes them inconvenient.
   - Do not replace meaningful integration tests with unit tests of a newly invented helper.
   - Do not assert implementation details such as exact field count or exact helper names; assert observable invariants.

   **Regression traps**
   - GPUI delivers key bindings before some element handlers. The tests must install real Chaus bindings or the modal bug disappears.
   - Windows printable input can arrive separately from key-down; testing only `KeyDownEvent` is insufficient.
   - Allocation tests must use the existing per-thread accounting pattern so parallel tests do not contaminate the result.
   - Test giant **Rope-backed** lines, not only contiguous `String` paths. The Rope path is the production editor path.

   **Citations:** [Rust guide — application code should favor directness over unnecessary abstraction](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L180-L190), [Rust guide — contextual assertions and readable implementation](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L1280-L1385), [close-sheet test](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/tests/close_sheet.rs), [conflict-sheet test](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/tests/conflict_sheet.rs), [current layout tests](https://github.com/estyseesghosts/chaus/tree/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-layout/tests).

2. **Finish modal keyboard ownership instead of special-casing only Recovery.**

   **Deliverables**
   - Change the editor key-context decision in `Editor::render()` so **any** active modal sheet uses `MODAL_KEY_CONTEXT`, not only a non-empty recovery queue.
   - Use the existing `modal_sheet_open()` predicate as the single definition of “the document must not receive editing input.”
   - Preserve the recovery interceptor. It solves a specific race that close/conflict sheets do not necessarily have: recovery can arrive asynchronously after document focus already exists.
   - Do **not** automatically generalize the recovery interceptor into a universal modal framework. Close/conflict sheets can use their existing focused-button and sheet key handlers once the editor key context is removed.
   - Ensure close/conflict Tab navigation still works under the modal context.
   - Ensure Escape still dismisses/cancels the correct sheet now that the ordinary `ChausEditor` Escape binding is unavailable.
   - Preserve the existing `EntityInputHandler` checks that reject text and paste while `modal_sheet_open()` is true.
   - Make modal ownership explicit enough that future sheets cannot accidentally forget it. A narrow method such as `Editor::key_context()` or `Editor::input_owner()` is acceptable; a generic `ModalManager` is not necessary.
   - Add regression coverage from Task 1 for every editor mutation key that could otherwise run behind the sheets.

   **Avoidables**
   - Do not sprinkle `if self.close_prompt { return; }` through every editing action. The dispatch boundary should prevent those actions from firing.
   - Do not make Backspace/Delete/Enter handlers individually understand modal sheets.
   - Do not remove the recovery interceptor merely because `MODAL_KEY_CONTEXT` exists; recovery's first-frame/focus race is different.
   - Do not handle Enter manually at both the sheet and button level and create double activation.

   **Regression traps**
   - Recovery's single-activation behavior on Enter/Space must remain intact.
   - Command shortcuts must resume immediately when the sheet closes.
   - Vim Normal/Insert/CommandLine ownership must return to exactly its prior state after dismissal.
   - Save-from-close → conflict → Cancel must still return to the close sheet.
   - Save-from-close → conflict → Overwrite must still complete the original close intent.

   **Citations:** [Rust guide — straightforward control flow and explicit APIs](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L180-L190), [current key-context selection in `editor.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L1042-L1110), [`MODAL_KEY_CONTEXT`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_keys.rs#L6-L20), [`modal_sheet_open()` and recovery ownership](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_recovery.rs#L180-L230), [close sheet](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_close.rs#L31-L126), [conflict sheet](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_conflict.rs#L30-L130).

3. **Repair the layout layer properly, including the no-wrap giant-line path, and decompose `chaus-layout` around real ownership.**

   This is the largest correctness task. Do not “fix” it by increasing `WINDOW_CHARS`.

   **Deliverables**
   - Split `chaus-layout/src/lib.rs` into **flat leaf modules**, while keeping one `chaus-layout` crate. Recommended responsibility split:
     - `units.rs`: `LineCharOffset`, `VisualColumn`, `WrapRowIndex`, `DisplayByteOffset`, plus new semantically necessary layout units.
     - `line_source.rs`: `LineSource` and bounded Rope/string window extraction.
     - `rows.rs`: `WrapPolicy`, `Segment`, `Rows`, wrapped-row iteration and row-breaking.
     - `checkpoints.rs`: wrapped-row checkpoints and the new horizontal/column indexing described below.
     - `display.rs`: tab expansion, display boundaries and source/display mapping.
     - `lib.rs`: module declarations, public re-exports and crate documentation only.
   - Do not create a `src/layout/` directory. Keep the crate flat, matching the style guide.
   - Fix `LineSource::Text::window()`. `start` and `end` are **Unicode scalar indices**; `str::char_indices()` yields **byte offsets**. Convert scalar offsets to byte offsets explicitly. A request for scalar range `1..3` must return exactly scalars 1 and 2.
   - Fix `display_boundaries_source()` so there is one unambiguous coordinate system:
     - preferably materialize exactly the already-known grapheme-safe segment range;
     - use the segment's known logical `start_column`;
     - make the local segment begin at zero;
     - remove the current absolute-`start`/window-relative mixing.
     - If the function remains public, its API must make it impossible or difficult to supply a column belonging to a different starting offset.
   - If repository-wide call analysis confirms `display_boundaries_source()` has no production consumer, it is acceptable to delete it rather than preserve a broken unused public API. Do **not** retain dead API merely to avoid changing tests.
   - Move `find_bar_width()` out of `chaus-layout`. It is Chaus editor UI geometry and is currently used by `editor_find.rs`; put it in that editor capability or a Chaus editor geometry helper. Generic text layout should not know the preferred width of Chaus's find UI.
   - Introduce a dedicated **unwrapped-line path**. `max_columns == None` must not pass through an algorithm that needs to discover the end of the entire line before it can produce the one logical row.
   - For no-wrap mode:
     - `row_count_source()` should return `1` for a hard line without scanning/materializing it.
     - `row_of_char_source()` should return row zero directly.
     - do not build wrapped-row checkpoints for a one-row unwrapped line.
   - Add a separate `ColumnCheckpoint`/`ColumnCheckpoints` concept for horizontal mapping. A wrapped-row checkpoint and a horizontal column checkpoint are not the same abstraction.
   - Column checkpoints should contain at minimum:
     - scalar `LineCharOffset`;
     - absolute `VisualColumn`.
   - Build/grow column checkpoints incrementally at a fixed bounded scalar/grapheme stride. Do not retain display strings in the index.
   - `column_at_source()` must start from the nearest scalar checkpoint at or before the requested scalar offset.
   - `char_at_column_source()` must start from the nearest visual-column checkpoint and continue bounded scanning until the requested column is reached or EOF is proven. **It must never silently stop because one 4096-character window ended.**
   - Add an unwrapped visible-window API for painting. It should accept the horizontal visible range/overscan and return only the text/source range required to shape the visible part of the hard line.
   - `editor_paint.rs` must use that API when wrap is `None`; it must not call `shape_line` with an entire multi-megabyte line just because clipping later hides most of it.
   - Include horizontal overscan so small scroll movements do not continually reshape single characters, but keep the overscan fixed/bounded.
   - Ensure syntax highlight runs are clipped/rebased onto the visible source window correctly.
   - Keep wrapped mode on its existing row-checkpoint path; do not make the wrapped path pay for horizontal indexing it does not need.
   - Update `LayoutWorkCounters` or equivalent diagnostics to separately show how much source was scanned/materialized for wrapped and unwrapped paths.

   **Avoidables**
   - Never call `RopeSlice::to_string()` over an entire giant hard line in paint/hit-test/reflow paths.
   - Never “solve” deep unwrapped hit testing by setting `WINDOW_CHARS` to several megabytes.
   - Do not pretend `RowCheckpoint` can represent horizontal indexing just to avoid defining the correct type.
   - Do not cache one `String` per row or per horizontal chunk.
   - Do not introduce a layout trait hierarchy. These are concrete algorithms with concrete data.
   - Do not create a separate crate for every new layout module.
   - Do not change wrapping semantics while doing the decomposition.

   **Regression traps**
   - Tabs depend on the **absolute logical-line column**, even when the visible substring starts far into the line.
   - Combining marks/ZWJ sequences cannot be cut in the middle.
   - CJK/emoji width must remain consistent between painting, caret movement and pointer hit testing.
   - Horizontal scrolling must still clamp correctly.
   - A click to the right of the rendered text must clamp to the actual line end, not to the end of the current materialized window.
   - Wrapped selection geometry must remain row-local; this was fixed recently and must not regress.
   - Background reflow generation/revision rejection must remain intact.

   **Citations:** [Rust guide — code organization and flat modules](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L192-L300), [Rust guide — newtypes for semantically different values](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L620-L660), [`LineSource::window()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-layout/src/lib.rs#L365-L445), [`display_boundaries_source()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-layout/src/lib.rs#L220-L330), [`Rows::next()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-layout/src/lib.rs#L650-L816), [Rope window/checkpoint path](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-layout/src/lib.rs#L1044-L1356), [`editor_paint.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_paint.rs#L35-L125), [`editor_pointer.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_pointer.rs#L170-L225), [`find_bar_width` consumer](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_find.rs#L250-L280).

4. **Replace the split-brain settings writer/reloader with one settings persistence owner.**

   **Deliverables**
   - Introduce one `SettingsStore`/`SettingsPersistence` entity in `chaus-app`. This component owns **all runtime disk interaction with `_chausrc`/`.chausrc`** after application startup.
   - Keep `SettingsGlobal` as the authoritative live settings value used by open windows. Do not merge UI state and persistence state into one giant object.
   - `SettingsStore` should own:
     - settings path;
     - last disk diagnostics;
     - latest requested settings snapshot;
     - write generation;
     - in-flight write identity;
     - pending latest write;
     - last completed generation;
     - current write error;
     - reload generation/state;
     - explicit shutdown state.
   - Keep the existing coalescing behavior: one write at a time, newest pending snapshot wins.
   - Preferences must **subscribe** to the persistence entity. Do not snapshot `visible_error()` immediately after queueing and assume it will update later.
   - When a background write succeeds or fails, publish the new status and notify subscribers. Preferences should derive `save_error` from the store's current state during render/update rather than maintaining an unsynchronized copy.
   - Preserve startup loading before GPUI starts; that synchronous read is acceptable because no UI thread exists yet. Store the resulting path/diagnostics in the persistence owner when the app initializes.
   - Remove the synchronous `read_settings()` call from `Preferences::new()`.
   - Make Preferences Reload asynchronous:
     - explicit Reload establishes a new persistence epoch/barrier;
     - pending not-yet-started writes from the old UI state are abandoned because the user explicitly chose disk as source of truth;
     - an already-running older write must settle before the reload read is accepted;
     - the reload result is generation checked before it can replace `SettingsGlobal`;
     - if the user changes settings while reload is running, the old reload result must not overwrite those newer settings.
   - Replace the current shutdown shortcut that directly calls `write_settings_rc()` alongside a possible older writer.
   - Shutdown must have one serialized path:
     - stop accepting new settings requests;
     - let/cancel-by-generation the active operation according to a defined state transition;
     - guarantee no older writer can commit after the final settings snapshot;
     - persist the newest accepted settings exactly once after earlier writes can no longer win;
     - only then allow application shutdown to finish.
   - Prefer making application shutdown asynchronous rather than blocking the GPUI thread for an arbitrary interval.
   - Move app-specific rc merge/read/write code into an app leaf module such as `settings_file.rs`; the generic document I/O crate should not own Chaus Preferences policy.

   **Avoidables**
   - Do not add another mutex-protected global next to `SettingsWriterGlobal`.
   - Do not have Preferences write the file itself.
   - Do not solve the shutdown race by adding `thread::sleep`.
   - Do not make every preference control wait for disk before updating the live app.
   - Do not silently discard disk errors.
   - Do not let Reload and Save run as two independent pipelines.

   **Regression traps**
   - Rapid steppers/theme toggles must still coalesce.
   - An old failed generation cannot replace the error belonging to a newer generation.
   - An old success cannot clear a newer failure.
   - External rc edits must not be overwritten silently.
   - A failed persistence operation must not revert the live in-memory theme/font/etc.
   - Closing immediately after changing a preference must persist the newest accepted value.
   - Reopening Preferences must show any still-current persistence failure.

   **Citations:** [Rust guide — use `Result` for recoverable I/O failures](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L575-L625), [Rust guide — coherent code organization](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L192-L300), [`settings_writer.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-app/src/settings_writer.rs#L1-L147), [`Preferences::apply/reload`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-app/src/preferences.rs#L80-L185), [current shutdown path](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-app/src/app.rs#L265-L287), [`write_settings_rc`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-io/src/lib.rs#L800-L860).

5. **Turn `Editor` from an 84-field shared state bag into a coordinator with explicit subsystem owners.**

   This task is primarily structural. Do not mix major behavior changes into it.

   **Deliverables**
   - Keep these truly central values directly on `Editor`:
     - `Document`;
     - `Selection`;
     - `Settings`;
     - primary editor `FocusHandle`.
   - Replace the remaining primitive field clusters with cohesive private state types. Use approximately these boundaries:
     - `ViewportState`: preferred row-local column, vertical/horizontal scroll, geometry, row index, wrap/column indexes, shaping cache, layout counters, committed wrap configuration, reflow request/in-flight/generation/publication state, zoom.
     - `PointerState`: selecting flag, drag pointer, drag generation, autoscroll state.
     - `TextInputState`: marked range, preedit start/active state, suppress-next-text-input state.
     - `FileSession`: current file metadata, `FileOperation`, request counter, document generation, forced-overwrite decision, pending close/open intent, disk watcher epoch/in-flight/change state, WQA state, test load hook.
     - `RecoveryState`: recovery queue, focus handles, key/id/epoch, selection, interceptor/window identity, original path/BOM, directory/adopted recovery path.
     - `VimState`: enabled flag, `Engine`, mapping timeout generation.
     - `BlinkState`: visible flag, timer epoch and blink key.
     - `FormatState`: formatter override, in-flight flag and generation.
     - `MenuState`: command registry, popup entity and pending popup request.
     - `HighlightState`: language/override, spans/revisions, tree/revision, generations, worker state, pending edit, statistics.
     - `StatusState`: visible message and epoch.
     - `FindState` only if it has enough behavior/state to justify a type; otherwise keeping `Option<FindBar>` directly is acceptable.
   - These are **responsibility boundaries**, not mandatory exact names. Do not invent extra wrappers where no independent lifecycle exists.
   - Make fields *inside* each subsystem state private to its defining module.
   - It is acceptable for `Editor` to expose `pub(crate)` subsystem fields if Rust module privacy otherwise prevents sibling adapters from calling methods, but no sibling module should be able to directly mutate the subsystem's primitive internals.
   - Give each state type a real constructor/default with its invariant established in one place.
   - Rewrite `Editor::new()` so it constructs these owners rather than assigning dozens of individual counters/options/booleans.
   - `Editor::new()` should still perform editor-level orchestration: determine loaded document/language, construct state owners, register commands, start initial highlight/watch/layout work.
   - Do **not** hide `Document` inside an `EditorState`/`EverythingState`; that creates the same god object one level lower.
   - Add module docs explaining the responsibility of each owner and what it explicitly does **not** own.

   **Avoidables**
   - No `EditorState`, `EditorManager`, `EditorContext` or `CommonState` containing the same 84 fields.
   - No one-field wrapper types merely to make the struct visually shorter.
   - No public mutable getters returning `&mut FileSession`, `&mut RecoveryState`, etc. to every sibling module.
   - Do not create traits for every state owner. There is one implementation of each.
   - Do not move all current `impl Editor` blocks unchanged into differently named files and call that decomposition.

   **Regression traps**
   - Construction order matters for focus handles and command registration.
   - Loaded-file language detection must happen exactly once.
   - Recovery IDs must remain per document/window as intended.
   - Reflow/highlight generations must begin in the same valid state.
   - The disk watcher must remain tied to the editor entity lifetime.
   - Vim live mode and settings-default Vim mode must remain distinct.
   - Test seams must not leak into normal state transitions.

   **Citations:** [Rust guide — organize around real abstractions and avoid premature generality](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L180-L230), [Rust guide — name leaf modules around their primary item](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L250-L280), [`Editor`'s current state bag](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L83-L225), [`Editor::new()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L228-L357), [`FileOperation` is already a useful state boundary](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/file_operation.rs#L1-L130).

6. **Move behavior to the state owner; make `Editor` orchestrate cross-subsystem transitions instead of letting every module mutate everything.**

   This is what makes Task 5 a real decomposition rather than cosmetic nesting.

   **Deliverables**
   - Move state-local transitions onto their owner:
     - `FileSession::begin_request`, request ownership checks, file-operation transitions, document-generation changes, disk epoch changes, overwrite decision, pending close intent.
     - `RecoveryState`: recovery IDs/epochs, queue navigation, adopted path, selection/interceptor ownership and stale-operation rejection.
     - `HighlightState`: generation creation, worker ownership, accepted/discarded publication, language override bookkeeping.
     - `ViewportState`: row/cache/index replacement, reflow generation/publication, scroll state and geometry.
     - `StatusState`: set/clear/epoch behavior.
     - `BlinkState`: whether a timer is current and whether caret is visible.
     - `FormatState`: generation and in-flight ownership.
   - Pass explicit inputs into those methods. Example: `ViewportState::refresh_rows(&Document, LineRange)` is better than giving ViewportState access to the whole Editor.
   - Preserve cross-subsystem coordinator methods on `Editor`. `after_document_mutation()` is exactly the sort of method that should remain:
     - tell file state that a new edit invalidates force-overwrite;
     - tell viewport which lines changed;
     - schedule recovery;
     - schedule/update highlighting;
     - adjust caret/scroll;
     - notify.
   - Do not duplicate that mutation completion path inside Vim, IME, formatter, standard editing, etc.
   - Change `editor_watch.rs` to use `FileSession` methods rather than directly changing `disk_epoch`, `disk_check_in_flight`, `disk_change`, etc.
   - Change `editor_element.rs` so it no longer directly writes primitive Editor fields such as `pending_menu` and `geometry`. Call narrow methods such as `queue_menu_request()` and `set_geometry()`.
   - Change menu/highlight/recovery/file modules similarly. Direct reach-through like `editor.highlight_generation += 1` from another responsibility should disappear.
   - Use privacy as an architectural test: if unrelated code cannot compile without opening another subsystem's private fields, add the narrow operation it actually needs instead of widening visibility.
   - Keep the dependency direction:
     - state owner knows only its own data and explicit value types;
     - Editor knows the owners;
     - UI adapters know Editor's narrow commands;
     - state owners do not know the entire Editor.
   - Reduce `Editor::render()` to composition plus dispatch wiring. Its current 300-line size is partly verbose GPUI setup, so do not chase a line target.
   - Extract the long `.on_action(...)` wiring into one adapter/builder function **only as a single coherent concern: binding editor actions to Editor methods**. Do not split one action per file or invent a macro DSL.
   - Move render-time state preparation into narrow methods where appropriate, but do not fight GPUI's lifecycle merely to make render “pure.”

   **Avoidables**
   - Do not expose every state owner through `*_mut()` and let arbitrary callers mutate it.
   - Do not create circular owner dependencies.
   - Do not put a GPUI `Context<Editor>` inside low-level state objects unless that state truly owns an async GPUI lifecycle.
   - Do not make layout depend on file state, recovery depend on highlighting, etc.
   - Do not duplicate `after_document_mutation()`.

   **Regression traps**
   - All async completions still require generation/request checks.
   - A save completion from a replaced document must still be ignored.
   - A highlight result from the wrong revision/language must still be discarded.
   - Recovery retirement and late snapshot ordering must remain correct.
   - Menu checked state must still follow the current window.
   - Pointer autoscroll timer must die with the owning drag/editor.

   **Citations:** [Rust guide — small coherent modules and direct application design](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L180-L300), [Rust guide — getters/setters should not expose implementation accidentally](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L1015-L1055), [`after_document_mutation()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_edit.rs#L1-L60), [`editor_watch.rs` direct shared-state mutation](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_watch.rs#L80-L220), [`editor_element.rs` direct editor-state access](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_element.rs#L1-L130), [`Editor::render()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L1042-L1347).

7. **Stop using `chaus-io` as a catch-all: give document I/O, recovery and formatter execution real boundaries.**

   **Deliverables**
   - First decompose the current 882-line non-leaf `chaus-io/src/lib.rs`.
   - The final `chaus-io/src/lib.rs` should contain crate docs, module declarations and deliberate re-exports. It should not continue implementing loading, conflict checking, saving, platform replacement and settings persistence alongside child modules.
   - Use clear flat leaf modules, approximately:
     - `loaded_file.rs` or `load.rs`: `LoadedFile`, `LoadError`, UTF-8/BOM/binary validation and `load`.
     - `file_meta.rs`: `FileMeta`, `FileIdentity`, metadata/identity capture.
     - `conflict.rs`: `SaveConflict`, `ConflictCheckError`, `metadata_matches`, `check_conflict`.
     - `save.rs`: `SaveRequest`, `SaveError`, save orchestration and encoding.
     - `atomic_replace.rs`: temporary-file commit, platform replacement and durability primitives.
     - `path.rs`: existing explicit path expansion.
   - Isolate Windows/Unix unsafe platform replacement inside the atomic-replace leaf. Every unsafe block must have a `SAFETY:` explanation describing the platform API preconditions and pointer/lifetime validity.
   - Move rc-settings merge/write policy to `chaus-app` as described in Task 4.
   - Promote `format.rs` to a **`chaus-format` crate** unless the refactor uncovers a real dependency on document-file internals. Current source does not have one: it is an external-process execution capability.
   - Promote `recovery.rs` to a **`chaus-recovery` crate** unless the refactor uncovers a real dependency on document load/save internals. Current source depends on `chaus-core::eol` and filesystem primitives, not the document-file session.
   - Do not blindly make new crates for `path`, conflict checking or atomic replace; those belong to the document-file I/O capability and share its platform contract.
   - Update `chaus-editor` dependencies/imports explicitly:
     - document file lifecycle → `chaus-io`;
     - formatter execution → `chaus-format`;
     - crash snapshots → `chaus-recovery`.
   - Remove transitional re-exports once all internal call sites use the correct owner. Do not leave compatibility aliases forever inside this private workspace.
   - Keep all existing file safety semantics intact.

   **Avoidables**
   - No `utils.rs`, `common.rs` or `platform_stuff.rs`.
   - Do not move 800 lines into a single `file.rs`.
   - Do not copy atomic-save code into recovery/format just to avoid dependencies.
   - Do not allow the formatter crate to know about `Document`, GPUI or editor state.
   - Do not let recovery know about GPUI prompts; recovery crate owns snapshot storage/protocol, editor owns UI.

   **Regression traps**
   - UTF-8 BOM preservation.
   - Mixed EOL preservation.
   - symlink and hard-link handling.
   - changed/deleted-on-disk conflict behavior.
   - create-new/replace semantics.
   - directory sync/durability-unconfirmed behavior.
   - Windows replace semantics and permissions.
   - formatter timeout/output cap/process kill.
   - recovery record compatibility, size limits, age limits and epoch/watermark ordering.

   **Citations:** [Rust guide — prefer small crates where capabilities are truly independent](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L192-L230), [Rust guide — non-leaf modules should organize/re-export](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L267-L290), [Rust guide — isolate and justify unsafe](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L990-L1030), [`chaus-io/src/lib.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-io/src/lib.rs), [`format.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-io/src/format.rs#L1-L80), [`recovery.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-io/src/recovery.rs#L1-L80).

8. **Decompose `Document` internally without weakening the single-document transaction authority.**

   `document.rs` is large, but its central mutation algorithm is coherent. The correct refactor is to extract the independent machinery around it, not fracture `Document::apply()`.

   **Deliverables**
   - Create a flat `edit.rs` containing the edit vocabulary:
     - `EditCause`;
     - `TextEdit`;
     - `EditTransaction`;
     - `AppliedEdit`;
     - `EditResult`;
     - `EditError`;
     - `LineRange`;
     - Tree-sitter edit-point types if they naturally belong with edit results.
   - Create `utf16_index.rs` containing `Utf16Index` and its Fenwick-tree implementation.
   - Create `history.rs` containing a real `History` owner for:
     - undo stack;
     - redo stack;
     - state identities;
     - grouping depth/start;
     - coalescing metadata.
   - `Document` should hold `history: History` instead of independently owning all of those primitives.
   - **History must not mutate the Rope.** It owns history bookkeeping and returns replay data/plans. `Document` remains the component that applies inverse/forward text/EOL changes.
   - Move `DocumentSnapshot` to `snapshot.rs` if doing so leaves a cleaner document model boundary. Snapshot is immutable background-work data and has a distinct responsibility.
   - Keep `Document::apply()` in `document.rs`.
   - You may extract a small validation helper for:
     - empty edits;
     - bounds;
     - overlap;
     if it makes the transaction path easier to read. Do not split every phase into five-line methods.
   - Keep `undo()`/`redo()` public behavior on `Document`; internally delegate stack/state movement to `History`.
   - Ensure the public crate surface remains deliberate through `chaus-core/src/lib.rs` re-exports/modules.
   - Keep Rope/EOL invariants enforced in one place.

   **Avoidables**
   - Do not create a second document mutation API in `History`.
   - Do not let `HistoryEntry::undo()` own arbitrary `&mut Document` if that merely recreates circular ownership after the move.
   - Do not introduce an `EditEngine` beside Document.
   - Do not split `Document::apply()` purely because it is ~155 lines.
   - Do not change history semantics while extracting it.

   **Regression traps**
   - typing coalescing window;
   - branching after undo;
   - saved-state identity versus stack depth;
   - grouped Vim operations;
   - IME commit grouping;
   - EOL undo/redo;
   - multi-edit range rebasing;
   - UTF-16 conversions around surrogate pairs;
   - UTF-16 index updates after line insertion/removal;
   - Tree-sitter byte/point edit ranges.

   **Citations:** [Rust guide — leaf modules around primary abstractions](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L240-L290), [Rust guide — named types/newtypes for distinct semantics](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L620-L675), [`document.rs` edit model and UTF-16 index](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-core/src/document.rs#L1-L340), [history and `Document` fields](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-core/src/document.rs#L340-L520), [`Document::apply()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-core/src/document.rs#L888-L1042).

9. **Reduce `chaus-vim::Engine` from another broad state bag into explicit modal subsystems, without building a Vim framework.**

   **Deliverables**
   - Keep the public `Engine` API stable where possible. The editor should still treat it as one modal engine.
   - Internally group its current state by lifecycle:
     - `MappingState`: mappings, mapping prefix and queued mapped/direct keys.
     - `CommandLineState`: active `CommandLine`, command generation, completion sequence/latest token, Ex history and search history.
     - `RepeatState`: last change, insert start/keys/repeat state and replaying state.
     - `SearchState`: last search and last `f/F/t/T` find state.
     - main modal command state can keep `Mode`, `Pending`, counts, visual anchor and register if those remain tightly coupled.
   - Keep fields inside those substate types private.
   - Move mapping resolution onto `MappingState`.
   - Move command-line generation/completion/history operations onto `CommandLineState`.
   - Move dot-repeat/insert-repeat bookkeeping onto `RepeatState`.
   - Remove `changed`/`message` side channels if the existing regression suite allows it cleanly:
     - prefer helpers returning meaningful `Outcome`s;
     - merge outcomes explicitly in `handle_key()`;
     - do not store transient output in Engine fields merely so a later layer can discover what happened.
   - After state ownership is clear, reassess `engine_normal.rs`.
   - Keep `normal_char()` as the Normal-mode dispatch table; that is a coherent job.
   - Extract a cluster from `engine_normal.rs` only when it can accept explicit inputs rather than `&mut Engine`. For example, a pure editing helper that needs Document + Selection + Register + RepeatState is a candidate; blindly moving half the methods to `normal_edit.rs` while they still mutate all Engine state is not.
   - Leave `motion::resolve()` alone unless an actual correctness/maintenance problem emerges. Its large `match` is an explicit motion table, not automatically a god function.
   - Preserve bounded Vim scope: do not turn this cleanup into full Vim compatibility.

   **Avoidables**
   - No trait-based state-machine framework.
   - No one module per key command.
   - No `EngineContext` with references to every substate.
   - No macros generating command tables merely to make the source shorter.
   - Do not change key grammar while moving state.

   **Regression traps**
   - mapping ambiguity timeout/fallback;
   - non-recursive mappings;
   - counts before operator and motion;
   - `0` as motion versus digit;
   - operator pending;
   - Visual mode anchor;
   - dot repeat;
   - insert repeat;
   - `f/F/t/T` and `;/,`;
   - command-line generations and stale completion rejection;
   - separate Ex/search histories;
   - Ctrl-R redo and Ctrl-D/U viewport commands.

   **Citations:** [Rust guide — application code should prefer simple, direct abstractions](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L180-L190), [Rust guide — avoid macros where ordinary Rust is clearer](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L900-L925), [`Engine` state](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-vim/src/engine.rs#L45-L145), [`Engine::handle_key()` and transient changed/message state](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-vim/src/engine.rs#L220-L360), [`engine_normal.rs`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-vim/src/engine_normal.rs#L1-L380), [`motion::resolve()`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-vim/src/motion.rs#L110-L300).

10. **Finish the unit/type boundary cleanup so character offsets, byte offsets, columns and rows cannot be accidentally interchanged.**

   **Deliverables**
   - Preserve the useful existing layout newtypes:
     - `LineCharOffset`;
     - `VisualColumn`;
     - `WrapRowIndex`;
     - `DisplayByteOffset`.
   - Add only the missing semantically distinct types that current code demonstrably mixes.
   - Add a row-local column type such as `RowColumn` for vertical caret preference. `preferred_column` is explicitly row-relative; it should not be a bare `usize` or masquerade as an absolute logical-line `VisualColumn`.
   - Add a wrap-width type such as `WrapWidthColumns`/`ColumnCount` for committed/requested wrap width.
   - Change `Editor::offset_at(line, column: usize)` to take an absolute visual-column type.
   - Change `row_span()` from `Range<usize>` to a type whose fields are visual columns.
   - Consider a document-wide visual row newtype where values cross function/module boundaries. Local `for row in first..last` counters can remain `usize`.
   - Fix core byte typing as part of the Document extraction:
     - `TextPoint.column_bytes` is a line-relative UTF-8 byte column and deserves a distinct type.
     - `AppliedEdit.old_byte_range/new_byte_range` should not expose an unlabelled `Range<usize>` if there is already a document-byte type.
   - Do not reuse a document-global `ByteOffset` for a line-relative byte column if their semantics differ; add `LineByteOffset`.
   - Conversion to raw `usize` should happen at the Ropey/tree-sitter/GPUI boundary where the external API requires it.
   - Keep index arithmetic local and explicit.
   - Update public docs to state coordinate spaces.

   **Avoidables**
   - Do not newtype every count in the program.
   - Generations, vector lengths, number of candidates, tab-stop counts, etc. do not need wrappers merely because they are integers.
   - Do not add broad `From<usize>` conversions everywhere if they make accidental mixing easy again.
   - Do not use type aliases where type confusion is precisely the problem.

   **Regression traps**
   - Vertical movement across wrapped rows.
   - Selection range geometry.
   - tabs measured from logical-line start.
   - Rope char ↔ byte conversion.
   - Tree-sitter incremental edit coordinates.
   - UTF-16 input-service conversion.
   - EOF and empty-document boundaries.

   **Citations:** [Rust guide — use newtypes when semantic mixing is easy](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L620-L660), [`Editor` currently stores several layout units as bare integers](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L83-L140), [`editor_cursor.rs` row-relative preferred-column logic](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor_cursor.rs#L45-L85), [`offset_at`/row span interfaces](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs#L880-L1015), [`TextPoint`/`AppliedEdit`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-core/src/document.rs#L75-L130).

11. **Run the final architecture/behavior gate and remove the transitional debris.**

   **Deliverables**
   - Run:
     - `cargo fmt --check`;
     - `cargo clippy --workspace --all-targets --locked -- -D warnings`;
     - `cargo test --workspace --locked`;
     - Windows release build;
     - Linux release build.
   - Run real Windows and Linux GUI smoke tests, not only `chaus --version`.
   - On both supported desktop platforms verify:
     - ordinary typing/editing;
     - IME/dead-key path where available;
     - pointer selection and drag autoscroll;
     - wrap word/character/none;
     - giant unwrapped file, including deep horizontal scrolling;
     - Save/Save As;
     - conflict dialog;
     - dirty-close dialog;
     - recovery dialog;
     - preference changes and persistence;
     - preference persistence failure;
     - Vim toggle, Normal/Insert/Visual/command-line basics;
     - theme/font changes.
   - Re-run the repository size/responsibility audit after the refactor.
   - **Do not judge completion by line count.** Instead verify these structural conditions:
     - `Editor` no longer contains dozens of unrelated primitive state fields.
     - each extracted subsystem's primitive state is private to that subsystem.
     - unrelated sibling modules cannot mutate it directly.
     - `chaus-io/src/lib.rs` is an actual façade rather than an implementation file.
     - `chaus-layout/src/lib.rs` is an actual façade after its decomposition.
     - no generic UI sizing remains in the layout crate.
     - no whole-line materialization exists in production no-wrap paint/hit-test paths.
     - one component owns settings disk ordering.
     - Preferences performs no blocking runtime filesystem reads.
     - Document remains the only live Rope/EOL mutation authority.
   - Remove temporary compatibility re-exports and stale field-access helpers added during the migration.
   - Remove dead code rather than leaving old and new architecture side by side.
   - Update crate/module docs with the final ownership map.
   - Update any old issue/docs text that still claims the pre-refactor ownership arrangement.
   - Keep comments that explain **why** a generation, epoch, checkpoint or ownership rule exists. Remove comments that merely restate obvious code.
   - Only after all tests/smokes pass should the implementer do the final `cargo fmt` and commit.

   **Avoidables**
   - Do not leave “temporary” duplicate paths for file saving, recovery, settings or layout.
   - Do not silence new Clippy warnings globally.
   - Do not add broad `#[allow]` merely because the refactor exposed poor APIs.
   - Do not claim Windows/Linux behavior from unit tests alone.
   - Do not continue refactoring coherent code just because some files remain long.

   **Regression traps**
   - The most dangerous final-pass failure is having both old and new owners active at once: two settings writers, two layout indexes, two save states, duplicated mutation callbacks, etc.
   - Search for old state field names before declaring the migration complete.
   - Search for direct `std::fs` calls in GPUI update/render code.
   - Search for whole-line Rope conversions in editor/layout hot paths.
   - Search for cross-module `pub(crate)` primitive fields that should now be private.

   **Citations:** [Rust guide — public documentation and meaningful comments](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L950-L1005), [Rust guide — readable implementation rather than clever compression](https://github.com/estyseesghosts/style-guides/blob/62dae7cc1305ef1703c41821130d0020e0ef3b22/rust-style-guide.md#L1280-L1430), [Chaus architectural/test invariants in `AGENTS.md`](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/AGENTS.md#L57-L95), [current editor root](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-editor/src/editor.rs), [current core document](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-core/src/document.rs), [current Vim engine](https://github.com/estyseesghosts/chaus/blob/8ba0c457b5650436a1a8fd08e945747f10f4b1cb/crates/chaus-vim/src/engine.rs).

The intended end state is not “smaller files.” It is: **one owner for each mutable lifecycle, narrow transitions between owners, typed coordinate boundaries, no duplicated mutation/file/settings paths, and `Editor` reduced to the place where those independently coherent pieces are deliberately coordinated.**