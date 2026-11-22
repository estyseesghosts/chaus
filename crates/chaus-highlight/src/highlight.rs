//! Parser/tree ownership, capture classification and revision-tagged spans.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

use chaus_core::document::AppliedEdit;
use chaus_core::position::DocumentRevision;
use ropey::Rope;
use tree_sitter::{InputEdit, Parser, Query, QueryCursor, StreamingIterator as _, Tree};

use crate::language::LanguageId;

/// Documents larger than this are shown as plain text; parsing them would
/// block a background worker and allocate too much per edit. The editor keeps
/// the file fully editable and saveable; only colour is disabled.
pub const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;

/// A semantic colour class shared by every grammar.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Constant,
    Variable,
    Property,
    Operator,
    Punctuation,
    Attribute,
    Tag,
}

/// One non-overlapping highlighted range in document char offsets.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// Highlight spans for one document revision and language.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Highlight {
    pub revision: DocumentRevision,
    pub language: LanguageId,
    pub spans: Arc<Vec<Span>>,
}

impl Highlight {
    #[must_use]
    pub fn empty(revision: DocumentRevision, language: LanguageId) -> Self {
        Self {
            revision,
            language,
            spans: Arc::new(Vec::new()),
        }
    }
}

/// Successful parse result; the tree is returned so the next edit can be
/// incremental.
pub struct ParsedHighlight {
    pub highlight: Highlight,
    pub tree: Option<Tree>,
}

#[derive(Debug)]
pub enum HighlightError {
    UnsupportedLanguage(LanguageId),
    Query(String),
    Parse,
    TooLarge(usize),
}

impl std::fmt::Display for HighlightError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedLanguage(language) => {
                write!(formatter, "no grammar for {}", language.as_str())
            }
            Self::Query(message) => write!(formatter, "highlight query failed: {message}"),
            Self::Parse => write!(formatter, "the parser did not produce a tree"),
            Self::TooLarge(bytes) => {
                write!(formatter, "{bytes} bytes exceed the highlighting limit")
            }
        }
    }
}

impl std::error::Error for HighlightError {}

/// Classify a query capture name such as `keyword.control` or
/// `function.method`. Unknown captures are ignored rather than guessed.
#[must_use]
pub fn capture_kind(name: &str) -> Option<TokenKind> {
    if name.starts_with("constant.numeric") {
        return Some(TokenKind::Number);
    }
    let base = name.split('.').next().unwrap_or(name);
    Some(match base {
        "keyword" => TokenKind::Keyword,
        "type" | "constructor" | "namespace" => TokenKind::Type,
        "function" | "method" | "label" => TokenKind::Function,
        "string" | "escape" | "char" => TokenKind::String,
        "number" | "integer" | "float" => TokenKind::Number,
        "comment" => TokenKind::Comment,
        "constant" | "boolean" => TokenKind::Constant,
        "variable" => TokenKind::Variable,
        "property" | "field" => TokenKind::Property,
        "operator" => TokenKind::Operator,
        "punctuation" | "delimiter" | "bracket" => TokenKind::Punctuation,
        "attribute" => TokenKind::Attribute,
        "tag" => TokenKind::Tag,
        _ => return None,
    })
}

type QueryCache = Mutex<HashMap<LanguageId, Arc<Query>>>;

fn query_for(language: LanguageId) -> Result<Arc<Query>, HighlightError> {
    static CACHE: OnceLock<QueryCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(query) = cache
        .lock()
        .expect("highlight query cache poisoned")
        .get(&language)
    {
        return Ok(query.clone());
    }
    let parser_language = language
        .parser_language()
        .ok_or(HighlightError::UnsupportedLanguage(language))?;
    // tree-sitter-typescript ships only the TypeScript additions; its normal
    // editor integration inherits the JavaScript query for expression-level
    // captures.
    let source = if matches!(language, LanguageId::TypeScript | LanguageId::Tsx) {
        format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            language
                .highlight_query()
                .ok_or(HighlightError::UnsupportedLanguage(language))?
        )
    } else {
        language
            .highlight_query()
            .ok_or(HighlightError::UnsupportedLanguage(language))?
            .to_string()
    };
    let query = Query::new(&parser_language, &source)
        .map_err(|error| HighlightError::Query(error.to_string()))?;
    let query = Arc::new(query);
    cache
        .lock()
        .expect("highlight query cache poisoned")
        .insert(language, query.clone());
    Ok(query)
}

/// Parse a document snapshot and return char-offset spans.
///
/// A single edit reuses the previous tree; multi-edit transactions, undo/redo
/// or a language change pass no tree and reparse. The caller tags the result
/// with the revision and discards it if the document moved on.
pub fn parse_snapshot(
    language: LanguageId,
    rope: &Rope,
    tree: Option<Tree>,
    edit: Option<InputEdit>,
    revision: DocumentRevision,
) -> Result<ParsedHighlight, HighlightError> {
    if language == LanguageId::PlainText {
        return Ok(ParsedHighlight {
            highlight: Highlight::empty(revision, language),
            tree: None,
        });
    }
    if rope.len_bytes() > MAX_HIGHLIGHT_BYTES {
        return Err(HighlightError::TooLarge(rope.len_bytes()));
    }
    let parser_language = language
        .parser_language()
        .ok_or(HighlightError::UnsupportedLanguage(language))?;
    let query = query_for(language)?;
    let text = rope.to_string();
    let mut parser = Parser::new();
    parser
        .set_language(&parser_language)
        .map_err(|error| HighlightError::Query(error.to_string()))?;
    let mut previous = tree;
    if let (Some(tree), Some(edit)) = (previous.as_mut(), edit) {
        tree.edit(&edit);
    }
    let parsed = parser
        .parse(&text, previous.as_ref())
        .ok_or(HighlightError::Parse)?;
    let spans = collect_spans(&query, parsed.root_node(), &text, rope);
    Ok(ParsedHighlight {
        highlight: Highlight {
            revision,
            language,
            spans: Arc::new(spans),
        },
        tree: Some(parsed),
    })
}

fn collect_spans(query: &Query, root: tree_sitter::Node<'_>, text: &str, rope: &Rope) -> Vec<Span> {
    let mut cursor = QueryCursor::new();
    let mut ranges = Vec::new();
    let mut captures = cursor.captures(query, root, text.as_bytes());
    while let Some((query_match, index)) = captures.next() {
        let capture = query_match.captures[*index];
        let node = capture.node;
        let name = query.capture_names()[capture.index as usize];
        let Some(kind) = capture_kind(name) else {
            continue;
        };
        if node.start_byte() >= node.end_byte() {
            continue;
        }
        ranges.push((node.start_byte()..node.end_byte(), kind));
    }
    let resolved = resolve_overlapping_captures(ranges);
    // Byte→char conversion is deliberately last: the interval algorithm works
    // in Tree-sitter's native UTF-8 byte space and only finalized boundaries
    // touch the Rope index.
    let mut spans: Vec<Span> = Vec::with_capacity(resolved.len());
    for (bytes, kind) in resolved {
        let start = rope.byte_to_char(bytes.start);
        let end = rope.byte_to_char(bytes.end);
        if start >= end {
            continue;
        }
        if let Some(last) = spans.last_mut() {
            if last.range.end == start && last.kind == kind {
                last.range.end = end;
                continue;
            }
        }
        spans.push(Span {
            range: start..end,
            kind,
        });
    }
    spans
}

/// Resolve possibly overlapping captures into sorted, non-overlapping spans.
///
/// Precedence is deterministic: the shortest capture wins the portion it
/// covers, then the earlier start, then (for identical ranges) the capture
/// order produced by the query cursor. This keeps an inner string escape or
/// interpolation coloured without discarding the surrounding string capture;
/// the outer capture keeps only the bytes the inner one did not claim.
///
/// The implementation sweeps capture boundaries and keeps active captures in a
/// heap keyed by that precedence, so it is `O(n log n)` in the capture count
/// instead of comparing every capture with every accepted span. Adjacent
/// pieces with the same kind are coalesced.
#[must_use]
pub fn resolve_overlapping_captures(
    captures: Vec<(Range<usize>, TokenKind)>,
) -> Vec<(Range<usize>, TokenKind)> {
    struct Active {
        start: usize,
        end: usize,
        order: usize,
    }

    let kinds: Vec<TokenKind> = captures.iter().map(|(_, kind)| *kind).collect();
    let mut items: Vec<Active> = captures
        .iter()
        .enumerate()
        .filter(|(_, (range, _))| range.start < range.end)
        .map(|(order, (range, _))| Active {
            start: range.start,
            end: range.end,
            order,
        })
        .collect();
    if items.is_empty() {
        return Vec::new();
    }
    // Start order lets the sweep add every capture whose start is at or before
    // the current boundary without rescanning the input.
    items.sort_by_key(|item| item.start);

    let mut boundaries: Vec<usize> = Vec::with_capacity(items.len() * 2);
    for item in &items {
        boundaries.push(item.start);
        boundaries.push(item.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    // (length, start, capture order, end). The first three form the documented
    // precedence; the end is carried so expired entries can be skipped without
    // looking them up in a second structure.
    type ActiveKey = std::cmp::Reverse<(usize, usize, usize, usize)>;
    let mut heap: std::collections::BinaryHeap<ActiveKey> = std::collections::BinaryHeap::new();
    let mut next_item = 0usize;
    let mut resolved: Vec<(Range<usize>, TokenKind)> = Vec::new();
    for (position, boundary) in boundaries.iter().copied().enumerate() {
        while next_item < items.len() && items[next_item].start <= boundary {
            let item = &items[next_item];
            heap.push(std::cmp::Reverse((
                item.end - item.start,
                item.start,
                item.order,
                item.end,
            )));
            next_item += 1;
        }
        // Captures that end at this boundary are no longer active. They stay
        // in the heap until they reach the top, which keeps stale removal out
        // of the hot path.
        while let Some(std::cmp::Reverse((_, _, _, end))) = heap.peek() {
            if *end <= boundary {
                heap.pop();
            } else {
                break;
            }
        }
        let Some(next_boundary) = boundaries.get(position + 1).copied() else {
            break;
        };
        if next_boundary <= boundary {
            continue;
        }
        let Some(std::cmp::Reverse((_, _, order, _))) = heap.peek().copied() else {
            continue;
        };
        let kind = kinds[order];
        if let Some((last, last_kind)) = resolved.last_mut() {
            if last.end == boundary && *last_kind == kind {
                last.end = next_boundary;
                continue;
            }
        }
        resolved.push((boundary..next_boundary, kind));
    }
    resolved
}

/// Build the single edit for a transaction, or `None` when incremental
/// coordinates cannot be derived safely.
#[must_use]
pub fn edit_for_transaction(edits: &[AppliedEdit]) -> Option<InputEdit> {
    match edits {
        [edit] => crate::input_edit(edit),
        _ => None,
    }
}
