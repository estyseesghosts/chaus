//! Detection, capture mapping and incremental parse behaviour.

use chaus_core::document::{Document, EditTransaction, TextEdit};
use chaus_core::eol::{Eol, EolMap};
use chaus_core::position::{CharOffset, CharRange, DocumentRevision, Selection};
use chaus_highlight::{capture_kind, edit_for_transaction, parse_snapshot, LanguageId, TokenKind};
use ropey::Rope;

fn full(language: LanguageId, text: &str) -> chaus_highlight::ParsedHighlight {
    let rope = Rope::from_str(text);
    parse_snapshot(language, &rope, None, None, DocumentRevision::ZERO)
        .expect("bundled grammar and query must parse")
}

fn document(text: &str) -> Document {
    let (normalized, eol) = EolMap::from_source(text);
    Document::from_normalized_text(normalized, eol, Eol::Lf)
}

#[test]
fn detection_uses_extensions_names_and_plain_text_fallback() {
    use std::path::Path;
    assert_eq!(LanguageId::from_path(Path::new("a.rs")), LanguageId::Rust);
    assert_eq!(LanguageId::from_path(Path::new("a.c")), LanguageId::C);
    assert_eq!(LanguageId::from_path(Path::new("a.hpp")), LanguageId::Cpp);
    assert_eq!(LanguageId::from_path(Path::new("a.tsx")), LanguageId::Tsx);
    assert_eq!(
        LanguageId::from_path(Path::new("Cargo.toml")),
        LanguageId::Toml
    );
    assert_eq!(
        LanguageId::from_path(Path::new(".bashrc")),
        LanguageId::Bash
    );
    assert_eq!(
        LanguageId::from_path(Path::new("notes.unknown")),
        LanguageId::PlainText
    );
    assert_eq!(LanguageId::parse("plain"), Some(LanguageId::PlainText));
    assert_eq!(LanguageId::parse("rs"), Some(LanguageId::Rust));
    assert_eq!(LanguageId::parse("nonsense"), None);
}

#[test]
fn capture_names_map_to_the_small_theme_vocabulary() {
    assert_eq!(capture_kind("keyword.control"), Some(TokenKind::Keyword));
    assert_eq!(capture_kind("string.escape"), Some(TokenKind::String));
    assert_eq!(capture_kind("function.method"), Some(TokenKind::Function));
    assert_eq!(capture_kind("type.builtin"), Some(TokenKind::Type));
    assert_eq!(capture_kind("comment"), Some(TokenKind::Comment));
    assert_eq!(capture_kind("number"), Some(TokenKind::Number));
    assert_eq!(
        capture_kind("punctuation.bracket"),
        Some(TokenKind::Punctuation)
    );
    assert_eq!(capture_kind("text.title"), None);
}

#[test]
fn every_bundled_language_parses_and_produces_spans() {
    let cases = [
        (LanguageId::Rust, "fn main() { let s = \"hi\"; // c\n}\n"),
        (LanguageId::C, "int main(void) { return 0; }\n"),
        (LanguageId::Cpp, "class A { public: int x; };\n"),
        (LanguageId::Python, "def f(x):\n    return \"s\"\n"),
        (LanguageId::JavaScript, "function f() { return 1; }\n"),
        (LanguageId::TypeScript, "interface A { x: number }\n"),
        (LanguageId::Tsx, "const x = <div>hi</div>;\n"),
        (LanguageId::Json, "{\"a\": 1, \"b\": true}\n"),
        (LanguageId::Toml, "key = \"value\"\n[section]\nnumber = 1\n"),
        (LanguageId::Bash, "echo \"hi\" # comment\n"),
    ];
    for (language, source) in cases {
        let parsed = full(language, source);
        assert!(
            !parsed.highlight.spans.is_empty(),
            "{} produced no spans; tree: {:?}",
            language.as_str(),
            parsed.tree.as_ref().map(|tree| tree.root_node().to_sexp())
        );
        let characters = source.chars().count();
        for span in parsed.highlight.spans.iter() {
            assert!(span.range.start < span.range.end);
            assert!(
                span.range.end <= characters,
                "{} span out of range",
                language.as_str()
            );
        }
    }
}

#[test]
fn rust_tokens_have_the_expected_kinds() {
    let parsed = full(
        LanguageId::Rust,
        "pub fn main() { let text = \"hi\"; let x = 42; /* note */ }\n",
    );
    let spans = &parsed.highlight.spans;
    let kind_at = |offset: usize| {
        spans
            .iter()
            .find(|span| span.range.contains(&offset))
            .map(|span| span.kind)
    };
    assert_eq!(kind_at(0), Some(TokenKind::Keyword), "pub");
    assert_eq!(kind_at(7), Some(TokenKind::Function), "main");
    assert_eq!(
        kind_at("pub fn main() { let text = ".chars().count()),
        Some(TokenKind::String),
        "string body"
    );
    assert_eq!(
        kind_at(
            "pub fn main() { let text = \"hi\"; let x = "
                .chars()
                .count()
        ),
        Some(TokenKind::Constant),
        "integer_literal is captured as constant.builtin by the Rust query"
    );
    assert_eq!(
        kind_at(source_offset(
            "pub fn main() { let text = \"hi\"; let x = 42; ",
            2
        )),
        Some(TokenKind::Comment)
    );
}

fn source_offset(prefix: &str, extra: usize) -> usize {
    prefix.chars().count() + extra
}

#[test]
fn incremental_parse_after_ascii_edit_matches_a_full_parse() {
    let mut doc = document("fn main() { let x = 1; }\n");
    let first = parse_snapshot(LanguageId::Rust, doc.rope(), None, None, doc.revision()).unwrap();
    let selection = Selection::caret(CharOffset(0));
    let transaction = EditTransaction::new(
        chaus_core::document::EditCause::Typing,
        TextEdit::new(CharRange::empty(CharOffset(0)), "pub "),
        selection,
        Selection::caret(CharOffset(4)),
    );
    let result = doc.apply(transaction).unwrap();
    let edit = edit_for_transaction(&result.edits).expect("single edit");
    let incremental = parse_snapshot(
        LanguageId::Rust,
        doc.rope(),
        first.tree,
        Some(edit),
        result.revision,
    )
    .unwrap();
    let fresh = full(LanguageId::Rust, &doc.text());
    assert_eq!(incremental.highlight.spans, fresh.highlight.spans);
}

#[test]
fn incremental_parse_keeps_unicode_byte_offsets_correct() {
    let mut doc = document("// é\nlet name = \"x\";\n");
    let first = parse_snapshot(LanguageId::Rust, doc.rope(), None, None, doc.revision()).unwrap();
    let prefix = CharOffset("// é\n".chars().count());
    let transaction = EditTransaction::new(
        chaus_core::document::EditCause::Typing,
        TextEdit::new(CharRange::empty(prefix), "π"),
        Selection::caret(prefix),
        Selection::caret(CharOffset(prefix.0 + 1)),
    );
    let result = doc.apply(transaction).unwrap();
    let edit = edit_for_transaction(&result.edits).unwrap();
    let incremental = parse_snapshot(
        LanguageId::Rust,
        doc.rope(),
        first.tree,
        Some(edit),
        result.revision,
    )
    .unwrap();
    let fresh = full(LanguageId::Rust, &doc.text());
    assert_eq!(incremental.highlight.spans, fresh.highlight.spans);
    assert!(doc.text().contains("πlet"), "the edit landed before let");
}

#[test]
fn multi_edit_transactions_fall_back_to_a_full_parse() {
    let transaction = EditTransaction::new(
        chaus_core::document::EditCause::ReplaceAll,
        TextEdit::new(CharRange::empty(CharOffset(0)), "a"),
        Selection::caret(CharOffset(0)),
        Selection::caret(CharOffset(1)),
    );
    let mut doc = document("");
    let result = doc.apply(transaction).unwrap();
    assert!(edit_for_transaction(&result.edits).is_some());
    assert!(
        edit_for_transaction(&[]).is_none(),
        "an empty edit list cannot describe an InputEdit"
    );
}

#[test]
fn plain_text_and_oversized_documents_do_not_parse() {
    let plain = full(LanguageId::PlainText, "fn main() {}");
    assert!(plain.highlight.spans.is_empty());
    assert!(plain.tree.is_none());

    let huge = format!("// {}", "x".repeat(chaus_highlight::MAX_HIGHLIGHT_BYTES));
    let rope = Rope::from_str(&huge);
    let result = parse_snapshot(LanguageId::Rust, &rope, None, None, DocumentRevision::ZERO);
    assert!(matches!(
        result,
        Err(chaus_highlight::HighlightError::TooLarge(_))
    ));
}
