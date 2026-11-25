# Implementation notes and verified baseline

Recorded 2026-11-22 while building the first chaus increment.

## Pins

| Component | Revision / version |
| --- | --- |
| Silvestris | `f1c2a010c1c84a90c9ad938eb307f5f2ef5a5ba7` on `estyseesghosts/silvestris`, branch `devel/sil-cocoa-fixture` (git dependency, `features = ["std-fs"]`) |
| GPUI | `6b720973027999bae402df23d469026f4cced988` (same pin as Silvestris `config/pins.toml`) |
| Rust | `1.98.1` (`rust-toolchain.toml`); this machine builds with the GNU host `x86_64-pc-windows-gnu`, CI uses the default MSVC host |
| Ropey | `1.6.1` |
| Tree-sitter | `0.25.10`; grammars and queries are compiled into the executable |
| `windows-sys` | `0.61` |

The table tracks the current `Cargo.toml` and `Cargo.lock`. Those two files are
authoritative if the pins move again.

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
grapheme. Line-break opportunities are attached to graphemes as they are
pulled rather than through a byte-offset table, and every break is snapped to a
grapheme boundary, so combining sequences, CJK and ZWJ emoji are never split.
A trailing whitespace run stays on its row while it fits one wrap width; a
longer run wraps, and only the final row may absorb one extra width, which
avoids a blank row without letting spaces widen a row without bound. Tabs keep
their stops anchored to the logical line, continuation rows start at the normal
left inset, and the source text and char offsets are untouched. A single `Rows`
walker backs `row_count`, `segments` and the editor's painting, caret and
hit-testing, so the caches cannot disagree. It buffers only the row being
decided, so a multi-MiB unwrapped line costs memory proportional to one row, and
`segments_window` builds display strings only for the visible rows.
Changing the mode reflows the window without touching the document or undo
history; `None` restores horizontal scrolling.

## Known gaps in this increment

- Right-clicking the titlebar opens the File/Edit/View commands built from
  Silvestris's `CommandRegistry` and `PopupMenu`. There is no visible menu row
  or toolbar: the window is the titlebar and the document. Each window owns its
  registry, so the checked Vim Mode/Wrap items and disabled Undo/Redo/Cut/Copy
  come from that window's state. Application and document commands are
  registered once and their GPUI chords are generated from the same registry;
  `CommandScope` is the responder boundary. `WindowFrame` owns left-button drag,
  double-click zoom and the traffic lights; a paint-registered window listener
  replaces only its native right-click menu. The document frame is
  `WindowRole::Document` with `WindowFrame::edited(dirty)`, so the title string
  never contains "Edited". `Ctrl+Shift+V` toggles Vim in both modes and is
  inert while a Silvestris text control, sheet, find bar or menu owns the
  keyboard. `Ctrl+,` opens a
  Preferences window for theme, font family, font size, line height, wrap mode,
  tab stop, spaces-vs-tabs and the Vim
  default applied to new windows. Applying a value updates every open editor
  in place through `Editor::set_settings` (document, selection, undo, focus and
  file state are preserved), writes the same validated `Settings` value to the
  rc file, and keeps comments, mappings and unsupported lines intact. `Reload`
  re-reads an externally edited file and lists invalid lines instead of
  overwriting them. The Vim checkbox is not the live mode; `Ctrl+Shift+V` and
  View → Vim Mode remain per-window.
- Width changes from paint are coalesced into one background reflow per editor.
  The committed row index and paint key keep the previous width until a
  generation- and revision-checked measurement is published, so dragging never
  paints new-width text against old row positions. Explicit wrap-mode changes
  still rebuild synchronously because they are not continuous resize events.
- Caret/native-selection row lookup uses `chaus_layout::row_of_char`, the same
  row walker as painting, and stops at the target row instead of building the
  whole segment list. A 1 MiB unwrapped line visits one row. Native selection
  geometry clips to the visible hard lines and builds only the visible row
  window, so Select All on 100k lines returns a viewport-sized set of
  rectangles. The remaining layout work is chunk-friendly input and deep-row
  checkpoints for paint.
- Find and replace support literal search only; regular expressions are not
  implemented. Vim `/` and `?` remain separate. Literal search streams Rope
  characters through a KMP prefix table, so a long needle over repeated
  prefixes no longer causes a comparison per candidate character per needle
  character. Case-insensitive matching still compares each scalar's lowercase
  expansion, which is an equivalence relation the same matcher accepts.
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
  never through a shell, and discards output if the document changed. A child
  that exits zero after reading only part of stdin is rejected rather than
  replacing the document with truncated text; a nonzero exit keeps its own
  diagnostic. At most one job runs per window: text expansion and the common
  prefix/suffix diff run on the background executor, and the result is applied
  only when its request and document revision still match. Hitting the output
  cap kills and reaps the child immediately instead of waiting for the 10-second
  deadline. Format-on-save is off and no formatter runtime is bundled.
- Open and Save As use Silvestris's `OpenPanel` and `SavePanel`. File → Open is
  an application-modal `WindowRole::OpenPanel` window titled "Chaus: Open";
  Save As is a document sheet. Both read the one `StdFsProvider` owned by
  `chaus-editor::FileDialogs` for the whole application session, because
  Silvestris location ids are opaque handles owned by that provider. Open and
  Save keep separate `PanelState` values; the Open frame and both panel states
  persist in `~/.chaus-ui.json` (Windows: `%USERPROFILE%\_chaus-ui.json`)
  through `PreferencePanelStates` and `PreferenceFrames`, never in `.chausrc`.
  A malformed UI document is reported and left untouched; the panels continue
  with defaults. `SaveDisposition::New` keeps Chaus's create-new save
  (`refuse_existing = true`), while `ReplaceConfirmed` permits overwrite.
  Explicit `:w path`, `:saveas path`, `:e path` and `:r path` still bypass the
  visual panel. Real-OS panel interaction, Trash, fractional DPI and Linux
  paths remain a manual acceptance gate.
- Startup validates positional paths with a cheap metadata check before GPUI
  starts, so missing paths and directories still produce a nonzero CLI status.
  Document decoding and the recovery scan run on the background executor after
  the first windows exist; each path owns its "Opening …" window and adopts its
  load without a second read. Recovery records get a separate untitled window
  so they never replace a CLI file. Content decode failures found after startup
  are reported in that window, not as a CLI exit code.
- GUI launches write bounded local events under the platform state directory,
  with the build SHA, monotonic sequence, correlation id and redacted phase
  outcomes. The panic hook is installed before CLI parsing; native faults are
  identified from the Windows Event Viewer record, not from a Rust stack. See
  [Diagnostics](diagnostics.md).
- One typed file operation owns the editor's Save sheet, open and write. Every
  completion checks its request id and document generation before it can
  install metadata, clear dirty state, change recovery or close the window.
  A second Save dispatched while the sheet or a write is pending is rejected,
  and a forced reload no longer overwrites edits made while its load was in
  flight.
- The save-conflict and dirty-close sheets cover the normal paths. Reload after
  a conflict replaces the document; there is no three-way merge.
- Dirty documents write a debounced crash-recovery snapshot under the platform
  state directory (`%LOCALAPPDATA%\Chaus\Recovery`, `$XDG_STATE_HOME/chaus/recovery`,
  or `~/.local/state/chaus/recovery`). The 64 MiB reader limit is now checked
  on the serialized record before writing, so an accepted snapshot is always
  listable; exceeding it returns a typed error and a visible editor status
  instead of replacing a usable snapshot with an unreadable one. Legacy records
  larger than the cap are still skipped at startup. Launch offers
  Recover/Discard per snapshot, and saving or discarding removes it. Snapshot
  retirement and removal run on the background executor; the epoch watermark
  remains the ordering barrier, and cleanup failures become a visible editor
  status. Recovery is best-effort and does not replace a power-failure test.
- A 2-second background watcher checks identity, size and mtime first and only
  hashes when one changed, with a forced hash every 30 seconds for coarse
  timestamps. A check in flight suppresses the next tick. Review opens the
  Reload / Save As / Overwrite / Cancel sheet. The watcher never reloads or
  overwrites the document automatically.
- Edits refresh only the hard lines reported by `Document`. A Fenwick tree
  answers "rows above this line" and "which line owns this row" in
  `O(log lines)`; a point edit updates one leaf, and a hard-line splice
  re-wraps only the edited range. Visible rows are shaped once per
  revision/highlight/font/theme/width and reused across text-preserving
  repaints, with a 256-row bound. Frame-time measurements on real hardware are
  still outstanding.
- Save refuses a target with additional hard links instead of silently
  breaking them. Windows uses `ReplaceFileW` and keeps the target's ACLs and
  attributes. Conflict checks are repeated immediately before replacement, but
  the check and replacement are still not one atomic operation; the adversarial
  two-writer test proves a paused writer detects a change made at that boundary,
  not that unrelated processes are serialized. A committed rename whose parent
  directory cannot be synced reports `DurabilityUnconfirmed` instead of success.
- Ctrl+Plus/Minus/0 change only the current window's document font size
  (9-48px, documented stops). The override is never written to `_chausrc`.
- Tab and Shift+Tab indent or outdent every selected hard line as one undo
  entry; `:trim`, `:retab` and the whitespace shortcuts act on the whole
  document.
- Geist themes use bundled Geist Sans for application UI and Geist Mono for the
  document; Aqua keeps Lato and Source Code Pro. The app installs Geist Sans
  through Silvestris's per-theme font-override API. A user `set font=` still
  overrides the document face; `None` follows the theme's mono. Preferences
  validates a typed family against GPUI's text system, warns and keeps the theme
  default when it is unavailable, and offers "Use Theme Default". Light/Dark
  within one family keeps the same mono; switching family changes it only when
  the user has not chosen a family.
- Word motions and line-boundary behaviour are tested against the in-memory
  engine, not compared command-for-command with real Vim.
- Tests live in `crates/*/tests/`; the native GUI still needs a real-OS smoke
  matrix on a clean machine.
