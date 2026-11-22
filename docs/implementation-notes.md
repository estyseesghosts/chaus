# Implementation notes and verified baseline

Recorded 2026-11-22 while building the first chaus increment.

## Pins

| Component | Revision / version |
| --- | --- |
| Silvestris checkout | `C:\Users\terezi\workdir\silvestris`, branch `source-list-shell` (worktree state, not a release tag) |
| GPUI | `6b720973027999bae402df23d469026f4cced988` (same pin as Silvestris `config/pins.toml`) |
| Rust | `1.98.1-x86_64-pc-windows-gnu` (`rust-toolchain.toml`) |
| Ropey | `1.6` |
| Tree-sitter | not linked yet |
| `windows-sys` | `0.61` |

Silvestris is consumed as a path dependency during development. Before packaging,
replace it with the exact upstream revision used for the release build and record
that revision here.

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
- Tree-sitter highlighting and external formatters are not linked.
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
- No external file watcher; changes made by other programs are detected when a
  save conflict is checked.
- Open and save run on the background executor, but a save still expands the
  document into one string and one byte buffer before writing. Initial row
  layout after a load runs on the UI thread. Large files are not yet measured.
- Save refuses a target with additional hard links instead of silently
  breaking them. Windows uses `ReplaceFileW` and keeps the target's ACLs and
  attributes. Conflict checks are repeated immediately before replacement, but
  the check and replacement are still not one atomic operation.
- `selection_bounds_for_range` delegates to the existing range geometry.
  Multi-row selections use the same start/end box as `bounds_for_range` rather
  than per-row rectangles.
- Word motions and line-boundary behaviour are tested against the in-memory
  engine, not compared command-for-command with real Vim.
- Tests live in `crates/*/tests/`; the native GUI still needs a real-OS smoke
  matrix on a clean machine.
