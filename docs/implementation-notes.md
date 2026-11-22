# Implementation notes and verified baseline

Recorded 2026-11-22 while building the first chaus increment.

## Pins

| Component | Revision / version |
| --- | --- |
| Silvestris | `d1db8f0c6cbe80e80fb1dfebbb5c78990d2b9265` on `estyseesghosts/silvestris`, branch `source-list-shell` (git dependency) |
| GPUI | `6b720973027999bae402df23d469026f4cced988` (same pin as Silvestris `config/pins.toml`) |
| Rust | `1.98.1` (`rust-toolchain.toml`); this machine builds with the GNU host `x86_64-pc-windows-gnu`, CI uses the default MSVC host |
| Ropey | `1.6` |
| Tree-sitter | not linked yet |
| `windows-sys` | `0.61` |

Silvestris is consumed from its pinned upstream revision, so a fresh clone
builds without a sibling checkout. Bump that revision in `Cargo.toml`,
`Cargo.lock` and the table above together.

## Windows build workaround

`gpui_platform` unconditionally enables GPUI's `windows-manifest` feature. On this
machine GPUI's build script passes a backslash path to devkitPro's MinGW
`windres` 2.47, which concatenates the include directory and the file name into
`resources/windowsgpui.rc` and fails. The same command succeeds with forward
slashes, so the failure is in path handling, not in the resource file.

chaus therefore depends on `gpui_windows` directly with `default-features = false`
and constructs `WindowsPlatform` in `crates/chaus-app/src/platform.rs`. Linux uses
`gpui_linux` directly. Revisit if the pinned GPUI or the toolchain changes; this
is a build-environment workaround, not a claim that the manifest feature is
broken everywhere.

The Windows GNU target initially linked the MinGW runtime dynamically. Those
DLLs are not present on a clean machine; `.cargo/config.toml` sets
`-C target-feature=+crt-static` for `x86_64-pc-windows-gnu`, and the executable
now imports only system libraries.

`0xC0000139` (`ENTRYPOINT_NOT_FOUND`) was not a CRT problem. chaus statically
imports `TaskDialogIndirect`, which exists only in Common-Controls 6, so the
process needs an application manifest declaring the
`Microsoft.Windows.Common-Controls` 6.0.0.0 dependency. Disabling
`gpui_platform` also disabled GPUI's `windows-manifest` feature, and the binary
carried no `RT_MANIFEST` resource, so the loader bound Common-Controls 5.82 and
the process aborted before `main` — even for `chaus --help`. `chaus-app` now
embeds `resources/windows/chaus.manifest.xml` from `build.rs`, passing
forward-slash paths to avoid the same devkitPro `windres` bug. Both `--help` and
`--version` print and exit 0, and the manifest resource is present in the PE.
MSVC has no `windres`, so there `build.rs` passes `/MANIFEST:EMBED` and
`/MANIFESTINPUT:` to `link.exe` and uses the same XML.

## Licensing

Silvestris and GPUI are consumed under their upstream licenses (Apache-2.0 for
GPUI; see the Silvestris repository for its declared license). chaus source is
app-owned. Silvestris fonts are not duplicated. Geist Sans is bundled separately
because Silvestris ships Geist Mono but not Geist Sans: see
`crates/chaus-app/assets/fonts/geist-sans/README.md` for the source commit,
OFL-1.1 text and SHA-256 of each face. It is registered with GPUI and selected
through Silvestris's public per-theme `FontOverrides` API, so Aqua typography
and every measured size stay unchanged.

## Soft wrapping

`WrapMode` has three values: `Word` (default), `Character`, and `None`.
`set wrap=word`, `set wrap=char`, `set wrap=none`, the legacy `set wrap` /
`set nowrap`, `:set wrap=...`, and View → Wrap all drive the same setting.
`chaus-layout` implements UAX #14 word-preferred wrapping through
`unicode-linebreak`: each row takes the last permitted break that fits; when no
break fits (a long URL, path or token) it falls back to the furthest fitting
grapheme. Byte offsets from the line-break crate are converted to char offsets
and snapped to grapheme boundaries, so combining sequences, CJK and ZWJ emoji
are never split. Trailing whitespace stays on the final row rather than making a
blank one. Tabs keep their stops anchored to the logical line, continuation
rows start at the normal left inset, and the source text and char offsets are
untouched. A single `Rows` walker backs `row_count`, `segments` and the
editor's painting, caret and hit-testing, so the caches cannot disagree.
Changing the mode reflows the window without touching the document or undo
history; `None` restores horizontal scrolling.

## Known gaps in this increment

- The titlebar carries File/Edit/View menus built from Silvestris's
  `CommandRegistry` and `PopupMenu`. Each window owns its registry, so the
  checked Vim Mode/Wrap items and disabled Undo/Redo/Cut/Copy come from that
  window's state; the document canvas has no toolbar or tabs. `Ctrl+Shift+V`
  toggles Vim in both modes and is inert while a sheet, find bar or menu owns
  the keyboard. `Ctrl+,` opens a Preferences window for theme, font family,
  font size, line height, wrap mode, tab stop, spaces-vs-tabs and the Vim
  default applied to new windows. Applying a value updates every open editor
  in place through `Editor::set_settings` (document, selection, undo, focus and
  file state are preserved), writes the same validated `Settings` value to the
  rc file, and keeps comments, mappings and unsupported lines intact. `Reload`
  re-reads an externally edited file and lists invalid lines instead of
  overwriting them. The Vim checkbox is not the live mode; `Ctrl+Shift+V` and
  View → Vim Mode remain per-window.
- Find and replace support literal search only; regular expressions are not
  implemented. Vim `/` and `?` remain separate.
- Syntax highlighting is linked through `chaus-highlight`: Rust, C, C++,
  Python, JavaScript, TypeScript, TSX, JSON, TOML and shell, selected by
  extension or known name with a View → Language override (including Plain
  Text and Auto Detect). Parsers and highlight queries are compiled into the
  executable. Parsing runs on the background executor after a 30 ms debounce;
  single-edit transactions reuse the previous tree, while multi-edit
  transactions and undo/redo reparse. Results are tagged with the document
  revision and discarded if the document, language or generation changed, so a
  stale tree is never painted. Files above 2 MiB stay plain text and report a
  status; a grammar/query failure disables colour for that window without
  affecting editing. Markdown is not in this subset yet because its block and
  inline trees need a two-grammar pipeline. External formatters remain separate
  and explicit: Format Document uses `rustfmt`, `clang-format`, `black` or
  `prettier` when installed, runs with a 10-second deadline and bounded output,
  never through a shell, and discards output if the document changed.
  Format-on-save is off and no formatter runtime is bundled.
- Open/save dialogs use the Win32 common dialogs on Windows. Linux uses the
  desktop's `zenity` or `kdialog` helper when one is installed; otherwise the
  command reports that no picker is available, so pass paths on the command
  line and use `Ctrl+S` for an already-open file. The Linux helper path has not
  been smoke-tested in this Windows environment.
- The save-conflict and dirty-close sheets cover the normal paths. Reload after
  a conflict replaces the document; there is no three-way merge.
- Dirty documents write a debounced crash-recovery snapshot under the platform
  state directory (`%LOCALAPPDATA%\Chaus\Recovery`, `$XDG_STATE_HOME/chaus/recovery`,
  or `~/.local/state/chaus/recovery`). Launch offers Recover/Discard per
  snapshot, and saving or discarding removes it. Recovery is best-effort and
  does not replace a power-failure test.
- A 2-second background watcher checks identity, size and mtime first and only
  hashes when one changed, with a forced hash every 30 seconds for coarse
  timestamps. A check in flight suppresses the next tick. Review opens the
  Reload / Save As / Overwrite / Cancel sheet. The watcher never reloads or
  overwrites the document automatically.
- Edits refresh only the hard lines reported by `Document`, and the row prefix
  is adjusted by the delta instead of rebuilt. Shape caches per visible row are
  still recreated each frame; large-file paint cost is not yet measured.
- Save refuses a target with additional hard links instead of silently
  breaking them. Windows uses `ReplaceFileW` and keeps the target's ACLs and
  attributes. Conflict checks are repeated immediately before replacement, but
  the check and replacement are still not one atomic operation.
- Ctrl+Plus/Minus/0 change only the current window's document font size
  (9-48px, documented stops). The override is never written to `_chausrc`.
- Tab and Shift+Tab indent or outdent every selected hard line as one undo
  entry; `:trim`, `:retab` and the whitespace shortcuts act on the whole
  document.
- Geist themes use bundled Geist Sans for application UI and Geist Mono for the
  document; Aqua keeps Lato and Source Code Pro. The app installs Geist Sans
  through Silvestris's per-theme font-override API. A user `set font=` still
  overrides the document face.
- Word motions and line-boundary behaviour are tested against the in-memory
  engine, not compared command-for-command with real Vim.
- Tests live in `crates/*/tests/`; the native GUI still needs a real-OS smoke
  matrix on a clean machine.
