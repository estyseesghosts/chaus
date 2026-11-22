# Vendored highlight queries

Most grammars export their `highlights.scm` as a Rust constant and chaus uses
that directly:

| Language | Source |
| --- | --- |
| Rust | `tree-sitter-rust 0.24.2` `HIGHLIGHTS_QUERY` |
| C | `tree-sitter-c 0.23.4` `HIGHLIGHT_QUERY` |
| C++ | `tree-sitter-cpp 0.23.4` `HIGHLIGHT_QUERY` |
| Python | `tree-sitter-python 0.23.6` `HIGHLIGHTS_QUERY` |
| JavaScript | `tree-sitter-javascript 0.23.1` `HIGHLIGHT_QUERY` |
| JSON | `tree-sitter-json 0.24.8` `HIGHLIGHTS_QUERY` |
| TOML | `tree-sitter-toml-ng 0.7.0` `HIGHLIGHTS_QUERY` |
| Bash | `tree-sitter-bash 0.23.3` `HIGHLIGHT_QUERY` |

`typescript.scm` is copied verbatim from `tree-sitter-typescript 0.23.2`
(`queries/highlights.scm`, MIT, <https://github.com/tree-sitter/tree-sitter-typescript>)
because that crate does not export the query as a constant. The TypeScript and
TSX parsers come from the same crate (`LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`).

All grammar parsers are compiled into the executable through their Rust crates;
no DLL, shared object or adjacent query file is loaded at runtime.
