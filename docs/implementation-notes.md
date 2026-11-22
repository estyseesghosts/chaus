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
app-owned. Bundled fonts come from Silvestris; do not add a second copy.

## Known gaps in this increment

- No native menu bar yet. `Ctrl+,` opens a Preferences window for the theme and
  the Vim default applied to new windows. Other settings still come from
  `_chausrc`.
- Find and replace support literal search only; regular expressions are not
  implemented. Vim `/` and `?` remain separate.
- Tree-sitter highlighting is not linked. External formatters are explicit and
  opt-in: Format Document uses `rustfmt`, `clang-format`, `black` or `prettier`
  when installed, runs with a 10-second deadline and bounded output, never
  through a shell, and discards output if the document changed. Format-on-save
  is off and no formatter runtime is bundled.
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
  document. Geist themes still map UI text to Inter because Silvestris bundles
  no Geist Sans face and exposes no public typography override.
- Word motions and line-boundary behaviour are tested against the in-memory
  engine, not compared command-for-command with real Vim.
- Tests live in `crates/*/tests/`; the native GUI still needs a real-OS smoke
  matrix on a clean machine.
