# Silvestris integration and upstream contributions

## Verified baseline (2026-11-24)

Chaus is pinned to `estyseesghosts/silvestris` `f1c2a010c1c84a90c9ad938eb307f5f2ef5a5ba7`
on `devel/sil-cocoa-fixture`, with `features = ["std-fs"]`. The workspace keeps
Silvestris's GPUI pin `6b720973027999bae402df23d469026f4cced988` and Rust
`1.98.1`. The pinned tree bundles Geist Mono and Source Code Pro, exposes
**Aqua Light/Dark and Geist Light/Dark**, and documents `StdFsProvider` as the
production host-filesystem adapter.

Public APIs used by Chaus:

- `file_panel::{OpenPanel, SavePanel, SaveDisposition, ExtensionPolicy}` for
  File → Open and Save As. The panel chooses a target; the consumer performs
  all I/O.
- `file_panel::StdFsProvider` behind `std-fs`. Its `LocationId`s are opaque
  handles owned by one provider instance; `home()`, `root()` and
  `location_of()` intern paths, and `resolve()` returns exact `PathBuf`s.
- `file_panel::{PanelStateStore, PreferencePanelStates}` and
  `window_role::{FrameStore, PreferenceFrames}` for consumer-owned panel state
  and frame persistence.
- `command::{CommandRegistry, CommandScope, apply_bindings}` for one dispatch
  path for menu, toolbar and shortcut activation. Bindings exclude focused
  Silvestris text clients.
- `window_role::{WindowRole, role_options}` for document, Preferences and
  Open-panel window semantics.

- [`silvestris::init`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/lib.rs) registers fonts, installs theme and text-field keybindings.
- [`chrome::window_options`, `chrome::open_window`, `WindowFrame`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/chrome.rs) manage native/client window chrome. macOS controls are native; Windows custom controls map to native window actions; Linux client-side decorations are implemented. The current source says macOS is **not yet validated**.
- [`theme::set_theme_preference`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/theme.rs) changes all windows' appearance without recreating their state.
- [`CommandRegistry`, `CommandScope`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/docs/commands.md) provide one command dispatch path. Command binding snapshots are startup-time; changing shortcut descriptions **does not** dynamically replace GPUI global bindings.
- [`PopupMenu`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/docs/menus.md) provides context menus; native macOS menu projection exists but OS parity is not complete.
- [`PreferencesNavigator`, `Form`, `TabView`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/preferences.rs) are **UI components**. Preferences persistence belongs to consumers.
- [`controls::text_field`](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/controls/text_field.rs) is **single-line**; it is a reference for native IME/selection handling, not the document editor.

## File dialogs

`chaus-editor::FileDialogs` is installed once by `chaus-app` and lives for the
application session. It holds:

- one `Rc<StdFsProvider>`, never replaced;
- a separate `PanelStateStore` for Open and for Save;
- the Open panel's `FrameStore`;
- a weak handle to the last presented panel, used only for diagnostics and
  integration tests.

`save_as_inner` creates `SavePanel::begin_sheet` with `ExtensionPolicy::Any`
and the current file's parent/name (home + `Untitled` otherwise). Its outcome
maps directly onto the existing save engine: `New → refuse_existing = true`,
`ReplaceConfirmed → refuse_existing = false`. The panel never writes bytes, and
`chaus_io::save`, durability, recovery and document-generation checks are
unchanged. `File → Open` is `OpenPanel::run_modal`; accepted paths go to the
existing background `chaus_io::load` and open their own document window.

`~/.chaus-ui.json` (Windows: `%USERPROFILE%\_chaus-ui.json`) stores the two
panel states and the Open frame. It is separate from `.chausrc`; a malformed
document is reported and never overwritten.

## Integration recipe

1. Depend on Silvestris via the agreed workspace/path/git revision, ensuring its **exact GPUI pin** is shared. Avoid duplicate incompatible GPUI type universes. Commit lockfile + Rust toolchain and build both GUI examples on both target OSes.
2. At app start, call `silvestris::init(ThemePreference::explicit(ThemeId::GEIST_DARK), cx)` (adapt signature to current source); load saved preference first, but use explicit Geist Dark when none exists. Do not inherit Silvestris demo's Aqua Light default.
3. Create a `WindowFrame` **without** a toolbar or bottom bar; use a single full-size document view. Keep title `Untitled` or the filename, plus modified indicator. No `Shell` sidebar/inspector: it solves a different layout problem.
4. Create a global `CommandRegistry` with stable ids: `file.new`, `file.open`, `file.save`, `file.save_as`, `file.close`, `edit.undo`, `edit.redo`, `edit.find`, `edit.replace`, `edit.format`, `view.vim_mode`, `view.appearance`, `settings.open`. The GUI menu, context menu and shortcuts must dispatch the same ids. Publish checked/disabled states as the document changes.
5. Display `View → Vim Mode` checked according to **active window**. Bind proposed shortcut `Ctrl+Shift+V` on Windows/Linux; platform-specific shortcuts require collision testing against GPUI/OS and app text contexts. A future macOS default may differ.
6. Build Settings using Silvestris preferences components: **Appearance** (theme, font, size, spacing), **Editor** (wrap, indent, numbers), **Vim** (enabled with `set vim`; standard editing is the default, plus mappings/help and the shortcut), **Files** (EOL, BOM/backup/recovery preferences). Settings model, validation, save/reload and conflict detection remain app-owned.

## Reusable upstream work: where the boundary lies

When implementation exposes a generic gap in Silvestris, write an upstreamable patch with public API, focused demo and tests. Candidate contributions:

| Reusable improvement | Belongs in Silvestris? | API boundary |
| --- | --- | --- |
| Multiline **text input bridge** with IME/selection geometry | Maybe; after prototype demonstrates generality | Input events, focus/selection, scroll access; **not** Ropey/Vim |
| Editable preferences field bound to external state | Yes | Typed read/validate/write callbacks; app owns persistence |
| Runtime shortcut replacement/registration | Yes, if feasible at pinned GPUI | Scope-aware keymap adapter and collision errors |
| Cross-platform native open/save dialog abstraction | Yes, if not already provided | Paths and cancellation/errors only; no file policy |
| Generic command toggle/menu checked-state semantics | Yes | No editor-specific enum in Silvestris |
| Syntax highlighting engine, Vim interpreter, formatter | **No** | Application-owned |
| chaus rc file parser | **No** | Application-owned |
| Entire Ropey edit model | **No** | Application-owned |

Upstream contribution process: reproduce gap with framework gallery first; add general API + unit/interaction test; keep theme token usage; validate Aqua/Geist, mouse/keyboard and native OS; update framework docs; submit in isolated commit/PR; consume via exact pinned revision, not an app-local copied widget.

## Config source of truth and GUI synchronization

Define one `Settings` struct with derived `EffectiveSettings` (defaults + user overrides). Parse `~/.chausrc` on Linux and `%USERPROFILE%\_chausrc` on Windows; allow `--config <path>` for tests/advanced use only if needed. Format is **Vim-like subset**, not genuine vimrc compatibility:

```vim
" chaus-specific, not general Vimscript
set theme=geist-dark
set font=GeistMono
set fontsize=14
set lineheight=1.4
set wrap
set tabstop=4
set expandtab
set novim
let mapleader = " "
nnoremap <leader>w :w<CR>
```

Supported grammar in v0.1: blank lines, comments beginning with `"` outside a value, `set` recognized keys/boolean `no` negation, one special-case `let mapleader = ...`, `nnoremap` for supported key sequences, and optionally `inoremap` later. Reject or warn on `source`, `autocmd`, unknown functions, expressions and `:!` operations. Define escaping, quoting and how literal `"` appears **before** permitting values containing spaces. No eval. Diagnose file/line and retain valid settings when one line is invalid.

GUI editing must **not replace the whole file**. Parse to a line-preserving representation with source spans and comments, update recognized field lines in place (or add managed lines), preserve unknown/mapping/comment lines, write conflict-aware through the same safe-file helper, and reload the canonical model. On a file change, offer Reload / Keep Local / Open Config; never merge blindly over another process's edits. Mappings should be validated against known command IDs and mode scopes. Run-time changes are applied without tearing down the active editor, and persist only on a successful write.

## Do not

- Do not fork Silvestris to introduce app-specific editor code, or copy the whole `controls/text_field.rs` into the app.
- Do not use the framework `Shell` because the sample explorer uses it; chaus needs only `WindowFrame`.
- Do not claim the existing Silvestris settings UI already persists user preferences.
- Do not treat a command descriptor shortcut update as a live GPUI keymap update; source explicitly says it is not.
- Do not ship a second copy of fonts Silvestris already bundles, and do not
  fetch fonts from a network at runtime. If an app needs a face Silvestris does
  not carry, bundle it with its license and provenance and select it through the
  public font-override API rather than editing theme internals.
- Do not infer native macOS/Linux behavior from Windows mock/render tests.

## Verification

`cargo test --workspace --locked`; theme switch with focused document and nonempty selection; 4-theme popup/settings visual regression; keyboard dispatch precedence with editor/textfield/modal; config GUI round-trip retaining comments; theme switch must retain focus, undo history, scroll position and active Vim mode.
