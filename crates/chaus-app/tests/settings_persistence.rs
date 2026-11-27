//! Deterministic ordering tests for the one settings persistence owner.
//!
//! Every disk transition is explicit here: no timers, sleeps or background
//! threads. Generations stand in for in-flight work so stale completions can
//! be delivered on purpose.

use std::io;
use std::path::PathBuf;

use chaus_app::settings_file::SettingsRead;
use chaus_app::settings_store::{
    ReloadOutcome, ReloadStart, SettingsPersistence, ShutdownStart, WriteCompletion, WriteKind,
};
use chaus_core::settings::{Settings, ThemeName};

fn settings(tab_stop: usize) -> Settings {
    let mut value = Settings::default();
    value.tab_stop = tab_stop;
    value
}

fn disk(theme: ThemeName, tab_stop: usize) -> SettingsRead {
    let mut value = Settings::default();
    value.theme = theme;
    value.tab_stop = tab_stop;
    SettingsRead::new(value, Vec::new(), Some(PathBuf::from("rc")))
}

#[test]
fn shutdown_waits_for_an_old_write_and_commits_only_the_newest_snapshot() {
    let mut store = SettingsPersistence::new(Some(PathBuf::from("rc")), Vec::new());
    let old = store.enqueue(settings(2)).expect("first change accepted");
    let in_flight = store.take_write().expect("first write starts");
    assert_eq!(in_flight.generation, old);
    let newer = store.enqueue(settings(8)).expect("second change accepted");
    assert!(newer > old);
    assert!(store.take_write().is_none(), "one write at a time");

    assert_eq!(
        store.begin_shutdown(),
        ShutdownStart::WaitForWrite { generation: old },
        "shutdown must wait for the older writer"
    );
    assert!(
        store.enqueue(settings(64)).is_none(),
        "no request is accepted after shutdown begins"
    );
    assert!(
        store.take_write().is_none(),
        "no new write may start while the older one is in flight"
    );

    match store.complete_write(old, Ok(())) {
        WriteCompletion::BeginFinalWrite(request) => {
            assert_eq!(request.kind, WriteKind::Final);
            assert_eq!(
                request.settings.tab_stop, 8,
                "newest accepted snapshot wins"
            );
            store.complete_write(request.generation, Ok(()));
        }
        other => panic!("expected the final write, got {other:?}"),
    }
    assert!(store.is_finished());
    assert_eq!(store.latest().map(|value| value.tab_stop), Some(8));
}

#[test]
fn a_late_old_write_cannot_override_the_final_snapshot_or_its_status() {
    let mut store = SettingsPersistence::new(Some(PathBuf::from("rc")), Vec::new());
    let old = store.enqueue(settings(2)).expect("first change accepted");
    store.take_write().expect("first write starts");
    store.enqueue(settings(8)).expect("second change accepted");
    store.begin_shutdown();

    // The old writer fails, then "finishes" again. Neither result belongs to
    // the current generation, so neither can publish an error or rewind the
    // snapshot that shutdown will persist.
    assert!(matches!(
        store.complete_write(old, Err(io::Error::other("disk full"))),
        WriteCompletion::BeginFinalWrite(_)
    ));
    assert!(
        store.visible_error().is_none(),
        "a pre-barrier failure must not leak into shutdown status"
    );
    store.complete_write(old, Ok(()));
    assert_eq!(store.latest().map(|value| value.tab_stop), Some(8));

    let final_write = store
        .take_write()
        .expect("the final write is released once");
    assert_eq!(final_write.generation, 3);
    assert_eq!(final_write.kind, WriteKind::Final);
    assert_eq!(final_write.settings.tab_stop, 8);
    assert_eq!(
        store.complete_write(final_write.generation, Ok(())),
        WriteCompletion::ShutdownFinished
    );
    assert!(store.visible_error().is_none());
}

#[test]
fn reload_abandons_pending_writes_and_waits_for_the_running_one() {
    let mut store = SettingsPersistence::new(Some(PathBuf::from("rc")), Vec::new());
    let old = store.enqueue(settings(2)).expect("first change accepted");
    let in_flight = store.take_write().expect("first write starts");
    assert_eq!(in_flight.generation, old);
    store.enqueue(settings(4)).expect("pending change");

    let ReloadStart::WaitForWrite { epoch, generation } = store.begin_reload() else {
        panic!("reload must wait for the running writer");
    };
    assert_eq!(generation, old);
    assert!(
        store.take_write().is_none(),
        "pending old writes are abandoned"
    );
    assert!(
        store.visible_error().is_none(),
        "disk becomes the source of truth at the reload barrier"
    );

    assert_eq!(
        store.complete_write(old, Err(io::Error::other("stale failure"))),
        WriteCompletion::BeginRead { epoch }
    );
    assert!(
        store.visible_error().is_none(),
        "the old failure cannot replace the reload's status"
    );
    assert!(matches!(
        store.complete_reload(epoch, Ok(disk(ThemeName::AquaDark, 6))),
        ReloadOutcome::Accepted(_)
    ));
    assert_eq!(store.latest().map(|value| value.tab_stop), Some(6));
    assert!(
        store.take_write().is_none(),
        "no abandoned write survives reload"
    );
}

#[test]
fn a_newer_live_change_rejects_the_reload_result() {
    let mut store = SettingsPersistence::new(Some(PathBuf::from("rc")), Vec::new());
    let ReloadStart::ReadNow { epoch } = store.begin_reload() else {
        panic!("an idle store reads immediately");
    };

    store
        .enqueue(settings(9))
        .expect("user changed a preference");
    assert!(
        matches!(
            store.complete_reload(epoch, Ok(disk(ThemeName::AquaLight, 4))),
            ReloadOutcome::Rejected(_)
        ),
        "an older reload result must not overwrite newer live settings"
    );
    assert_eq!(store.latest().map(|value| value.tab_stop), Some(9));

    let write = store
        .take_write()
        .expect("the newer change is still written");
    assert_eq!(write.settings.tab_stop, 9);
    assert_eq!(write.kind, WriteKind::Normal);
    assert_eq!(
        store.complete_write(write.generation, Err(io::Error::other("read-only"))),
        WriteCompletion::Notify
    );
    assert!(store
        .visible_error()
        .is_some_and(|message| message.contains("read-only")));
}

#[test]
fn a_failed_reload_is_reported_without_discarding_the_live_value() {
    let mut store = SettingsPersistence::new(Some(PathBuf::from("rc")), Vec::new());
    let ReloadStart::ReadNow { epoch } = store.begin_reload() else {
        panic!("an idle store reads immediately");
    };
    let outcome = store.complete_reload(epoch, Err("rc is unreadable".to_owned()));
    assert_eq!(
        outcome,
        ReloadOutcome::Rejected("rc is unreadable".to_owned())
    );
    assert_eq!(store.diagnostics(), &["rc is unreadable".to_owned()][..]);
    assert!(store.latest().is_none(), "no live setting was reverted");
}
