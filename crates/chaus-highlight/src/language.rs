//! Language detection and bundled grammar/query selection.

use std::path::Path;

/// Languages with a bundled parser and highlight query.
///
/// Markdown is intentionally absent from this increment: its grammar is split
/// into block and inline trees, which needs a different query pipeline. Plain
/// text is always available and is the fallback for unknown extensions.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LanguageId {
    #[default]
    PlainText,
    Rust,
    C,
    Cpp,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
    Json,
    Toml,
    Bash,
}

impl LanguageId {
    /// Every selectable language, with Plain Text first so it is easy to find.
    pub const ALL: [Self; 11] = [
        Self::PlainText,
        Self::Rust,
        Self::C,
        Self::Cpp,
        Self::JavaScript,
        Self::TypeScript,
        Self::Tsx,
        Self::Python,
        Self::Json,
        Self::Toml,
        Self::Bash,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::PlainText => "Plain Text",
            Self::Rust => "Rust",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Python => "Python",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Bash => "Shell",
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plain",
            Self::Rust => "rust",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Python => "python",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Bash => "bash",
        }
    }

    /// Parse a `View → Language` / rc override value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "plain" | "text" | "plaintext" => Some(Self::PlainText),
            "rust" | "rs" => Some(Self::Rust),
            "c" => Some(Self::C),
            "cpp" | "c++" | "cxx" => Some(Self::Cpp),
            "javascript" | "js" | "jsx" => Some(Self::JavaScript),
            "typescript" | "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "python" | "py" => Some(Self::Python),
            "json" | "jsonc" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "bash" | "sh" | "shell" | "zsh" => Some(Self::Bash),
            _ => None,
        }
    }

    /// Detect from a path's file name and extension. Unknown files are plain
    /// text; no parser is attempted.
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Self::PlainText;
        };
        if matches!(
            name,
            "Cargo.toml" | "Cargo.lock" | "pyproject.toml" | "taplo.toml"
        ) {
            return Self::Toml;
        }
        if matches!(
            name,
            ".bashrc" | ".bash_profile" | ".bash_aliases" | ".profile" | ".zshrc"
        ) {
            return Self::Bash;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "rs" => Self::Rust,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" => Self::Cpp,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "py" | "pyi" | "pyw" => Self::Python,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "sh" | "bash" | "zsh" | "ksh" => Self::Bash,
            _ => Self::PlainText,
        }
    }

    /// Bundled parser, or `None` for plain text.
    #[must_use]
    pub fn parser_language(self) -> Option<tree_sitter::Language> {
        Some(match self {
            Self::PlainText => return None,
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
        })
    }

    /// Bundled highlight query source.
    #[must_use]
    pub fn highlight_query(self) -> Option<&'static str> {
        Some(match self {
            Self::PlainText => return None,
            Self::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
            Self::C => tree_sitter_c::HIGHLIGHT_QUERY,
            Self::Cpp => tree_sitter_cpp::HIGHLIGHT_QUERY,
            Self::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
            Self::TypeScript | Self::Tsx => {
                include_str!("../queries/typescript.scm")
            }
            Self::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
            Self::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Self::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            Self::Bash => tree_sitter_bash::HIGHLIGHT_QUERY,
        })
    }
}
