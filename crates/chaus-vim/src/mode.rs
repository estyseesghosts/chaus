//! Modal state.

/// The active editing mode. Insert mode delegates text entry to the platform
/// input handler; the other modes consume keys themselves.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    VisualChar,
    VisualLine,
    CommandLine,
}

impl Mode {
    /// Short label for the unobtrusive corner indicator.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::VisualChar | Self::VisualLine => "VISUAL",
            Self::CommandLine => "COMMAND",
        }
    }

    /// Whether the platform text input service should accept typed text.
    #[must_use]
    pub fn accepts_text(self) -> bool {
        matches!(self, Self::Insert)
    }
}
