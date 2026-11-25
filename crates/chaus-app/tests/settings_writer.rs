//! Serialized, coalescing persistence for the rc file.

use std::io;

use chaus_app::settings_writer::WriteQueue;
use chaus_core::settings::Settings;

fn settings(tab_stop: usize) -> Settings {
    let mut value = Settings::default();
    value.tab_stop = tab_stop;
    value
}

#[test]
fn rapid_changes_coalesce_to_the_latest_snapshot() {
    let mut queue = WriteQueue::default();
    queue.enqueue(settings(2));
    queue.enqueue(settings(4));
    queue.enqueue(settings(8));

    let (generation, value) = queue.take().expect("one write should be started");
    assert_eq!(generation, 3);
    assert_eq!(value.tab_stop, 8, "only the newest snapshot is written");
    assert!(!queue.has_pending());
    assert!(queue.has_in_flight());
    assert!(
        queue.take().is_none(),
        "a second writer must not start while one is in flight"
    );
}

#[test]
fn a_new_change_while_writing_is_written_after_the_first_completes() {
    let mut queue = WriteQueue::default();
    queue.enqueue(settings(2));
    let (first, _) = queue.take().unwrap();
    queue.enqueue(settings(4));

    assert!(queue.has_pending());
    assert!(queue.take().is_none());
    queue.finish(first, Ok(()));

    let (second, value) = queue.take().expect("the newer snapshot follows");
    assert_eq!(second, 2);
    assert_eq!(value.tab_stop, 4);
}

#[test]
fn an_old_failure_cannot_clear_a_newer_success() {
    let mut queue = WriteQueue::default();
    queue.enqueue(settings(2));
    let (old, _) = queue.take().unwrap();
    queue.finish(old, Err(io::Error::other("disk full")));
    assert!(queue
        .visible_error()
        .is_some_and(|message| message.contains("disk full")));

    queue.enqueue(settings(4));
    let (new, _) = queue.take().unwrap();
    queue.finish(new, Ok(()));
    assert_eq!(
        queue.visible_error(),
        None,
        "a newer success clears the error"
    );
}

#[test]
fn a_stale_duplicate_completion_cannot_clear_a_newer_error() {
    let mut queue = WriteQueue::default();
    queue.enqueue(settings(2));
    let (old, _) = queue.take().unwrap();
    queue.enqueue(settings(4));
    queue.finish(old, Ok(()));

    let (new, _) = queue.take().expect("the newer snapshot follows");
    queue.finish(new, Err(io::Error::other("permission denied")));

    // A late duplicate completion for the old generation must be ignored: it
    // is no longer in flight, so it cannot disturb the newer error.
    queue.finish(old, Ok(()));
    assert!(queue
        .visible_error()
        .is_some_and(|message| message.contains("permission denied")));
}

#[test]
fn the_latest_snapshot_is_available_for_the_exit_flush() {
    let mut queue = WriteQueue::default();
    queue.enqueue(settings(2));
    let (first, _) = queue.take().unwrap();
    queue.enqueue(settings(8));
    assert_eq!(queue.latest().map(|value| value.tab_stop), Some(8));
    queue.finish(first, Ok(()));
    assert_eq!(queue.latest().map(|value| value.tab_stop), Some(8));
}
