# chaus

A small native plaintext editor written in Rust on [GPUI](https://github.com/zed-industries/zed) and [Silvestris](https://github.com/estyseesghosts/silvestris). One executable, one document per window. No tabs, projects, plugin system or embedded terminal.

## Features

- UTF-8 editing. UTF-8 BOM and LF/CRLF/CR line endings are preserved. Saves write through a same-directory temporary file and leave the document dirty if replacement fails.
- One window per file. Passing several paths on the command line opens one window for each.
- Pointer and keyboard editing: drag/double/triple-click selection, soft wrap (word, character, off), undo/redo, clipboard, IME, and literal find and replace.
- Optional Vim mode with Normal, Insert, Visual and Command states, toggled at runtime with `Ctrl+Shift+V`. Both editing modes share one document and undo history.
- Syntax highlighting for Rust, C, C++, Python, JavaScript, TypeScript, TSX, JSON, TOML and shell. Parsers and queries are compiled into the executable; files above 2 MiB stay plain text.
- Aqua Light/Dark and Geist Light/Dark themes, selectable in Preferences (`Ctrl+,`) or the rc file.
- External-change detection with Reload / Save As / Overwrite / Cancel, a dirty-close prompt, and debounced crash-recovery snapshots.
- Optional external formatters (`rustfmt`, `clang-format`, `black`, `prettier`) when installed. Format on save is off.

Find and replace is literal only. There is no regex search, no Vimscript evaluation, no visible menu bar or toolbar — commands live in the titlebar right-click menu — and macOS is not yet validated. [Implementation notes](docs/implementation-notes.md) list the remaining gaps.

## Run

```sh
chaus                      # one untitled window
chaus notes.txt            # open a file
chaus a.txt b.txt          # one window per file
chaus --new                # also open an empty window
chaus --help
chaus --version
```

`--` ends option parsing, so paths beginning with `-` can still be opened. Unknown options print an error and exit with status 2. The Windows release binary is a GUI-subsystem program: it attaches to an existing terminal for `--help` and `--version`, and does not allocate a console when launched from Explorer.

## Build

Requires Rust 1.98.1 (pinned in `rust-toolchain.toml`), git, and a C compiler that targets the Rust host for the bundled Tree-sitter grammars. Windows has extra toolchain constraints; see [Build and run](docs/build-and-run.md).

```sh
cargo build --release -p chaus-app --bin chaus --locked
```

The executable is `target/release/chaus`, or `target\release\chaus.exe` on Windows. Fonts, grammars and highlight queries are embedded, so nothing needs to sit next to the binary.

## Keys

| Key | Action |
| --- | --- |
| `Ctrl+N` | New window |
| `Ctrl+O` | Open file |
| `Ctrl+S` / `Ctrl+Shift+S` | Save / Save As |
| `Ctrl+F` / `Ctrl+H` | Find / Replace |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / Redo |
| `Ctrl+,` | Preferences |
| `Ctrl+Shift+V` | Toggle Vim mode |
| `Ctrl+Shift+I` | Format document |
| `Ctrl+Shift+T` | Trim trailing whitespace |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` | Zoom in / out / reset (current window) |

On macOS the primary modifier is `Cmd`; that platform is not validated. Right-click the titlebar for the File, Edit and View commands.

## Configuration

Settings live in `~/.chausrc` on Linux and `%USERPROFILE%\_chausrc` on Windows. The file is a small, strict Vim-like subset, not Vimscript: no `source`, `autocmd`, expressions or shell commands. Invalid lines are reported with their line number and the valid settings still apply.

```vim
" chaus settings
set theme=geist-dark
set fontsize=14
set wrap=word
set tabstop=4
set expandtab
set novim
```

Themes are `geist-dark`, `geist-light`, `aqua-light` and `aqua-dark`. Wrap modes are `word`, `char` and `none`. The Preferences window edits the same file in place, preserving comments and lines it does not manage.

## Source layout

`chaus-core` owns the Ropey document, positions and settings parser; `chaus-layout` handles wrapping; `chaus-highlight` owns Tree-sitter parsing; `chaus-io` handles checked file I/O; `chaus-cli` parses arguments; `chaus-vim` implements the modal engine; `chaus-dialog` provides native pickers; `chaus-editor` is the GPUI view; `chaus-app` is the binary and window lifecycle.

## Documentation

- [Build and run](docs/build-and-run.md) — prerequisites, commands and artifact checks on Windows and Linux
- [Implementation notes](docs/implementation-notes.md) — verified pins, build workarounds and current gaps
- [Silvestris integration](docs/silvestris.md), [editor](docs/editor.md), [Ropey positions](docs/ropey.md), [file I/O](docs/file-io.md), [Vim](docs/vim.md), [Tree-sitter](docs/treesitter.md)
- [Windows build](docs/windows-build.md) and [Linux build](docs/linux-build.md) — packaging and release-validation requirements
