//! `~/.chausrc` parsing.
//!
//! This is a small, strict, line-oriented subset that looks like Vimscript but
//! is deliberately not Vimscript: no `source`, no `autocmd`, no expressions,
//! no shell commands. Unknown lines are reported with their line number and
//! skipped; every valid setting still applies.

use std::fmt;

/// Predefined appearance choices shipped with the app.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeName {
    #[default]
    GeistDark,
    GeistLight,
    AquaLight,
    AquaDark,
}

impl ThemeName {
    /// Every theme exposed by the preferences UI.
    pub const ALL: [Self; 4] = [
        Self::GeistDark,
        Self::GeistLight,
        Self::AquaLight,
        Self::AquaDark,
    ];

    /// Parse a `set theme=...` value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "geist-dark" => Some(Self::GeistDark),
            "geist-light" => Some(Self::GeistLight),
            "aqua-light" => Some(Self::AquaLight),
            "aqua-dark" => Some(Self::AquaDark),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GeistDark => "geist-dark",
            Self::GeistLight => "geist-light",
            Self::AquaLight => "aqua-light",
            Self::AquaDark => "aqua-dark",
        }
    }
}

/// A normal-mode mapping from the rc file. The Vim engine validates the
/// sequence before activation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMapping {
    pub lhs: String,
    pub rhs: String,
}

impl RawMapping {
    /// Build a mapping candidate; the Vim engine validates it before use.
    #[must_use]
    pub fn new(lhs: impl Into<String>, rhs: impl Into<String>) -> Self {
        Self {
            lhs: lhs.into(),
            rhs: rhs.into(),
        }
    }
}

/// Effective editor settings.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeName,
    /// Explicit editor face. `None` uses the active theme's monospace face.
    pub font_family: Option<String>,
    pub font_size: f32,
    pub line_height: f32,
    pub wrap: bool,
    pub tab_stop: usize,
    pub expand_tabs: bool,
    /// Default Vim mode for new windows.
    pub vim: bool,
    pub mapleader: char,
    pub normal_mappings: Vec<RawMapping>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeName::GeistDark,
            font_family: None,
            font_size: 14.0,
            line_height: 1.4,
            wrap: true,
            tab_stop: 4,
            expand_tabs: true,
            vim: false,
            mapleader: '\\',
            normal_mappings: Vec::new(),
        }
    }
}

/// A recognized `set` key, for in-place GUI updates that keep comments and
/// unknown lines intact.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKey {
    Theme,
    Font,
    FontSize,
    LineHeight,
    Wrap,
    TabStop,
    ExpandTabs,
    Vim,
    Mapleader,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRole {
    Blank,
    Comment,
    Setting(SettingKey),
    Mapping,
    Unknown,
    Invalid,
}

/// One physical source line, retained verbatim for comment-preserving saves.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLine {
    pub text: String,
    pub role: LineRole,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// One-based line number.
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        write!(
            formatter,
            "line {}: {severity}: {}",
            self.line, self.message
        )
    }
}

/// The parsed result. `lines` is the comment-preserving representation the
/// preferences UI updates in place.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ParsedRc {
    pub settings: Settings,
    pub lines: Vec<SourceLine>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedRc {
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Parse rc-file text. Never panics; malformed lines become diagnostics.
#[must_use]
pub fn parse_rc(source: &str) -> ParsedRc {
    let mut settings = Settings::default();
    let mut lines = Vec::new();
    let mut diagnostics = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            lines.push(SourceLine {
                text: raw.to_string(),
                role: LineRole::Blank,
            });
            continue;
        }
        if trimmed.starts_with('"') {
            lines.push(SourceLine {
                text: raw.to_string(),
                role: LineRole::Comment,
            });
            continue;
        }
        let role = parse_line(trimmed, line_number, &mut settings, &mut diagnostics);
        lines.push(SourceLine {
            text: raw.to_string(),
            role,
        });
    }

    ParsedRc {
        settings,
        lines,
        diagnostics,
    }
}

/// Update the theme and Vim-default lines in an rc source while preserving
/// comments, ordering and unrelated settings.
///
/// A line is rewritten only when it sets one of those values on its own;
/// mixed `set` lines are left intact and an override line is appended.
#[must_use]
pub fn update_rc(source: &str, settings: &Settings) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    let mut theme_found = false;
    let mut vim_found = false;
    for line in &mut lines {
        let mut words = line.split_whitespace();
        if words.next() != Some("set") {
            continue;
        }
        let Some(value) = words.next() else {
            continue;
        };
        if words.next().is_some() {
            continue;
        }
        if value.starts_with("theme=") {
            *line = format!("set theme={}", settings.theme.as_str());
            theme_found = true;
        } else if matches!(value, "vim" | "novim") {
            *line = if settings.vim {
                "set vim".to_string()
            } else {
                "set novim".to_string()
            };
            vim_found = true;
        }
    }
    if !theme_found {
        lines.push(format!("set theme={}", settings.theme.as_str()));
    }
    if !vim_found {
        lines.push(if settings.vim {
            "set vim".to_string()
        } else {
            "set novim".to_string()
        });
    }
    let mut result = lines.join(newline);
    result.push_str(newline);
    result
}

fn parse_line(
    line: &str,
    line_number: usize,
    settings: &mut Settings,
    diagnostics: &mut Vec<Diagnostic>,
) -> LineRole {
    let mut words = line.split_whitespace();
    let Some(command) = words.next() else {
        return LineRole::Blank;
    };
    match command {
        "set" => parse_set(line, line_number, settings, diagnostics),
        "let" => parse_let(line, line_number, settings, diagnostics),
        "nnoremap" => parse_mapping(line, line_number, settings, diagnostics),
        "source" | "autocmd" | "function" | "nnoremap!" | "inoremap" => {
            diagnostics.push(Diagnostic {
                line: line_number,
                severity: Severity::Error,
                message: format!("'{command}' is not supported by chaus"),
            });
            LineRole::Invalid
        }
        _ if command.starts_with(':') => {
            diagnostics.push(Diagnostic {
                line: line_number,
                severity: Severity::Error,
                message: "Ex commands are not valid in the rc file".to_string(),
            });
            LineRole::Invalid
        }
        _ => {
            diagnostics.push(Diagnostic {
                line: line_number,
                severity: Severity::Warning,
                message: format!("unknown command '{command}'"),
            });
            LineRole::Unknown
        }
    }
}

fn parse_set(
    line: &str,
    line_number: usize,
    settings: &mut Settings,
    diagnostics: &mut Vec<Diagnostic>,
) -> LineRole {
    let argument = line["set".len()..].trim();
    if argument.is_empty() {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Warning,
            message: "set needs a name".to_string(),
        });
        return LineRole::Invalid;
    }
    let words: Vec<&str> = argument.split_whitespace().collect();
    let mut role = LineRole::Unknown;
    for word in words {
        let result = if let Some((key, value)) = word.split_once('=') {
            apply_key_value(key, value, settings)
        } else {
            apply_boolean(word, settings)
        };
        match result {
            Ok(key) => role = LineRole::Setting(key),
            Err(message) => diagnostics.push(Diagnostic {
                line: line_number,
                severity: Severity::Warning,
                message,
            }),
        }
    }
    role
}

fn apply_key_value(key: &str, value: &str, settings: &mut Settings) -> Result<SettingKey, String> {
    let value = value.trim_matches('"');
    match key {
        "theme" => {
            settings.theme =
                ThemeName::parse(value).ok_or_else(|| format!("unknown theme '{value}'"))?;
            Ok(SettingKey::Theme)
        }
        "font" => {
            if value.is_empty() {
                return Err("font needs a family name".to_string());
            }
            settings.font_family = Some(value.to_string());
            Ok(SettingKey::Font)
        }
        "fontsize" => {
            settings.font_size = parse_positive_f32(value, "fontsize")?;
            Ok(SettingKey::FontSize)
        }
        "lineheight" => {
            settings.line_height = parse_positive_f32(value, "lineheight")?;
            Ok(SettingKey::LineHeight)
        }
        "tabstop" => {
            let parsed: usize = value
                .parse()
                .map_err(|_| format!("tabstop needs a number, got '{value}'"))?;
            if parsed == 0 || parsed > 32 {
                return Err("tabstop must be between 1 and 32".to_string());
            }
            settings.tab_stop = parsed;
            Ok(SettingKey::TabStop)
        }
        _ => Err(format!("unknown setting '{key}'")),
    }
}

fn apply_boolean(word: &str, settings: &mut Settings) -> Result<SettingKey, String> {
    let (enabled, name) = match word.strip_prefix("no") {
        Some(name) => (false, name),
        None => (true, word),
    };
    match name {
        "wrap" => {
            settings.wrap = enabled;
            Ok(SettingKey::Wrap)
        }
        "expandtab" => {
            settings.expand_tabs = enabled;
            Ok(SettingKey::ExpandTabs)
        }
        "vim" => {
            settings.vim = enabled;
            Ok(SettingKey::Vim)
        }
        _ => Err(format!("unknown boolean setting '{word}'")),
    }
}

fn parse_positive_f32(value: &str, name: &str) -> Result<f32, String> {
    let parsed: f32 = value
        .parse()
        .map_err(|_| format!("{name} needs a number, got '{value}'"))?;
    if !parsed.is_finite() || parsed <= 0.0 || parsed > 256.0 {
        return Err(format!("{name} must be a positive, sane number"));
    }
    Ok(parsed)
}

fn parse_let(
    line: &str,
    line_number: usize,
    settings: &mut Settings,
    diagnostics: &mut Vec<Diagnostic>,
) -> LineRole {
    let rest = line["let".len()..].trim();
    let Some((name, value)) = rest.split_once('=') else {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Error,
            message: "only 'let mapleader = \"<char>\"' is supported".to_string(),
        });
        return LineRole::Invalid;
    };
    if name.trim() != "mapleader" {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Error,
            message: format!("cannot set variable '{}'", name.trim()),
        });
        return LineRole::Invalid;
    }
    let value = value.trim();
    let unquoted = value.trim_matches('"');
    if !value.starts_with('"') || !value.ends_with('"') || unquoted.chars().count() != 1 {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Error,
            message: "mapleader must be a single quoted character".to_string(),
        });
        return LineRole::Invalid;
    }
    settings.mapleader = unquoted
        .chars()
        .next()
        .expect("mapleader is exactly one character");
    LineRole::Setting(SettingKey::Mapleader)
}

fn parse_mapping(
    line: &str,
    line_number: usize,
    settings: &mut Settings,
    diagnostics: &mut Vec<Diagnostic>,
) -> LineRole {
    let rest = line["nnoremap".len()..].trim();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let lhs = parts.next().unwrap_or("").trim();
    let rhs = parts.next().unwrap_or("").trim();
    if lhs.is_empty() || rhs.is_empty() {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Error,
            message: "nnoremap needs a key sequence and a target".to_string(),
        });
        return LineRole::Invalid;
    }
    if lhs.chars().count() > 16 {
        diagnostics.push(Diagnostic {
            line: line_number,
            severity: Severity::Error,
            message: "mapping key sequence is longer than 16 keys".to_string(),
        });
        return LineRole::Invalid;
    }
    settings.normal_mappings.push(RawMapping {
        lhs: lhs.to_string(),
        rhs: rhs.to_string(),
    });
    LineRole::Mapping
}
