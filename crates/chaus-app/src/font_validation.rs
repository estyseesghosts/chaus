//! Decision logic for the Preferences font-family field.
//!
//! An uninstalled typo must not discard the last explicit family the person
//! chose. The pure decision here is kept separate from GPUI so it can be tested
//! without a window or a live text system.

/// What the Preferences form should do with a submitted font-family string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontDecision {
    /// Empty input: clear the explicit family and follow the theme default.
    Reset,
    /// An installed family: commit it and clear any warning.
    Accept(String),
    /// An uninstalled family: keep the last valid value and show why.
    Keep {
        retained: Option<String>,
        warning: String,
    },
}

/// Decide how to handle `family` using an availability predicate.
///
/// `current` is the last committed explicit family. When `family` is invalid,
/// it is left in `warning` text and `current` is preserved as `retained`.
#[must_use]
pub fn decide_font_family(
    family: &str,
    current: Option<&str>,
    is_available: impl Fn(&str) -> bool,
    theme_default: &str,
) -> FontDecision {
    let family = family.trim();
    if family.is_empty() {
        return FontDecision::Reset;
    }
    if is_available(family) {
        return FontDecision::Accept(family.to_string());
    }
    let retained = current.map(str::to_owned);
    let kept = current.unwrap_or(theme_default);
    let warning = format!("\"{family}\" is not installed; keeping \"{kept}\".");
    FontDecision::Keep { retained, warning }
}
