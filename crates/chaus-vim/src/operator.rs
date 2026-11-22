//! Applying operators to a validated character range.

use chaus_core::document::{Document, EditCause, EditTransaction, TextEdit};
use chaus_core::position::{CharOffset, CharRange, Selection};

use crate::register::Register;

/// A Vim operator waiting for a motion or text object.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    pub(crate) fn from_key(key: char) -> Option<Self> {
        match key {
            'd' => Some(Self::Delete),
            'c' => Some(Self::Change),
            'y' => Some(Self::Yank),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> char {
        match self {
            Self::Delete => 'd',
            Self::Change => 'c',
            Self::Yank => 'y',
        }
    }
}

/// What the caller should do after an operator ran.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) enter_insert: bool,
    pub(crate) caret: CharOffset,
}

/// Apply `operator` to `range`, routing every edit through the document
/// transaction API. `linewise` only affects the register's paste semantics.
pub(crate) fn apply(
    operator: Operator,
    document: &mut Document,
    selection: Selection,
    range: CharRange,
    linewise: bool,
    register: &mut Register,
) -> Option<Applied> {
    match operator {
        Operator::Yank => {
            let text = document.slice(range);
            register.set(
                if linewise && !text.ends_with('\n') {
                    format!("{text}\n")
                } else {
                    text
                },
                linewise,
            );
            Some(Applied {
                enter_insert: false,
                caret: range.start,
            })
        }
        Operator::Delete | Operator::Change => {
            let text = document.slice(range);
            register.set(
                if linewise && !text.ends_with('\n') {
                    format!("{text}\n")
                } else {
                    text
                },
                linewise,
            );
            let after = Selection::caret(range.start);
            let transaction =
                EditTransaction::new(EditCause::Vim, TextEdit::deletion(range), selection, after);
            document.apply(transaction).ok()?;
            Some(Applied {
                enter_insert: operator == Operator::Change,
                caret: range.start,
            })
        }
    }
}
