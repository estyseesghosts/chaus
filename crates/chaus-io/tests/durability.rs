//! A committed rename with a failed directory sync is a distinct outcome.

#[cfg(unix)]
use chaus_core::eol::Eol;
#[cfg(unix)]
use chaus_io::{save, set_fail_dir_sync_for_test, SaveError, SaveRequest};

#[cfg(unix)]
#[test]
fn a_post_rename_directory_sync_failure_is_reported_as_unconfirmed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.txt");
    std::fs::write(&path, "old\n").unwrap();
    set_fail_dir_sync_for_test(true);
    let bytes = b"new\n".to_vec();
    let result = save(SaveRequest::new(&path, &path, &bytes, false, Eol::Lf, None));
    set_fail_dir_sync_for_test(false);

    match result {
        // The committed content is described so the editor can adopt it as
        // the new baseline instead of later reporting its own write as an
        // external change.
        Err(SaveError::DurabilityUnconfirmed { meta, .. }) => assert_eq!(meta.size, 4),
        other => panic!("expected an unconfirmed durability outcome: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "new\n",
        "the replacement is committed even though its directory sync failed"
    );
}
