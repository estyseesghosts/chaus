//! Opaque completion identity: command-line session, text revision and the
//! order of repeated Tab requests. A result from any other request is inert.

use chaus_core::document::Document;
use chaus_core::position::{CharOffset, Selection};
use chaus_vim::{CompletionRequest, Engine, Key, Mode, Outcome};

struct Fixture {
    engine: Engine,
    document: Document,
    selection: Selection,
}

impl Fixture {
    fn new() -> Self {
        Self {
            engine: Engine::default(),
            document: Document::new(),
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

    fn open(&mut self, text: &str) -> CompletionRequest {
        self.key(Key::Char(':'));
        assert_eq!(self.engine.mode(), Mode::CommandLine);
        self.type_text(text);
        self.engine
            .completion_request()
            .expect("a path argument at the end of an Ex line")
    }

    fn line(&self) -> Option<String> {
        self.engine
            .command_line()
            .map(|view| format!("{}{}", view.before_caret, view.after_caret))
    }
}

#[test]
fn a_new_line_with_identical_text_and_revision_rejects_the_old_request() {
    let mut fixture = Fixture::new();
    let old = fixture.open("e docs/x");

    fixture.key(Key::Escape);
    assert_eq!(fixture.line(), None);
    let fresh = fixture.open("e docs/x");
    assert_eq!(old.typed, fresh.typed);
    assert_ne!(
        old.token, fresh.token,
        "a new line must not reuse the previous session identity"
    );

    assert!(!fixture.engine.is_completion_current(&old.token));
    assert_eq!(
        fixture
            .engine
            .apply_completion(&old.token, "docs/stale.txt", true),
        None,
        "a stale result has no message and no edit"
    );
    assert_eq!(fixture.line().as_deref(), Some("e docs/x"));

    assert!(fixture.engine.is_completion_current(&fresh.token));
    assert_eq!(
        fixture
            .engine
            .apply_completion(&fresh.token, "docs/x.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e docs/x.txt"));
}

#[test]
fn editing_the_line_invalidates_the_request_even_when_text_returns() {
    let mut fixture = Fixture::new();
    let request = fixture.open("e a");
    fixture.type_text("b");
    assert!(!fixture.engine.is_completion_current(&request.token));
    assert_eq!(
        fixture
            .engine
            .apply_completion(&request.token, "ab/stale.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e ab"));

    let current = fixture
        .engine
        .completion_request()
        .expect("current request");
    assert_ne!(current.token, request.token);
    assert_eq!(
        fixture
            .engine
            .apply_completion(&current.token, "ab/file.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e ab/file.txt"));
}

#[test]
fn repeated_requests_without_text_change_are_explicitly_ordered() {
    let mut fixture = Fixture::new();
    let first = fixture.open("e a");
    let second = fixture.engine.completion_request().expect("second request");
    assert_ne!(
        first.token, second.token,
        "a second Tab is newer even though the line did not change"
    );

    assert_eq!(
        fixture
            .engine
            .apply_completion(&first.token, "first.txt", true),
        None,
        "the older result is dropped"
    );
    assert_eq!(fixture.line().as_deref(), Some("e a"));

    assert_eq!(
        fixture
            .engine
            .apply_completion(&second.token, "second.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e second.txt"));
}

#[test]
fn cancelling_or_resetting_the_engine_discards_the_request() {
    let mut fixture = Fixture::new();
    let request = fixture.open("e a");
    fixture.engine.cancel_prefix();
    assert!(!fixture.engine.is_completion_current(&request.token));
    assert_eq!(
        fixture
            .engine
            .apply_completion(&request.token, "a.txt", true),
        None
    );
    assert_eq!(fixture.line(), None);

    let request = fixture.open("e b");
    fixture.engine.reset();
    assert!(!fixture.engine.is_completion_current(&request.token));
    assert_eq!(
        fixture
            .engine
            .apply_completion(&request.token, "b.txt", true),
        None
    );
    assert_eq!(fixture.line(), None);
}

#[test]
fn generations_belong_to_each_engine_not_to_the_process() {
    let mut first = Fixture::new();
    let mut second = Fixture::new();

    let first_request = first.open("e docs/x");
    first.key(Key::Escape);
    let first_next = first.open("e docs/x");
    let second_request = second.open("e docs/x");

    assert_eq!(
        first_request.token, second_request.token,
        "two independent engines may start at the same generation"
    );
    assert_ne!(first_next.token, second_request.token);
    assert!(!first.engine.is_completion_current(&second_request.token));
    assert_eq!(
        first
            .engine
            .apply_completion(&second_request.token, "other.txt", true),
        None
    );
    assert_eq!(first.line().as_deref(), Some("e docs/x"));
    assert!(second.engine.is_completion_current(&second_request.token));
}

#[test]
fn unicode_quoted_and_home_paths_keep_their_request_identity() {
    let mut fixture = Fixture::new();
    let request = fixture.open("e \"café/na");
    assert!(request.quoted);
    assert_eq!(request.typed, "café/na");
    assert_eq!(
        fixture
            .engine
            .apply_completion(&request.token, "café/naïve.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e \"café/naïve.txt\""));

    let mut fixture = Fixture::new();
    let request = fixture.open("e ~/caf");
    assert_eq!(request.typed, "~/caf");
    assert_eq!(
        fixture
            .engine
            .apply_completion(&request.token, "~/café.txt", true),
        None
    );
    assert_eq!(fixture.line().as_deref(), Some("e ~/café.txt"));
}
