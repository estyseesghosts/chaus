# Limited Vim mode: deterministic modal input, not Vim emulation

## Product behavior

Standard editing is the default. A **runtime toggle**, `Ctrl+Shift+V` on Windows/Linux, switches the active document window's editing behavior between standard and Vim. Add the same checked action under `View → Vim Mode` and an editable setting `set vim`/`set novim` for the default state of new windows. The shortcut toggles the **active window** without touching other windows; it must not run when a modal native dialog, preferences field, search/replace textbox or menu is focused.

When enabled, initialize `NORMAL`. A small `NORMAL / INSERT / VISUAL / COMMAND` indicator is allowed; do not add a full status bar. When disabling Vim: cancel a pending operator/count/search/Ex command, exit Visual, clear transient register prefix state, preserve document selection only according to documented standard-editor policy (recommend collapse to caret), and ensure next keystroke types normally. Re-enabling always begins in Normal, never restores an old half-completed operator.

## Key dispatch: state machine, not action hotkeys

Proposed state:

```text
VimMode = Normal | Insert | VisualChar | VisualLine | CommandLine
Pending = None | Operator { op, count } | MotionCount | Prefix { keys }
VimState { mode, pending, count, register, last_find, last_search,
           last_change, visual_anchor, command_input, pending_timeout }
```

On each key:

1. Check focus: only dispatch if the document editor is the focused key context; a native menu/dialog or settings/search input takes precedence.
2. Check OS/app global commands (Vim toggle, Save, Close) with scoped bindings and collision tests. The normal-mode character grammar has precedence over ordinary text insertion while active.
3. If IME composition is in progress in Insert or command entry, defer to native composition. In Normal mode do not hand letter commands to text input.
4. Evaluate user remapping first with bounded prefix matching; then parse count/operator/motion/text object using a deterministic table/state machine; validate document positions.
5. Translate one completed command to **semantic editor intents** (`DeleteRange`, `ReplaceRange`, `MoveCaret`, `ApplySelection`, `Undo`), not direct Rope operations. One operator is one undo unit. Provide error/bell/status feedback for unknown sequences without swallowing a later valid key indefinitely.

Counts: `3j`, `2dw`, `d3w` and `2d3w` should have documented multiplication semantics (Vim-style repeat counts); cap enormous counts to prevent overflow/long UI stalls. `0` means line start when not inside a numeric count. `Esc` cancels prefixes/returns to Normal; timeout only for ambiguous *user mappings*, not for waiting for an operator motion.

## Precise v0.1 command contract

| Category | Commands | Notes |
| --- | --- | --- |
| Movement | `h j k l`, `w b e`, `0 ^ $`, `gg G`, `f{char}` and `;` (optional) | Follow grapheme boundaries; maintain preferred X for vertical movement; define word classes |
| Insert | `i I a A o O`, `Esc` | `o/O` insert new logical lines; autoindent; IME enabled |
| Operators | `d`, `c`, `y`, with motions; doubled `dd`, `cc`, `yy` | Operator-pending; changes via shared transaction API |
| Text objects | `iw aw`, `i" a"`, `i' a'`, `i( a(`, `i[ a[` | Quote/bracket pairs; predictable errors for unmatched delimiters |
| Visual | `v`, `V`, motion, `d/c/y`, `Esc` | Linewise inclusive behavior; blockwise Visual **excluded** |
| Paste | `p`, `P` | Distinguish charwise/linewise yank; internal unnamed register + system clipboard policy |
| Search | `/`, `?`, `n`, `N`, `*` optionally | Literal substring first; highlight match temporarily; cancel returns to old selection |
| Changes | `u`, `Ctrl+R`, `.`, `x`, `r{char}` optionally | `.` replays semantic last change, not raw key bytes; avoid history corruption |
| Ex subset | `:w`, `:q`, `:wq`, `:q!`, `:e {path}`, `:set {supported}` | No arbitrary Ex, `:!`, `:source`, register expressions or shell |

**Scope limits:** no macro recording, named registers, jump lists, marks, complex search regex, substitutions, splits/tabs, full Vim text-object language, Vimscript evaluation, `:global`, `:normal`, `:help` compatibility, or plugin ABI. If a key is unimplemented, show a concise nonblocking indicator rather than pretending success.

Linewise operations must define newline ownership: deleting last line, yanking a line without trailing newline, and pasting at EOF all need fixtures. `cw` and `ciw` are not merely aliases to `dw`; change enters Insert mode after the deletion and should undo as one change. `.` replays the complete command including inserted text after `Esc`.

## Mapping language in the rc file

**Do not implement full Vimscript.** Support a strict, testable subset:

```vim
set vim
let mapleader = " "
nnoremap <leader>w :w<CR>
nnoremap <leader>e :e<Space>
```

`<leader>`, `<CR>`, `<Esc>`, `<Space>`, `<C-x>` tokens are interpreted by our parser only if explicitly supported. Map targets to recognized **editor intents or Vim token sequences**; validate them before activation. For v0.1 limit to **non-recursive Normal-mode mappings**, with an explicit maximum sequence length and ambiguity timeout (e.g. configurable 500 ms for user mappings only). Reject cycles, duplicates and collisions with the mandatory toggle/OS shortcuts. For a mapping from `a` to `b`, resolve once; don't recursively expand into a second remapping. Surface diagnostics with filename and line.

Keep mapping registry separate from GPUI global keymap. Silvestris's command binding snapshots cannot safely be rewritten by changing a descriptor at runtime; dispatch Vim mapping chords within the focused editor state. When config reloads, replace the mapping table atomically, abort current pending prefixes, and keep the current editing mode unless the user explicitly toggles it.

The rc file and `:set` are **Scratchpad-specific**, despite Vim-like syntax. Do not claim that an arbitrary user's `.vimrc` will work. Use the app's own `~/.scratchpadrc` / Windows `_scratchpadrc` and document exactly which settings parse. See [silvestris.md](silvestris.md).

## Cursor semantics / insert handling

- Normal-mode caret may be drawn as a block cursor, but the **editing model** should stay a char-boundary caret. Handle empty lines and EOF explicitly, preventing off-by-one errors.
- Normal-mode `h/l/w/b` navigate grapheme-aware boundaries; Vim's historical byte/character edge cases may differ—document supported semantics rather than reverse engineering every Vim detail.
- Insert mode routes native text composition and default editing shortcuts to the same editor input pipeline as standard mode. `Esc` commits finished composition or cancels ongoing preedit according to platform's text-service behavior; never corrupt or double-insert it.
- Mouse click/drag in Vim mode updates the shared selection/caret; define whether it enters Visual or remains Normal (recommend remain Normal unless user explicitly invokes `v/V`).
- When a dialog gains focus, clear pending command prefix; returning focus must not execute a stale `d` with an unrelated key.

## Tests: sequences with expected results

Implement table-driven cases against an in-memory Document:

- From `one two three` at start: `dw` → `two three`; undo → original; `.` after an equivalent change repeats once.
- `ciw` selects word under cursor, deletes it, enters Insert; `hello<Esc>` makes one undo step; undo restores word, caret position policy tested.
- `2dd` removes two logical lines even with CRLF import; `yy` then `p/P` preserves linewise semantics at EOF.
- `d2w` and `2dw` match for simple words; `2d3w` is six word motions within validated limit.
- `di"` leaves delimiters, `da"` includes them; unmatched quote cancels harmlessly.
- `/hello<CR> n N` moves between matches, honoring wrap policy; cancelled search restores prior caret.
- `Ctrl+Shift+V` in Normal, Insert, Visual, `d` pending and `:` command input always lands in standard mode with no pending command. The next `a` types `a` normally.
- Toggle while focused in Settings/Find/Save dialog does nothing to their text; global command still available from appropriate parent menu.
- Rebind mapping, then reload invalid/recursive mapping; existing valid mappings continue; no crash/focus loss.
- Test auto-repeat, AltGr, dead keys, non-US layout, Ctrl+R conflicts, IME and clipboard interaction on real OS.

## Do not

- Never duplicate an edit implementation under Vim handlers; never emulate Vim by synthesizing a long series of mouse/keyboard events.
- Never globally intercept letters when another control has focus.
- Never pretend to run `~/.vimrc`, Vimscript functions, plugins or arbitrary Ex commands.
- Never silently route unknown Vim commands to text insertion in Normal mode.
- Never make Vim mode default on first launch or hide the toggle from GUI-only users.

## References

[Vim user reference](https://vimhelp.org/usr_toc.txt.html), [Vim mapping reference](https://vimhelp.org/map.txt.html), [Vim text objects](https://vimhelp.org/motion.txt.html), [Silvestris commands](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/docs/commands.md), [pinned GPUI bindings](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/src/keymap/binding.rs).
