//! The temporary `:`, `/` and `?` input line and its per-window history.
//!
//! The command text lives here, never in the document. The caret is a byte
//! offset that only ever rests on an extended-grapheme boundary of `text`.

use std::collections::VecDeque;

use unicode_segmentation::UnicodeSegmentation;

/// Longest command line accepted, in Unicode scalars. Input past this is
/// refused with a message rather than silently dropped.
pub const MAX_COMMAND_CHARS: usize = 1024;

/// Entries kept per history. History is per window and never written to disk.
pub const MAX_HISTORY: usize = 100;

/// Which input the line feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandPrefix {
    Ex,
    SearchForward,
    SearchBackward,
}

impl CommandPrefix {
    #[must_use]
    pub fn symbol(self) -> char {
        match self {
            Self::Ex => ':',
            Self::SearchForward => '/',
            Self::SearchBackward => '?',
        }
    }
}

/// Submitted lines, oldest first. Empty lines and immediate repeats are not
/// recorded; lines that failed to parse are, so they can be corrected.
#[derive(Debug, Default)]
pub(crate) struct History {
    entries: VecDeque<String>,
}

impl History {
    pub(crate) fn push(&mut self, entry: &str) {
        if entry.trim().is_empty() || self.entries.back().is_some_and(|last| last == entry) {
            return;
        }
        if self.entries.len() == MAX_HISTORY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry.to_owned());
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(String::as_str)
    }
}

/// Text refused because the line would exceed [`MAX_COMMAND_CHARS`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommandFull;

/// Opaque identity of one Tab request on one command line.
///
/// `generation` changes when a new line opens, `revision` changes with the
/// line's text, and `sequence` orders repeated Tab presses that leave the text
/// alone. All three are owned by the `Engine` (and therefore by one editor
/// window); no process-wide counter is involved. A result is applied only when
/// the whole token still matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompletionToken {
    generation: u64,
    revision: u64,
    sequence: u64,
}

impl CompletionToken {
    pub(crate) fn new(generation: u64, revision: u64, sequence: u64) -> Self {
        Self {
            generation,
            revision,
            sequence,
        }
    }
}

/// The active input line.
#[derive(Debug)]
pub(crate) struct CommandLine {
    prefix: CommandPrefix,
    text: String,
    caret: usize,
    /// Identifies this session. It changes for every new line, so a revision
    /// from a previous line cannot collide with one from this line.
    generation: u64,
    /// Bumped on every text change within this session.
    revision: u64,
    /// Index into the history while browsing it, and the unsent draft that
    /// returning past the newest entry restores.
    browsing: Option<usize>,
    draft: String,
    /// Uncommitted IME composition, shown at the caret but not part of `text`.
    preedit: Option<String>,
}

impl CommandLine {
    pub(crate) fn new(prefix: CommandPrefix, generation: u64) -> Self {
        Self {
            prefix,
            text: String::new(),
            caret: 0,
            generation,
            revision: 0,
            browsing: None,
            draft: String::new(),
            preedit: None,
        }
    }

    pub(crate) fn prefix(&self) -> CommandPrefix {
        self.prefix
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn before_caret(&self) -> &str {
        &self.text[..self.caret]
    }

    pub(crate) fn after_caret(&self) -> &str {
        &self.text[self.caret..]
    }

    pub(crate) fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn token(&self, sequence: u64) -> CompletionToken {
        CompletionToken::new(self.generation, self.revision(), sequence)
    }

    pub(crate) fn caret_at_end(&self) -> bool {
        self.caret == self.text.len()
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.browsing = None;
    }

    /// Insert at the caret. Line breaks end the inserted text; the caller is
    /// told when anything was refused.
    pub(crate) fn insert(&mut self, text: &str) -> Result<(), CommandFull> {
        let text = text.split(['\r', '\n']).next().unwrap_or_default();
        if text.is_empty() {
            return Ok(());
        }
        if self.text.chars().count() + text.chars().count() > MAX_COMMAND_CHARS {
            return Err(CommandFull);
        }
        self.text.insert_str(self.caret, text);
        self.caret += text.len();
        self.preedit = None;
        self.touch();
        Ok(())
    }

    pub(crate) fn set_preedit(&mut self, preedit: Option<String>) {
        self.preedit = preedit.filter(|text| !text.is_empty());
    }

    fn previous_boundary(&self) -> usize {
        self.text[..self.caret]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.caret..]
            .graphemes(true)
            .next()
            .map_or(self.caret, |grapheme| self.caret + grapheme.len())
    }

    pub(crate) fn left(&mut self) {
        self.caret = self.previous_boundary();
    }

    pub(crate) fn right(&mut self) {
        self.caret = self.next_boundary();
    }

    pub(crate) fn home(&mut self) {
        self.caret = 0;
    }

    pub(crate) fn end(&mut self) {
        self.caret = self.text.len();
    }

    pub(crate) fn backspace(&mut self) {
        let start = self.previous_boundary();
        if start == self.caret {
            return;
        }
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
        self.touch();
    }

    pub(crate) fn delete(&mut self) {
        let end = self.next_boundary();
        if end == self.caret {
            return;
        }
        self.text.replace_range(self.caret..end, "");
        self.touch();
    }

    /// Replace `start..` with `replacement` and put the caret at the end.
    pub(crate) fn replace_tail(
        &mut self,
        start: usize,
        replacement: &str,
    ) -> Result<(), CommandFull> {
        if !self.text.is_char_boundary(start) || start > self.text.len() {
            return Ok(());
        }
        let kept = self.text[..start].chars().count();
        if kept + replacement.chars().count() > MAX_COMMAND_CHARS {
            return Err(CommandFull);
        }
        self.text.truncate(start);
        self.text.push_str(replacement);
        self.caret = self.text.len();
        self.touch();
        Ok(())
    }

    /// Step to an older entry. The unsent line is kept as the draft.
    pub(crate) fn history_older(&mut self, history: &History) {
        let next = match self.browsing {
            None if history.len() == 0 => return,
            None => {
                self.draft = std::mem::take(&mut self.text);
                history.len() - 1
            }
            Some(0) => return,
            Some(index) => index - 1,
        };
        self.show_history(history, next);
    }

    /// Step to a newer entry, or back to the draft after the newest.
    pub(crate) fn history_newer(&mut self, history: &History) {
        let Some(index) = self.browsing else {
            return;
        };
        if index + 1 >= history.len() {
            self.text = std::mem::take(&mut self.draft);
            self.caret = self.text.len();
            self.revision = self.revision.wrapping_add(1);
            self.browsing = None;
            return;
        }
        self.show_history(history, index + 1);
    }

    fn show_history(&mut self, history: &History, index: usize) {
        if let Some(entry) = history.get(index) {
            self.text = entry.to_owned();
            self.caret = self.text.len();
            self.revision = self.revision.wrapping_add(1);
            self.browsing = Some(index);
            self.preedit = None;
        }
    }
}
