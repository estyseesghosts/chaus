# Chaus — native plaintext editor implementation packet

> **Working name:** the product name is not final; the implemented binary, Rust packages and config files use `chaus`. Rename consistently before the first release. The specification below describes the target application; the implemented increment and its known gaps are recorded in `docs/implementation-notes.md`.

## Product contract

Build a small native plaintext/code editor with **Rust + pinned GPUI + Silvestris**. It should resemble a plain-text Apple TextEdit window: **one document per window, no tabs, file explorer, minimap, toolbars, or permanent status bar**. Default appearance is **Geist Dark + bundled Geist Mono**. Allow Geist Light, Aqua Light, and Aqua Dark. Windows and Linux are the first release targets; retain a portable architecture for macOS later.

Two ways of editing the *same document*: conventional point-and-click/keyboard editing by default and an optional, intentionally bounded Vim mode, toggled at runtime with **Ctrl+Shift+V on Windows/Linux** (proposed default). Menu item: **View → Vim Mode**, with checked state. When on, show an unobtrusive `NORMAL` / `INSERT` / `VISUAL` / `COMMAND` indicator (bottom corner, not a persistent full-width bar). Toggling Vim mode off must immediately restore ordinary text entry and cancel pending modal operators safely. The shortcut must work even when the cursor is in the document, but not intercept a modal dialog, menu, or settings field.

One **self-contained application executable** (no colocated assets, grammar DLLs, scripts, or runtime installer). The executable must launch a GUI window from a terminal:

```sh
chaus                       # new untitled window
chaus path/to/file.txt      # open a file
chaus --new                 # new untitled window
chaus --help                # print usage in terminal without launching GUI
chaus --version             # print version
```

Open multiple positional paths as **multiple windows**. Resolve relative paths against the invoking process's current directory. Preserve proper exit statuses on invalid flags and file-open failures. `--` terminates option parsing. Support paths with spaces and non-UTF-8 OS path representations using `OsString`/`PathBuf`, not `String`. An optional `-` (stdin) mode may be added later; do not silently block on terminal stdin in v0.1.

**Self-contained** means resources required by *the app* are embedded or statically linked. It does **not** mean no OS/DLL/libc/graphics-driver dependencies. See platform build docs.

## Required features (v0.1)

- New/open/save/save-as, dirty-file prompt, multiple independent windows, safe file replacement, external-change protection, UTF-8 + BOM awareness, CRLF/LF preservation, explicit error paths.
- Plain-text mouse editing, drag/double/triple-click selection, keyboard navigation, scroll, soft wrap, copy/cut/paste, undo/redo, find and replace, Unicode graphemes, IME composition, native text services and accessibility appropriate to the pinned GPUI APIs.
- Reasonably large files without full-buffer redraw/reparse. No specific performance promise before measurement; test 1–10 MiB and 100k+ lines, including long lines.
- Ropey text model, visible-line GPUI painting, Tree-sitter highlighting for a small embedded grammar set, optional external code formatters and built-in indent/dedent/trim whitespace.
- Silvestris-managed chrome/menus/appearance/preferences UI; application-owned settings storage and a limited **vimrc-style** parser in `~/.chausrc` (on Windows, `%USERPROFILE%\_chausrc` by default). GUI and file drive one validated settings model. Never execute arbitrary Vimscript.
- Limited Vim Normal/Insert/Visual/Command grammar with repeat counts, basic operators/motions/text objects, search, undo/redo and configurable normal-mode mappings. Not a Vim clone.

## Scope boundary: NOT an IDE

**Do not add** tabs, split panes, project/workspace state, command palette as a dependency, Git, LSP, completion, debugger, integrated terminal, extension/plugin system, runtime scripting, Vimscript VM, advanced Ex semantics, collaboration, rich text, Markdown preview, GPUI Kit or the Zed editor subsystem. The UI should not visibly change character when syntax highlighting is enabled.

## Source layout

```text
chaus/
  Cargo.toml                 # workspace, shared GPUI pin with Silvestris
  Cargo.lock                 # commit for reproducible release builds
  rust-toolchain.toml        # match Silvestris toolchain initially
  crates/
    chaus-core/              # Ropey document, positions, EOL map, undo history, settings parser
    chaus-layout/            # soft-wrap segments and column mapping, no GPUI dependency
    chaus-io/                # file metadata, checked load/save, external-change detection
    chaus-cli/               # argument parsing and invocation model
    chaus-vim/               # modal state machine, motions, operators, mappings
    chaus-dialog/            # native open/save pickers and availability reporting
    chaus-editor/            # GPUI view, input handling, painting, Vim integration
    chaus-app/               # binary, window lifecycle, platform setup
  docs/                      # implementation guides and recorded gaps
```

The separation of responsibilities is the requirement, not the filename spelling; a crate must not import the application GUI from a lower layer.

## Milestones and gates

| Milestone | Deliverable | Must pass before proceeding |
| --- | --- | --- |
| M0: dependency gate | Silvestris demo builds; exact GPUI revision and OS prerequisites pinned | Windows + Linux CI build, licenses collected, `cargo test --locked` |
| M1: minimal window | One Geist Dark editor window, no tabs/toolbar, title/dirty indicator | Resize/close/focus, native window behavior, theme switching |
| M2: document editing | Rope, selection, IME, undo/redo, clipboard, soft wrapping, scroll | Unicode/IME and mouse regression suite; no full-document layout on keystroke |
| M3: safe files + CLI | File open/save, multi-window CLI, prompts, external modification detection | Round-trip fixtures; clean-machine executable smoke test |
| M4: settings | rc parser + Silvestris preference GUI and reloading | Invalid config isolation, preserve comments/manual lines |
| M5: Vim | modal state machine, toggle and mappings | standard editor and Vim modes share same undo/edit API; Vim sequence tests |
| M6: code editing | bundled grammars, incremental highlighting, formatting | edits near multi-byte characters + stale work tests |
| M7: ship | release binaries, help text, native validation | cold start, shutdown, input, DWM/Wayland/X11 checks; dependency audit |

Do not treat a passing GPUI mock test as proof of native input, DPI, font or window correctness. Build a **real-OS smoke matrix** and run it before release.

## Reading order / ownership

1. [AGENTS.md](AGENTS.md): implementation rules, sequencing, integration tests, prohibitions.
2. [docs/silvestris.md](docs/silvestris.md): upstreamable integration, exact framework limitations, configuration GUI.
3. [docs/editor.md](docs/editor.md): rendering, input, selections, undo and mode-independent commands.
4. [docs/ropey.md](docs/ropey.md): UTF-8/char/grapheme/UTF-16/visual position contracts.
5. [docs/file-io.md](docs/file-io.md): preserving bytes and safe, conflict-aware saving.
6. [docs/vim.md](docs/vim.md): modal grammar, toggle, remapping, fallback.
7. [docs/treesitter.md](docs/treesitter.md): languages, incremental parse, highlighting, formatter isolation.
8. [docs/windows-build.md](docs/windows-build.md) and [docs/linux-build.md](docs/linux-build.md): builds and distributable binaries.

## Authoritative references (check against pinned versions)

- [Silvestris repository and README](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/README.md), [Cargo.toml](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/Cargo.toml), [commands](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/docs/commands.md), [preferences](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/preferences.rs), [text field](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/controls/text_field.rs), [chrome](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/chrome.rs).
- Pinned GPUI [`input.rs`](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/src/input.rs), [example input component](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/examples/input.rs), [keybinding](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui/src/keymap/binding.rs).
- [Ropey reference](https://docs.rs/ropey/latest/ropey/struct.Rope.html), [Tree-sitter reference](https://docs.rs/tree-sitter/latest/tree_sitter/), [Tree-sitter highlights](https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html).

**Source-state caveat:** Silvestris was inspected on its then-default `source-list-shell` branch on 2026-11-22. Its public APIs are evolving. Treat API signatures in that exact branch and pinned GPUI commit as authoritative at implementation time; update this packet if those pins change.
