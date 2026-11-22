# AGENTS.md — implementation rules

These instructions are mandatory for implementation agents. Read [README.md](README.md) and the relevant `docs/*.md` before touching a subsystem. Deliver working increments, not architectural placeholders.

## READ BEFORE WRITING CODE 

Always follow the style as outlined in rust_style_guide.md. Always read that document entirety before you start writing new code. Additionally, you should always put tests into new files. 

## First actions

1. Inspect **current** `estyseesghosts/silvestris` source and its default branch before any integration. Record exact Silvestris SHA, GPUI SHA, Rust toolchain, platform features and license obligations in a lock/decision note. The inspected baseline on 2026-11-22 used GPUI `6b720973027999bae402df23d469026f4cced988` and Rust `1.98.1`; do not assume they never change.
2. Build the Silvestris gallery/explorer using upstream instructions. Prove the app can import its **public** crate, call `silvestris::init`, create `chrome::window_options`/`open_window`, compose `WindowFrame`, and switch all four themes.
3. Audit the selected GPUI pin's `EntityInputHandler` and `ElementInputHandler` contracts; inspect real source, not examples from latest `main`. Implement a tiny multiline input spike with IME and native focus before building the rest.
4. Establish a cross-platform test and build baseline. If an API is absent, document the gap and keep temporary adapter code minimal; avoid speculative forks.

## Dependency and layering rules

- Application owns windows, documents, CLI, persistence, language registry and modal editing. Silvestris owns generic chrome, themes, menus, settings **widgets**, accessible controls. GPUI owns paint/input platform integration. Ropey owns text storage. Tree-sitter owns parsing. No GPUI Kit, no Zed editor feature import.
- **Upstream to Silvestris** only portable, app-agnostic features: generic multiline input primitives where demonstrably reusable; command dispatch improvements; a reusable preferences field/state binding; platform dialog/clipboard abstractions if truly generic. Keep Ropey document policy, Vim grammar, language queries and scratchpad rc parser in the app. Prefer `silvestris` API extension + isolated test + upstream PR over copy-pasting framework files.
- Use **one** `Document` and transaction API for all input modes, clipboard, formatting, and undo. `editor_view` cannot mutate the Rope without `Document::apply(EditTransaction)`.
- Never silently convert all file encodings or newline types. Never write a document merely because its preview/formatter/parser changed.
- No global mutable `static` document state; key state and cursors are per-window/per-document. Settings can be app-global but applied to each window without replacing the view/focus handles.

## Build order

Follow M0–M7 in README. Within M2: (a) one-line typing, (b) newline and click positions, (c) scrolling/wrapping, (d) selections and clipboard, (e) IME, (f) history. Do not begin Vim until the shared transactions, history and text positions are tested. Do not begin syntax highlighting until line layout has stable visible-range APIs.

Each milestone PR must include: behavior summary; changed modules; screenshots/video only where visual validation matters; tests run + results; measured performance/regressions; explicit known gaps; docs updated. Avoid claiming issues fixed based solely on compilation.

## Technical invariants

- Store Ropey offsets as **character indices** in the document layer. Use explicit newtypes for UTF-8 byte indices, UTF-16 text-service indices, Unicode scalar indices, grapheme/caret boundaries, logical line/column, *visual* wrap row, and GPUI pixel X. Do not pass bare `usize` across these interfaces.
- Edit transactions are sorted and applied safely, recording old/new ranges and before/after carets. Undo reverses entire user operations; redo replays them. IME preedit must not become dozens of undo entries.
- Paint/shape only visible and overscan rows; invalidate changed line/layout spans; background parse is revision-tagged; stale results discarded; GPU/paint cannot block on formatters.
- All user-visible controls, menus and shortcuts route to shared action handlers; no duplicate Save implementations. Conflicting keybindings have documented resolution; toggling Vim cannot leave an operator pending.
- Save is checked against the file identity/version originally read, writes to a same-directory temp, preserves permissions when possible, commits with OS-appropriate replacement, and retains dirty state on any failure.
- No blocking I/O on GPUI's UI thread; disk, parse or formatter jobs hand back revision-checked results. Windows must stay responsive during long operations.

## Must-test matrix

| Category | Required tests |
| --- | --- |
| Positions | ASCII, `é`, combining accents, emoji ZWJ sequences, CJK, CRLF, tabs, RTL/mixed BiDi if supported |
| Input | US/non-US keyboard, dead keys, IME preedit/commit/cancel, paste Unicode, native selection queries |
| Selection | drag past viewport edge, double/triple-click, soft-wrapped line, empty file, EOF, word boundaries |
| History | typing groups, paste, replace all, format, Vim `ciw`/`.`, IME grouping, undo/redo after mode switch |
| File safety | UTF-8 BOM, LF/CRLF/mixed EOL, invalid UTF-8, read-only, target locked, changed on disk, save failure, crash during save |
| Vim | counts, operator-pending, text objects, `Esc`, toggle in every mode, mapping cycles, focus into dialogs |
| Highlighting | rapid edits, offscreen changes, revision races, mismatched queries, huge file fallback |
| Packaging | fresh Windows VM, fresh Linux user profile, CLI in working directory with spaces, embedded fonts and grammars |

Mandatory automated quality gates, as available: `cargo fmt --check`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, release build and native smoke runs. Add property-based offset/undo testing and targeted fuzz tests for config/Vim parsers. Test both dark/light themes at 100%, 125%, 150%, 200% display scaling where the OS exposes them.

## Explicitly forbidden shortcuts

- **Do not** build a fake text editor with one huge `String` rerendered per keystroke; do not copy Zed's entire editing stack or adopt GPUI Kit.
- **Do not** map raw keys to text mutations in multiple modules; send semantic commands to one transaction layer.
- **Do not** conflate bytes, UTF-16 code units, Rope chars, graphemes and screen columns, even where tests appear ASCII-only.
- **Do not** send all keys into Vim mode when a modal/settings/search input has focus; never swallow platform IME composition in Insert mode.
- **Do not** implement arbitrary `:!` shell commands, vimrc `source`, executable expressions, Vimscript evaluation or plugin loading for v0.1.
- **Do not** call external formatters through a shell, silently install them, format on save by default or discard the edit if the document changed while formatting.
- **Do not** claim a Linux binary is fully static or universally portable; inspect ELF dependencies. Do not ship an `.exe` requiring app-local files or unbundled language DLLs.
- **Do not** modify Silvestris's demo/theme internals merely to make the app compile. Use public APIs and propose upstreamable additions with tests.

## Completion definition

A person can copy `scratchpad.exe` (Windows) or the Linux executable to a clean supported machine, run `scratchpad` from terminal to open a responsive window, open/edit/save a UTF-8 file without corruption, switch Vim mode with Ctrl+Shift+V, switch themes in preferences, and exit without data loss. All bundled fonts, grammars and queries work with no development checkout. External formatters are optional and explicitly identified as external.
