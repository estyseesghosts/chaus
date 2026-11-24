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
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
}

impl Key {
    /// Outside the command line, navigation keys are their Normal-mode
    /// motions. Inside it they edit the input line instead.
    #[must_use]
    pub fn as_motion(self) -> Self {
        match self {
            Self::Left => Self::Char('h'),
            Self::Right => Self::Char('l'),
            Self::Up => Self::Char('k'),
            Self::Down => Self::Char('j'),
            Self::Home => Self::Char('0'),
            Self::End => Self::Char('$'),
            Self::Delete => Self::Char('x'),
            other => other,
        }
    }

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
