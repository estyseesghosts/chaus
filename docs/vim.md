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
| Movement | `h j k l`, `w W b B e E ge`, `0 ^ $`, `gg G`, `f F t T` and `; ,`, `%` | Follow grapheme boundaries; maintain preferred X for vertical movement; define word classes. `%` scans forward to the nearest bracket and then matches it, bounded in depth and length. |
| Insert | `i I a A o O`, `Esc` | `o/O` insert new logical lines; autoindent; IME enabled |
| Operators | `d`, `c`, `y`, with motions; doubled `dd`, `cc`, `yy` | Operator-pending; changes via shared transaction API. `dgg`, `cgg`, `ygg` and counted `2dgg`/`d2gg` share the motion resolver. |
| Text objects | `iw aw`, `iW aW`, `i" a"`, `i' a'`, `i( a(`, `i[ a[`, `i{ a{` | Quote/bracket pairs; predictable errors for unmatched delimiters |
| Visual | `v`, `V`, motion, `d/c/y`, `x`, `o`, `>`, `<`, `%`, `Esc` | The same motions as Normal mode; linewise behavior is preserved for `V`; blockwise Visual **excluded** |
| Paste | `p`, `P` | Distinguish charwise/linewise yank; internal unnamed register + system clipboard policy |
| Search | `/`, `?`, `n`, `N`, `*`, `#` | Literal substring; `*`/`#` derive the word under or next to the caret and store it for `n`/`N`; no regex |
| Changes | `u`, `Ctrl+R`, `.`, `x`, `X`, `s`, `S`, `D`, `C`, `Y`, `r{char}`, `J`, `gJ`, `>>`, `<<` | `.` replays semantic last change, not raw key bytes; avoid history corruption |
| Viewport | `zz`, `zt`, `zb`, `Ctrl+D`, `Ctrl+U` | The engine returns semantic intents; the editor maps them to its visual rows. Counts scroll extra half pages. |
| Exit | `ZZ`, `ZQ` | `ZZ` saves if needed then closes; `ZQ` discards and closes, refusing while a write is in flight. |
| Ex subset | See [Command line and Ex reference](#command-line-and-ex-reference) | No arbitrary Ex, `:!`, `:source`, register expressions or shell |

**Scope limits:** no macro recording, named registers, jump lists, marks, complex search regex, substitutions, splits/tabs, full Vim text-object language, Vimscript evaluation, `:global`, `:normal`, `:help` compatibility, or plugin ABI. If a key is unimplemented, show a concise nonblocking indicator rather than pretending success.

Linewise operations must define newline ownership: deleting last line, yanking a line without trailing newline, and pasting at EOF all need fixtures. `cw` and `ciw` are not merely aliases to `dw`; change enters Insert mode after the deletion and should undo as one change. `.` replays the complete command including inserted text after `Esc`.

`J` follows Vim's count rule: it joins `[count]` hard lines with a minimum of two, so plain `J` and `2J` both join two lines and `3J` joins three. It inserts one space when both sides are non-empty, the first line does not already end in whitespace, and the next line does not start with `)`; it removes the next line's leading indent, and leaves the caret at the join point. `gJ` removes only the line break. Counts stop at the last line. A join is one transaction and one undo step, and `.` repeats the count.

`>>` and `<<` indent or outdent whole hard lines through the editor's existing indentation policy (tab stop, `expandtab`), not spaces guessed by the modal layer. `3>>` covers three lines; Visual `>`/`<` covers the highlighted range and returns to Normal. The engine emits one `Indent` outcome for the range, so it is a single undo step and `.` repeats it.

## Command line and Ex reference

`:`, `/` and `?` open a temporary input line over the bottom-left corner of the document; it replaces the mode badge while open and disappears on Enter or Escape. The text lives in the Vim engine (`crates/chaus-vim/src/command_line.rs`), never in the document, its undo history or its recovery snapshot.

Input line editing: Left/Right move by extended grapheme, Home/End, Backspace (on an empty line it closes the line), Delete, Ctrl+V paste (stops at the first line break), Up/Down history, Tab path completion in `:` lines. Text from the platform text service (IME commit, dead keys, AltGr) goes into the line; an IME composition is shown at the caret until committed. The line is capped at 1024 characters; input past that is refused with a message, not dropped silently. Toggling Vim, opening a dialog or the find bar, or an application shortcut closes an unfinished line without running it. The IME candidate window is still positioned at the document caret, not on the input line.

History is per window and in memory only: up to 100 `:` lines and, separately, 100 `/`/`?` patterns. Empty lines and immediate repeats are not recorded; lines that failed to parse are, so they can be corrected. Up past the newest entry restores the unsent draft. `n`/`N` keep using the last search regardless of history browsing.

Every `:` line is parsed into a typed `ExCommand` (`crates/chaus-vim/src/ex_command.rs`) before anything runs; a parse error is shown and nothing else happens.

| Command | Behavior |
| --- | --- |
| `:w` / `:w!` | Save the current file through the normal checked save. `!` does not skip the external-change check; the conflict sheet is the override. Untitled: opens Save As. |
| `:w {path}` | Named document: write a **copy** to `path`. The window keeps its file, stays dirty if it was, and keeps its recovery snapshot. Untitled document: write to `path` and make it the document's file once the write succeeds. Refuses an existing `path`. A path naming the current file is a normal save. |
| `:w! {path}` | As above, but may replace an existing `path`. |
| `:sav[eas][!] {path}` | Write to `path` and make it the document's file once the write succeeds. Refuses an existing `path` without `!`. On failure the old file and dirty state are kept. |
| `:q` | Close this window if clean. Dirty: "No write since last change" and the window stays open; the GUI close sheet is not shown. File → Close keeps its sheet. |
| `:q!` | Discard edits and close this window. Refused while a write is in flight. |
| `:wq` | Save, then close only if the saved revision is still current. Untitled opens Save As. Cancel, conflict or failure keeps the window open with a message. |
| `:x[it]` | Close a clean window without writing; otherwise as `:wq`. |
| `:e[dit] {path}` | Open `path` in this window. Refused when the document is dirty. |
| `:e[dit]! {path}` / `:e!` | Replace the document even if dirty; `:e!` alone reloads the current file. Edits typed while the load runs cancel it. |
| `:o[pen][!] {path}` | Chaus alias for `:e[dit]` (not Vim's historical `:open`). Same dirty check, `!` force and load-cancellation behavior. |
| `:ene[w][!]` | Replace this window's document with an empty untitled one; dirty needs `!`. Settings, theme and Vim mode stay. |
| `:f[ile]` | Show the file name (`[No Name]` when untitled), `[Modified]`, and the line count. |
| `:pw[d]` | Show the process working directory that relative paths resolve against. |
| `:noh[lsearch]` | Vim search paints no match decoration in chaus, so this reports that nothing is shown. The last pattern stays for `n`/`N`. |
| `:{N}` / `:$` | Go to hard line N (soft wrap is ignored) at its first non-blank. `:0` is line 1; past the end is the last line. |
| `:se[t] {args}` | `wrap`, `nowrap`, `wrap=word`/`char`/`none`, `expandtab`/`et`, `noexpandtab`/`noet`, `tabstop=N`/`ts=N` (1–16), and `wrap?`, `expandtab?`, `tabstop?` queries. Applies to this window only and is never written to the rc file. |
| `:format`, `:trim`, `:retab` | Format with the configured external formatter, trim trailing whitespace, convert indentation to spaces. |
| `:wa[ll]` | Start a checked save of every modified named document in every window. Untitled windows are reported as not written; no picker opens. |
| `:qa[ll]` | Close every window, only if every window is clean and idle; otherwise nothing closes. |
| `:qa[ll]!` | Discard every window's edits and close them all. Refused, closing nothing, while any window has a write in flight. |
| `:wqa[ll]` | Start a checked save for every modified named document and close all windows only after every started save has actually succeeded. Dirty untitled windows, conflicts, in-flight writers, duplicate save targets and edits made while a save is pending leave all windows open with a status message. Unlike `:wa` followed by `:qa`, it does not treat a queued save as success. |
| `:r[ead] {path}` | Insert the file's text as whole lines below the current hard line, as one undo step. The file association is unchanged. A BOM is not inserted; one trailing newline of the read file is dropped so no blank line appears. If the document changes before the read finishes, nothing is inserted. |

Command names accept the abbreviations shown in brackets. A name must be followed by a space before its argument, so `:wq1` is an unknown command rather than `:w q1`.

Path arguments:

- `"..."` is taken literally, backslashes included. A path cannot contain `"`. A quoted leading `~` is a file named `~`, not the home directory.
- Unquoted, a backslash followed by a space is an escaped space; every other backslash is literal, so `C:\Users\me\a.txt` and `\\server\share\a.txt` work as typed.
- Unquoted, a leading `~` component (`~`, `~/...`, and `~\...` on Windows) expands to the current user's home directory. `~user` is rejected. The parser records the spelling; the editor resolves it through `chaus_io::expand_home` only when the file operation runs.
- Unquoted `*`, `?`, and a leading `%` or `#` are rejected because Vim would expand them; quote the path to use them literally.
- Relative paths resolve against the process working directory (`:pwd`), not the current file's directory.

Tab completes the path argument of `:e`, `:o`, `:w`, `:saveas` and `:r` when the caret is at the end of the line. A leading `~` is expanded for the directory scan only; the visible line keeps `~/...`, so the completed command still means the same thing when it runs. A quoted `~` is scanned as the literal relative directory. The directory is listed off the UI thread, examining at most 4096 entries and keeping 256 matches. One match is inserted (a directory gets a trailing separator); several matches insert their longest common prefix and list up to eight in the status. Hidden entries appear only when the typed name starts with `.`. Names that are not valid UTF-8 are skipped. Matching is case-insensitive on Windows. A result that arrives after the line changed is dropped. Completion never opens, creates or writes a file.

Known limits: the existing-file check for `:w {path}` runs immediately before the rename, not atomically with it. A path that is the current file under a different spelling (symlink, 8.3 name) is treated as another file and therefore refused without `!`. Native Windows/Linux GUI runs of this workflow are not automated yet (#63); `crates/chaus-editor/tests/ex_commands.rs`, `editor_home_paths.rs`, `editor_exit.rs` and `editor_wqa_all.rs` drive the editor through GPUI's test platform with real temporary files.

## Mapping language in the rc file

**Do not implement full Vimscript.** Support a strict, testable subset:

```vim
set vim
let mapleader = " "
nnoremap <leader>w :w<CR>
nnoremap <leader>e :e<Space>
```

`<leader>`, `<CR>`, `<Esc>`, `<Space>`, `<C-x>` tokens are interpreted by our parser only if explicitly supported. Map targets to recognized **editor intents or Vim token sequences**; validate them before activation. For v0.1 limit to **non-recursive Normal-mode mappings**, with an explicit maximum sequence length and a 500 ms ambiguity timeout for user mappings only. Reject cycles, duplicates and collisions with the mandatory toggle/OS shortcuts. For a mapping from `a` to `b`, resolve once; don't recursively expand into a second remapping. Surface diagnostics with filename and line.

Keep mapping registry separate from GPUI global keymap. Silvestris's command binding snapshots cannot safely be rewritten by changing a descriptor at runtime; dispatch Vim mapping chords within the focused editor state. When config reloads, replace the mapping table atomically, abort current pending prefixes, and keep the current editing mode unless the user explicitly toggles it.

The rc file and `:set` are **chaus-specific**, despite Vim-like syntax. Do not claim that an arbitrary user's `.vimrc` will work. Use the app's own `~/.chausrc` / Windows `_chausrc` and document exactly which settings parse. See [silvestris.md](silvestris.md).

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
