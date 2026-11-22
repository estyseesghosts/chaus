# Tree-sitter: incremental syntax highlighting without an IDE

## Implemented subset (2026-11-22)

`chaus-highlight` links `tree-sitter 0.25` and the grammar crates for Rust, C,
C++, Python, JavaScript, TypeScript/TSX, JSON, TOML and shell. Captures map to
a small token vocabulary (keyword, type, function, string, number, comment,
constant, variable, property, operator, punctuation, attribute, tag) and the
editor paints them with the active Silvestris semantic palette. View → Language
offers Auto Detect, Plain Text and each bundled language; detection also maps
`Cargo.toml`/`Cargo.lock` to TOML and rc files to shell. TypeScript reuses the
JavaScript query plus the vendored TypeScript additions
(`crates/chaus-highlight/queries/`). Markdown is deferred: its grammar needs two
linked trees (block and inline), which is a separate pipeline.

The editor debounces 30 ms, parses a Rope snapshot on the background executor,
reuses the tree only for a single-edit transaction, and accepts spans only when
generation, document revision and language still match. Files above 2 MiB and
Plain Text are not parsed. `chaus-highlight` has no GPUI dependency and tests
cover detection, capture mapping, all bundled grammars, Unicode byte→char
offsets, incremental-vs-full equality and the oversized fallback.

## Scope

Use [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) for **syntax colour only**. No LSP, semantic diagnostics, project index, autocomplete, fold tree, code navigation, or grammar downloads. Start with embedded languages: Rust, C, C++, Python, JavaScript, TypeScript, JSON, TOML, Bash/shell and Markdown. The specific grammar crate versions must be selected and pinned as a **mutually compatible** set with the chosen `tree-sitter` runtime ABI. Check each grammar's license and API at lock time.

Detect language by file extension, then known filename (`Cargo.toml`, `Makefile`, `.bashrc`), then explicit `View → Language` override. Unsaved files are plain text unless the user chooses language. An unknown extension is plain text, with no parse attempt. **Plaintext must remain visually and functionally first-class.**

## Bundled resources

- Link grammar parsers into the executable via their Rust crates/C compilation (`tree-sitter-rust`, `tree-sitter-c`, etc.); do **not** ship individual grammar DLLs or `.so` files and do not dynamically download parsers.
- Bundle `highlights.scm` queries using `include_str!`, verifying query compatibility at startup or CI. Some grammar crates do not bundle the desired queries: vendor a licensed query file **with source and attribution**, then test it.
- Expose a `LanguageRegistry { id, extensions, filenames, parser_language, highlight_query, formatter }`. Cache compiled `Query` objects by language. A failing grammar/query should disable highlighting for that language and emit an actionable diagnostic, not crash the window.
- Map captures to theme-dependent tokens (keyword, type, function, string, number, comment, punctuation, operator, constant). Do not hardcode RGBA in grammar queries. Geist Dark default; Aqua variants must remain legible.

## Incremental edit and parse contract

The document model owns **Ropey character indices**, while Tree-sitter uses **UTF-8 byte offsets** and row/column `Point`s in **bytes**. Construct [`InputEdit`](https://docs.rs/tree-sitter/latest/tree_sitter/struct.InputEdit.html) at transaction creation from the **old state and inserted text**; send exact `start_byte`, `old_end_byte`, `new_end_byte`, `start_position`, `old_end_position`, `new_end_position`. Points count hard line rows and UTF-8 byte columns, not shaped visual rows or Ropey chars. Prefer newline-normalized text in the parse view, matching document canonical text.

On successful edit:

```text
Document::apply(tx)
  -> EditResult { revision, edits_in_application_order, changed_lines }
  -> language worker receives snapshot + old Tree + InputEdit(s)
  -> old_tree.edit(edit) for each applied edit
  -> parser.parse_with(rope chunk callback, Some(&edited_old_tree))
  -> QueryCursor highlights only relevant visible/changed ranges
  -> UI applies spans iff (document id, revision, language id) still match
```

For multi-edit transactions, `InputEdit`s must be applied **in the same coordinate order** the Rope mutations occurred. If correct edit transforms cannot be constructed, perform a full reparse for that rare transaction; do not feed invalid tree offsets. Tree-sitter's incremental contract explicitly requires calling `Tree::edit` before `Parser::parse(..., Some(&old_tree))`; see [Parser](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Parser.html) and [Tree](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Tree.html).

**Threading:** parser instances are owned by dedicated worker tasks/threads, never shared unsafely across GPUI contexts; the UI uses revision-checked immutable highlight snapshots. If the user types faster than the parser, coalesce superseded work and cancel/discard older results. A language change invalidates the old tree/query even if revision is unchanged. Bound parse and capture work on enormous inputs; degrade to plaintext without freezing the UI.

**Rope reading:** avoid `rope.to_string()` on every keystroke. Use parser's supported chunk callback / read API with Rope slices, supplying contiguous UTF-8 chunk bytes with correct lifetime. Check the exact pinned `tree-sitter` crate signature; preserve a snapshot for worker lifetime. A one-time flatten for small documents on initial open may be acceptable if measured; repeated full-buffer allocation is not.

## Highlight spans and rendering

Use `QueryCursor` to produce sorted styled spans in **UTF-8 bytes**; convert to Rope char offsets for document representation and to local shaped-run byte offsets for GPUI. Scope cursor queries to changed/visible byte ranges plus a configurable lookbehind/overscan for multiline constructs; do not globally repaint after every edit. Tree-sitter can have captures overlapping; define precedence (e.g. more specific named captures win, query order as tie break) and normalize spans to non-overlapping runs before shaping.

Each line layout cache key includes font, tab width, wrap width, line revision, language/highlight revision, theme revision and DPI; a theme change only recalculates colour/style data, not document storage. Lines that do not have fresh highlighting may temporarily display old safe highlights or plain text; never show spans from an **incompatible document revision** that could index outside text.

Test strings/comments across edits and multiline delimiters. Capture boundaries for emoji/non-ASCII must use byte offsets derived from UTF-8, not `chars().count()`.

## Formatting (separate from highlighting)

Built-in: indent/outdent, auto-indent, trim trailing whitespace, convert tabs/spaces and explicit EOL conversion. These act through document transactions and preserve undo.

External opt-in: `rustfmt`, `clang-format`, `black`, `prettier`/other known executables as installed; no bundled formatter runtime unless specifically approved. Detect executable in PATH, show unavailability in menu; never download tools. Execute direct `Command::new(program).args([...])` with explicit arguments, **no shell interpolation**, constrained timeout and output size, inherited/captured stderr according to UI policy, and safe cwd. Prefer stdin/stdout protocols to avoid overwriting on-disk originals. Feed captured document **revision** into job; if revision changes before formatter returns, discard output or ask user to retry, never apply stale changes. Apply diff or full replace as **one undo transaction**; preserve EOL policy and file encoding. Format-on-save is OFF by default.

Avoid claiming formatters support all languages; registry explicitly lists availability and flags. Never rewrite the file on disk directly from a formatter without first applying the operation through the editor.

## Test gates

- CI verifies grammar/query build and capture names, all languages; both light/dark themes have nonzero contrast checks for major tokens.
- Multi-byte UTF-8 inserted before token → changed range correct; multiline string/comment edits across rows → later syntax remains correct; syntax errors → parser recovers.
- Edit while parse running, then undo, switch language, close window: stale worker results are discarded without panic/use-after-free.
- Open 10 MiB/100k-line fixture and long single line: edit/scroll responsive; bounded parser fallback logged and visible plain text.
- Formatter absent, exits nonzero, produces invalid UTF-8, times out or outputs huge text: unchanged document + accessible error. Formatter races with user typing: no overwrite.
- Embedded grammar verification on clean machine with zero external assets.

## Do not

- Do not parse synchronously inside GPUI `render`/`paint` or reparse the whole document after each key.
- Do not share mutable `Parser` across threads or let workers mutate `Document`.
- Do not use Tree-sitter node offsets as Ropey char indices; they are byte offsets.
- Do not import a full theme manager or Zed language system to get syntax colours.
- Do not ship grammar DLLs, network-fetched queries, LSP servers, or implicit external formatter dependencies.

## Sources

[Tree-sitter syntax highlighting queries](https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html), [Rust Parser](https://docs.rs/tree-sitter/latest/tree_sitter/struct.Parser.html), [InputEdit](https://docs.rs/tree-sitter/latest/tree_sitter/struct.InputEdit.html), [QueryCursor](https://docs.rs/tree-sitter/latest/tree_sitter/struct.QueryCursor.html), [Ropey API](https://docs.rs/ropey/latest/ropey/struct.Rope.html).
