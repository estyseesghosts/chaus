Use `master @ b6194a5` as the audit baseline. I would split the work like this; the order matters.

1. **[P0][Vim/Ex] Support `~` home-directory paths in Ex file commands**

   **Source:** [`ex_command.rs:L360-L432`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/ex_command.rs#L360-L432) explicitly rejects an unquoted leading `~`. [`editor_completion.rs:L115-L166`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor_completion.rs#L115-L166) passes the typed directory directly to `read_dir`, so completion cannot understand `~/...` either.

   **Deliverables**
   - Accept unquoted `~/path` on Linux/macOS and `~/path` plus `~\path` on Windows.
   - Support it consistently for `:e`, `:w`, `:saveas`, and `:r`.
   - Resolve `~` through one shared path-resolution helper outside the pure `chaus-vim` parser; do not make `chaus-vim` query the filesystem/environment.
   - Make Tab completion resolve the expanded directory while keeping the visible command line spelled as `~/...`.
   - Keep quoted `"~/foo"` literal according to the existing quoted-path contract.
   - Add parser, completion, and real temporary-file integration tests.

   **Avoid**
   - Do not turn this into shell expansion.
   - Do not add `$VAR`, `%USERPROFILE%`, `~user`, command substitution, `%`, `#`, `*`, or `?`.
   - Do not change relative-path resolution away from Chaus's existing working-directory semantics.
   - Do not duplicate expansion independently in read/write/edit/completion call sites.

   **Regression traps**
   - Paths containing spaces and Unicode.
   - Windows drive paths and UNC paths must remain untouched.
   - `"~"` must remain a literal filename when quoted.
   - `~/existing-file` must retain existing overwrite/refusal behavior for `:w` versus `:w!`.
   - Completion must not accidentally replace `~/` with the absolute home path in the command line.

2. **[P0][Vim/Ex] Add `:o {path}` as an explicit Chaus alias for `:edit {path}`**

   **Source:** the typed vocabulary currently contains `Edit` but no open alias in [`ex_command.rs:L22-L70`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/ex_command.rs#L22-L70) and [`ex_command.rs:L132-L279`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/ex_command.rs#L132-L279). Path completion currently recognizes only Write/SaveAs/Edit/Read in [`ex_command.rs:L405-L432`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/ex_command.rs#L405-L432).

   **Deliverables**
   - Accept `:o path` as a convenience alias for Chaus's existing `:e path`.
   - Accept `:o! path` with exactly the same dirty-document semantics as `:e! path`.
   - Route both to the existing `ExCommand::Edit`; do not create a parallel open lifecycle.
   - Enable existing path completion for `:o`.
   - Document explicitly that this is a Chaus convenience alias, not an attempt to reproduce historical Vim `:open` semantics.

   **Avoid**
   - No second open implementation.
   - No different dirty-buffer rules between `:o` and `:e`.
   - Do not overload `:o` for GUI file-picker behavior.

   **Regression traps**
   - Dirty document + `:o path` must refuse exactly like `:e path`.
   - `:o! path` must not allow an old async load to overwrite edits made after the load starts.
   - `:o ~/foo` must work once issue 1 lands.
   - Tab completion must behave identically for `:o` and `:e`.

3. **[P0][Vim] Replace duplicated character-based motion handling with one composable motion model**

   **Source:** Normal motion is currently split between top-level dispatch, `Pending`, `move_caret()`, and `motion_range()` in [`engine_normal.rs:L42-L395`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L42-L395). Visual mode independently reimplements most of the same motions in [`engine_visual.rs:L14-L102`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_visual.rs#L14-L102). `Pending` is currently command-specific rather than a general motion parse state in [`engine.rs:L36-L84`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine.rs#L36-L84).

   **Deliverables**
   - Introduce a semantic motion representation rather than passing raw `char` motion keys around.
   - Represent at minimum the existing motions: horizontal/vertical, word motions, line start/nonblank/end, first/last line, and find-character.
   - Have the resolver return enough information for both movement and operators: target/range, characterwise versus linewise, inclusive versus exclusive behavior, and count.
   - Make Normal movement, operator-pending movement, and Visual movement consume the same resolver.
   - Preserve existing grapheme-aware behavior.
   - Preserve `2d3w` count multiplication and bounded count handling from [`engine.rs:L327-L345`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine.rs#L327-L345).
   - This ticket should primarily restructure existing behavior; later tickets add vocabulary.

   **Avoid**
   - Do not introduce a second document-position model inside `chaus-vim`.
   - Do not synthesize keypresses to make operators use Normal movement.
   - Do not copy another motion switch into Visual mode.
   - Do not change undo grouping or register behavior while restructuring motions.

   **Regression traps**
   - `h/j/k/l/w/b/e/0/^/$/gg/G` with and without counts.
   - `dw`, `d2w`, `2dw`, `2d3w`, `d$`, `dj`, `yG`.
   - Visual movement must preserve anchor/head direction.
   - Empty documents, empty lines, EOF, CRLF-imported documents, Unicode graphemes, wrapped display lines.
   - Soft wrapping must never change logical Vim line semantics.

4. **[P0][Vim] Make multi-key motions composable with operators**

   **Depends on:** issue 3.

   **Source:** `gg` is handled only by top-level `Pending::G`, while operator-pending dispatch asks `motion_range()` for a single `char`, so `dgg/ygg/cgg` cannot exist cleanly today. See [`engine_normal.rs:L154-L246`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L154-L246) and [`engine_normal.rs:L290-L395`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L290-L395).

   **Deliverables**
   - Allow operator-pending state to consume a multi-key motion.
   - Implement `dgg`, `ygg`, `cgg`.
   - Support counts consistently: `d2gg`, `2dgg`, `y10G`, etc., with clearly tested semantics.
   - Make incomplete sequences cancellable with Escape.
   - Unknown continuations must fail cleanly and clear pending state instead of poisoning the next command.
   - Add table-driven tests covering Normal movement and all three operators against the same parsed motion.

   **Avoid**
   - No bespoke `dgg` special case.
   - Do not route `gg` through a fake `G` command.
   - Do not let user-mapping timeout rules apply to built-in operator/motion prefixes.

   **Regression traps**
   - Plain `gg` and counted `5gg` must remain correct.
   - `d` followed by an invalid `g...` sequence must not delete anything.
   - `cgg` must enter Insert and remain one undoable user operation.
   - `ygg` must populate the correct linewise register without mutating the document.

5. **[P1][Vim] Complete the common word, delimiter, and character-find motion set**

   **Depends on:** issue 3.

   **Source:** Chaus currently implements lowercase `w/b/e` in [`motion.rs:L38-L105`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/motion.rs#L38-L105). Find support is only forward `f` plus same-direction `;`, and `last_find` stores only a character in [`engine_normal.rs:L652-L700`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L652-L700) and [`engine.rs:L423-L428`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine.rs#L423-L428).

   **Deliverables**
   - Add `W`, `B`, `E`, and `ge`.
   - Define WORD as a non-whitespace run, distinct from the current word character classes.
   - Add `F`, `t`, `T`, and `,`.
   - Replace `last_find: Option<char>` with state containing character, direction, and find/till kind.
   - Make `;` repeat the same find and `,` repeat it in the opposite direction.
   - Allow all applicable motions after `d/c/y`: e.g. `dW`, `cge`, `df)`, `dt,`, `yF:`.
   - Add `%` for matching `()`, `[]`, `{}` using a bounded delimiter matcher.
   - Counts must work for all applicable motions.

   **Avoid**
   - Do not make `%` a full syntax-aware language parser.
   - Do not scan indefinitely on malformed gigantic inputs; keep delimiter search bounded/sane.
   - Do not implement WORD by changing existing lowercase `w/b/e` semantics.

   **Regression traps**
   - Punctuation-heavy text: `foo.bar/baz`.
   - Whitespace-only lines.
   - Repeated delimiters and escaped-looking punctuation.
   - `f/F/t/T` failure must leave the caret unchanged.
   - `;` and `,` after different find directions.
   - Operator inclusivity: `dfx` and `dtx` must differ correctly.
   - `%` must not jump to an unrelated delimiter after an unmatched pair.

6. **[P1][Vim] Give Visual mode motion and text-object parity with Normal/operator mode**

   **Depends on:** issues 3 and 5.

   **Source:** Visual mode currently has its own limited motion switch and accepts only `d/x`, `c`, `y`, `:`, `o`, and a small motion set; everything else reports “not implemented” in [`engine_visual.rs:L14-L102`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_visual.rs#L14-L102). Text objects currently exist only after an operator in [`engine_normal.rs:L213-L244`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L213-L244).

   **Deliverables**
   - Route Visual movement through the shared motion resolver.
   - Support `viw/vaw`, quote objects, and bracket objects already supported by operator-pending mode.
   - Support the new `W/B/E/ge`, find motions, and `%` in Visual mode where meaningful.
   - Preserve `o` anchor/head swapping.
   - Keep Visual-line (`V`) behavior explicitly linewise.
   - Add tests for forward and reversed selections.

   **Avoid**
   - Do not create separate Visual-only text-object implementations.
   - Do not silently convert Visual-line selections to characterwise ranges.
   - No Visual-block mode in this scope.

   **Regression traps**
   - `viw`, `vaw`, `vi"`, `va(` at beginning/end of document.
   - Unmatched quote/bracket leaves selection intact.
   - `o` after several motions still swaps the same two ends.
   - `v%` across multiple lines.
   - `vd`, `vc`, `vy` continue to use the correct register and undo behavior.

7. **[P1][Vim] Add common Normal-mode edit aliases and correct `cw` semantics**

   **Depends on:** issue 3.

   **Source:** Normal mode currently directly implements `x`, `r`, `p/P`, operators and insert commands in [`engine_normal.rs:L42-L153`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L42-L153). Change operations are grouped into one user operation in [`engine_normal.rs:L395-L501`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L395-L501).

   **Deliverables**
   - Implement the common aliases:
     - `D` → line-end delete semantics.
     - `C` → line-end change semantics.
     - `S` → linewise change semantics.
     - `s` → substitute character(s), then Insert.
     - `X` → delete backward.
     - `Y` → linewise yank.
   - Correct `cw` so it behaves like Vim's common “change to end of current word” case rather than blindly reusing the ordinary `w` operator range.
   - Extend the same correction to `cW`.
   - All aliases must record semantic last-change information so `.` repeats them correctly.
   - Preserve counts.

   **Avoid**
   - Do not implement aliases by recursively feeding their equivalent key sequence through user mappings.
   - Do not create extra undo units for the deletion and subsequent Insert portion of `C/S/s/cw`.
   - Do not change plain `dw` merely to make `cw` work.

   **Regression traps**
   - `cw` versus `dw` in the middle of a word.
   - `cw` on whitespace.
   - `3s`, `3X`, `2D`, `2S`.
   - Empty line and end-of-line behavior.
   - `.` after each command.
   - Undo after a change command must restore the entire pre-change state in one step.

8. **[P1][Vim] Implement `>>`, `<<`, Visual `>`/`<` using Chaus's existing indentation engine**

   **Depends on:** issue 3.

   **Source:** Chaus already has selection-aware indentation and outdent logic in [`editor_indent.rs:L15-L69`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor_indent.rs#L15-L69). Vim currently has no `>`/`<` operator in [`operator.rs:L11-L85`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/operator.rs#L11-L85).

   **Deliverables**
   - Add `>>` and `<<` for current-line indentation/outdent.
   - Support counts such as `3>>`.
   - Support Visual `>` and `<`.
   - Prefer an editor-level semantic intent that invokes the existing indentation implementation rather than moving indentation policy into `chaus-vim`.
   - Preserve `expandtab` and `tabstop` behavior already owned by the editor.
   - Define and test post-command caret/selection placement.
   - Make `.` repeat Normal-mode indentation changes where practical.

   **Avoid**
   - No duplicate tab/space conversion code in `chaus-vim`.
   - Do not make these operate on soft-wrapped display rows.
   - Do not rewrite unaffected indentation elsewhere in the document.

   **Regression traps**
   - Mixed tabs/spaces.
   - Reversed Visual selections.
   - Selection ending exactly at the start of the following line, which `selected_lines()` already treats specially.
   - First/last line and empty lines.
   - Undo must revert the whole multi-line indent in one transaction.

9. **[P1][Vim] Add line joining with `J` and `gJ`**

   **Source:** there is currently no join operation in Normal dispatch [`engine_normal.rs:L61-L153`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L61-L153), although Chaus already has a transaction-based document editing model used throughout the same file.

   **Deliverables**
   - Implement `J` to join the current line with the following logical line using documented Vim-like whitespace normalization.
   - Implement `gJ` to join without inserting/removing joining whitespace beyond the newline itself.
   - Support counts.
   - Perform the full join as one document transaction/undo unit.
   - Record it for `.` replay.
   - Define caret placement after the join.

   **Avoid**
   - Do not operate on visual wrap rows.
   - Do not synthesize Delete/Space key events.
   - Do not silently alter unrelated leading/trailing whitespace.

   **Regression traps**
   - Empty current/next line.
   - Next line beginning with spaces or tabs.
   - Current line ending with spaces.
   - Final document line: no-op with deterministic feedback.
   - CRLF source documents must retain Chaus's internal/import/export line-ending contract.
   - Counted joins must not walk beyond EOF.

10. **[P1][Vim] Implement `*` and `#` using the existing Vim search state**

   **Source:** `/`, `?`, `n`, and `N` already use `last_search` and `find_literal()` in [`engine_normal.rs:L683-L700`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/engine_normal.rs#L683-L700) and [`motion.rs:L271-L299`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/motion.rs#L271-L299).

   **Deliverables**
   - `*`: derive the word under/adjacent to the Normal caret, store it as the current forward search, and jump to the next occurrence.
   - `#`: same in the reverse direction.
   - Subsequent `n/N` must use that same stored search.
   - Reuse the established Chaus word classification unless a deliberately separate “search word” helper is justified.
   - Preserve existing literal-search behavior; no regex requirement.
   - Add Unicode and punctuation fixtures.

   **Avoid**
   - Do not secretly introduce regex or Vim `\<...\>` syntax.
   - Do not route through the GUI Find bar.
   - Do not mutate the document or selection into a range merely to identify the word.

   **Regression traps**
   - Caret on punctuation or whitespace.
   - Only one occurrence in the document.
   - Search wraps around document boundaries consistently with `/` and `?`.
   - Unicode words.
   - `n/N` direction after `#`.

11. **[P1][Vim] Add viewport commands `zz`, `zt`, `zb`, Ctrl-D and Ctrl-U without contaminating the pure Vim engine with UI geometry**

   **Source:** Chaus already owns viewport position in [`editor.rs:L75-L76`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor.rs#L75-L76) and clamps it in [`editor.rs:L751-L759`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor.rs#L751-L759). Caret-follow logic lives in [`editor_cursor.rs:L173-L199`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor_cursor.rs#L173-L199). Modified keys are currently rejected from Vim dispatch except Ctrl-R in [`editor_vim.rs:L31-L70`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor_vim.rs#L31-L70).

   **Deliverables**
   - Add semantic Vim outcomes/intents for viewport operations rather than exposing `scroll_row` to `chaus-vim`.
   - `zz`: caret row centered where possible.
   - `zt`: caret row at viewport top.
   - `zb`: caret row at viewport bottom.
   - Ctrl-D / Ctrl-U: half-page movement plus corresponding viewport movement.
   - Modify `editor_vim.rs` modifier dispatch narrowly so these built-in Ctrl commands reach Vim while unrelated application shortcuts remain untouched.
   - Use visual rows for viewport placement but logical document movement semantics where appropriate.

   **Avoid**
   - Do not make `chaus-vim` depend on GPUI, pixel geometry, or `Editor`.
   - Do not globally admit arbitrary Ctrl-letter combinations into the Vim engine.
   - Do not steal Ctrl-O; Chaus already documents Ctrl-O as **Open File** in [`README.md:L43-L56`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/README.md#L43-L56).

   **Regression traps**
   - Small files shorter than one viewport.
   - Soft-wrapped very long lines.
   - Top/bottom document boundaries.
   - Resized window after scrolling.
   - Ctrl-R redo remains functional.
   - Ctrl-S/Ctrl-O/Ctrl-F and other Chaus application shortcuts must retain priority.

12. **[P2][Vim/Ex] Add `ZZ`, `ZQ`, and `:wqa` as thin lifecycle compositions**

   **Source:** Chaus already implements `:wq`, `:x`, `:wa`, and `:qa[!]` in the typed Ex layer [`ex_command.rs:L22-L70`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-vim/src/ex_command.rs#L22-L70), with actual multiwindow behavior routed through the editor in [`editor_ex.rs`](https://github.com/estyseesghosts/chaus/blob/b6194a541b135e170d236032d309161ef822b1d0/crates/chaus-editor/src/editor_ex.rs).

   **Deliverables**
   - `ZZ`: equivalent lifecycle intent to save-if-needed then close the current window.
   - `ZQ`: discard and close current window, subject to the same in-flight-write safety rules as `:q!`.
   - `:wqa`: save all modified named documents and close all windows **only after all required saves have actually succeeded**.
   - Reuse the existing save/write lifecycle; introduce orchestration state if necessary rather than treating `:wa` + immediate `:qa` as sufficient.
   - Any failure, conflict, cancellation, untitled dirty document, stale save revision, or busy writer must leave the relevant windows open and produce useful status.
   - Add multiwindow asynchronous tests.

   **Avoid**
   - Do not implement `:wqa` by literally dispatching `:wa` followed immediately by `:qa`.
   - Do not interpret “save queued” as “save succeeded.”
   - Do not bypass conflict detection, recovery bookkeeping, durability checks, or in-flight-write guards.
   - No general Ex command chaining in this ticket.

   **Regression traps**
   - One of several windows fails to save.
   - One file changes externally during the operation.
   - Untitled dirty window.
   - Edit occurs while its save is pending.
   - Two windows target the same file.
   - Clean windows should not be unnecessarily written.
   - `ZZ` on a clean document should close without writing.

### Dependency shape

Issues **1 and 2** can land independently. **3 is the foundation** for the next Vim grammar pass; do it before **4–8** rather than piling more special cases into `engine_normal.rs` and `engine_visual.rs`. Issues **9 and 10** are mostly independent once the shared motion/state work settles. **11** intentionally crosses the `chaus-vim` → `chaus-editor` boundary and should stay UI-specific. **12** should remain last because it touches asynchronous file/window lifecycle rather than the modal grammar itself.

I would leave named registers, macros, marks/jump lists, Vimscript, shell commands, splits/buffers, Visual Block, arbitrary Ex ranges, and `:%s` out of this pass. The set above is enough to close most of the “I instinctively typed a normal Vim command and Chaus didn't understand it” gap without turning Chaus into a Vim emulator.