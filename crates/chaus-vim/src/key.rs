//! Platform-neutral key input for the modal state machine.

/// One key press, stripped of modifiers the editor layer has already resolved.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Control(char),
    Escape,
    Enter,
    Backspace,
    Tab,
}

impl Key {
    /// Parse the `<...>` tokens accepted in rc mappings.
    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        match token {
            "CR" | "cr" => Some(Self::Enter),
            "Esc" | "esc" => Some(Self::Escape),
            "Space" | "space" => Some(Self::Char(' ')),
            "Tab" | "tab" => Some(Self::Tab),
            "BS" | "bs" => Some(Self::Backspace),
            _ => {
                let control = token
                    .strip_prefix("C-")
                    .or_else(|| token.strip_prefix("c-"))?;
                let mut chars = control.chars();
                let character = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                Some(Self::Control(character.to_ascii_lowercase()))
            }
        }
    }
}
