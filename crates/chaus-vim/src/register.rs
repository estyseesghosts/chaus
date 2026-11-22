//! The unnamed register shared by yank, delete and put.

/// Yanked or deleted text, remembering whether it was a whole line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Register {
    text: String,
    linewise: bool,
}

impl Register {
    pub(crate) fn set(&mut self, text: String, linewise: bool) {
        self.text = text;
        self.linewise = linewise;
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn is_linewise(&self) -> bool {
        self.linewise
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}
