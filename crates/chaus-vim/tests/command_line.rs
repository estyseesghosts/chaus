//! The `:`/`/`/`?` input line: editing, history, bounds and isolation.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{
    CommandPrefix, Engine, ExCommand, Key, Mode, Outcome, MAX_COMMAND_CHARS, MAX_HISTORY,
};

struct Fixture {
    engine: Engine,
    document: Document,
    selection: Selection,
}

impl Fixture {
    fn new(text: &str) -> Self {
        let mut document = Document::new();
        if !text.is_empty() {
            let edit = chaus_core::document::TextEdit::insertion(CharOffset::ZERO, text);
            let transaction = chaus_core::document::EditTransaction::new(
                chaus_core::document::EditCause::Typing,
                edit,
                Selection::caret(CharOffset::ZERO),
                Selection::caret(CharOffset::ZERO),
            );
            document.apply(transaction).unwrap();
        }
        Self {
            engine: Engine::default(),
            document,
            selection: Selection::caret(CharOffset::ZERO),
        }
    }

    fn key(&mut self, key: Key) -> Outcome {
        self.engine
            .handle_key(key, &mut self.document, &mut self.selection)
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.key(Key::Char(character));
        }
    }

    fn line(&self) -> (String, String) {
        let view = self.engine.command_line().expect("command line open");
        (view.before_caret.to_owned(), view.after_caret.to_owned())
    }

    fn submit(&mut self, text: &str) -> Outcome {
        self.key(Key::Char(':'));
        self.type_text(text);
        self.key(Key::Enter)
    }
}

#[test]
fn inline_editing_moves_by_grapheme() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char(':'));
    assert_eq!(fixture.engine.mode(), Mode::CommandLine);
    assert_eq!(
        fixture.engine.command_line().unwrap().prefix,
        CommandPrefix::Ex
    );
    // "e" + combining acute + family emoji (ZWJ sequence).
    fixture
        .engine
        .insert_command_text("we\u{301}\u{1F468}\u{200D}\u{1F469}");
    fixture.key(Key::Left);
    assert_eq!(
        fixture.line(),
        (
            "we\u{301}".to_owned(),
            "\u{1F468}\u{200D}\u{1F469}".to_owned()
        )
    );
    fixture.key(Key::Left);
    assert_eq!(fixture.line().0, "w");
    fixture.key(Key::Delete);
    assert_eq!(
        fixture.line(),
        ("w".to_owned(), "\u{1F468}\u{200D}\u{1F469}".to_owned())
    );
    fixture.key(Key::Home);
    fixture.key(Key::Char('x'));
    fixture.key(Key::End);
    fixture.key(Key::Backspace);
    assert_eq!(fixture.line(), ("xw".to_owned(), String::new()));
}

#[test]
fn escape_and_command_text_never_touch_the_document() {
    let mut fixture = Fixture::new("hello\n");
    let revision = fixture.document.revision();
    let selection = fixture.selection;
    fixture.key(Key::Char(':'));
    fixture.type_text("dd x i abc");
    fixture.key(Key::Escape);
    assert_eq!(fixture.engine.mode(), Mode::Normal);
    assert!(fixture.engine.command_line().is_none());
    assert_eq!(fixture.document.revision(), revision);
    assert_eq!(fixture.document.text(), "hello\n");
    assert_eq!(fixture.selection, selection);
}

#[test]
fn overflow_is_refused_with_a_message_not_dropped_silently() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char(':'));
    let long = "a".repeat(MAX_COMMAND_CHARS);
    assert_eq!(fixture.engine.insert_command_text(&long), Outcome::Handled);
    match fixture.key(Key::Char('b')) {
        Outcome::Message(message) => assert!(message.contains("limited"), "{message}"),
        other => panic!("expected a message, got {other:?}"),
    }
    assert_eq!(fixture.line().0.chars().count(), MAX_COMMAND_CHARS);
}

#[test]
fn pasted_text_stops_at_a_line_break() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char(':'));
    fixture.engine.insert_command_text("w a.txt\nq!\n");
    assert_eq!(fixture.line().0, "w a.txt");
}

#[test]
fn submitted_lines_become_typed_commands_or_messages() {
    let mut fixture = Fixture::new("");
    assert_eq!(
        fixture.submit("w! \"a b\""),
        Outcome::Ex(ExCommand::Write {
            force: true,
            path: Some("a b".into())
        })
    );
    match fixture.submit("frob") {
        Outcome::Message(message) => assert!(message.contains("frob"), "{message}"),
        other => panic!("unknown commands must not execute: {other:?}"),
    }
    assert_eq!(fixture.engine.mode(), Mode::Normal);
}

#[test]
fn history_walks_back_and_restores_the_draft() {
    let mut fixture = Fixture::new("");
    fixture.submit("set ts=4");
    fixture.submit("bogus");
    fixture.submit("bogus");
    fixture.submit("");
    fixture.key(Key::Char(':'));
    fixture.type_text("dra");
    fixture.key(Key::Up);
    assert_eq!(fixture.line().0, "bogus", "invalid lines are kept, once");
    fixture.key(Key::Up);
    assert_eq!(fixture.line().0, "set ts=4");
    fixture.key(Key::Up);
    assert_eq!(fixture.line().0, "set ts=4", "oldest entry stays");
    fixture.key(Key::Down);
    fixture.key(Key::Down);
    assert_eq!(fixture.line().0, "dra", "the unsent draft comes back");
}

#[test]
fn search_history_is_separate_from_ex_history() {
    let mut fixture = Fixture::new("one two one\n");
    fixture.key(Key::Char('/'));
    fixture.type_text("two");
    fixture.key(Key::Enter);
    assert_eq!(fixture.selection.head, CharOffset(4));
    fixture.submit("set ts=2");
    fixture.key(Key::Char('/'));
    fixture.key(Key::Up);
    assert_eq!(fixture.line().0, "two");
    fixture.key(Key::Escape);
    fixture.key(Key::Char(':'));
    fixture.key(Key::Up);
    assert_eq!(fixture.line().0, "set ts=2");
    fixture.key(Key::Escape);
    // n still repeats the search after browsing history.
    fixture.selection = Selection::caret(CharOffset::ZERO);
    fixture.key(Key::Char('n'));
    assert_eq!(fixture.selection.head, CharOffset(4));
}

#[test]
fn history_is_bounded() {
    let mut fixture = Fixture::new("");
    for index in 0..MAX_HISTORY + 20 {
        fixture.submit(&format!("{}", index + 1));
    }
    fixture.key(Key::Char(':'));
    for _ in 0..MAX_HISTORY + 50 {
        fixture.key(Key::Up);
    }
    assert_eq!(
        fixture.line().0,
        "21",
        "only the newest {MAX_HISTORY} remain"
    );
}

#[test]
fn navigation_keys_stay_motions_outside_the_command_line() {
    let mut fixture = Fixture::new("abc\n");
    fixture.key(Key::Right);
    assert_eq!(fixture.selection.head, CharOffset(1));
    fixture.key(Key::Delete);
    assert_eq!(fixture.document.text(), "ac\n");
}

#[test]
fn tab_requests_completion_and_applies_only_to_the_same_line() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char(':'));
    fixture.type_text("e my\\ do");
    assert_eq!(fixture.key(Key::Tab), Outcome::Complete);
    let request = fixture.engine.completion_request().unwrap();
    assert_eq!(request.typed, "my do");
    assert_eq!(
        fixture
            .engine
            .apply_completion(request.revision, "my docs/", false),
        None
    );
    assert_eq!(fixture.line().0, "e \"my docs/");
    let request = fixture.engine.completion_request().unwrap();
    fixture.type_text("x");
    assert_eq!(
        fixture
            .engine
            .apply_completion(request.revision, "my docs/stale.txt", true),
        None
    );
    assert_eq!(
        fixture.line().0,
        "e \"my docs/x",
        "a stale answer is dropped"
    );
    let request = fixture.engine.completion_request().unwrap();
    fixture
        .engine
        .apply_completion(request.revision, "my docs/x.txt", true);
    assert_eq!(fixture.line().0, "e \"my docs/x.txt\"");
    assert_eq!(
        fixture.key(Key::Enter),
        Outcome::Ex(ExCommand::Edit {
            force: false,
            path: Some("my docs/x.txt".into())
        })
    );
}

#[test]
fn tab_in_search_or_non_path_commands_does_not_complete() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char('/'));
    assert_eq!(fixture.key(Key::Tab), Outcome::Handled);
    fixture.key(Key::Escape);
    fixture.key(Key::Char(':'));
    fixture.type_text("set ts");
    fixture.key(Key::Tab);
    assert_eq!(fixture.engine.completion_request(), None);
}

#[test]
fn backspace_on_an_empty_line_leaves_command_mode() {
    let mut fixture = Fixture::new("");
    fixture.key(Key::Char(':'));
    fixture.key(Key::Backspace);
    assert_eq!(fixture.engine.mode(), Mode::Normal);
}
