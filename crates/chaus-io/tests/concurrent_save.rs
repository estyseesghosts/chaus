//! Two writers cannot both accept the same baseline across the commit boundary.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chaus_core::eol::Eol;
use chaus_io::{save, set_pre_replace_gate_for_test, LoadError, SaveError, SaveRequest};

fn request<'a>(
    path: &'a std::path::Path,
    bytes: &'a [u8],
    baseline: Option<&'a chaus_io::FileMeta>,
) -> SaveRequest<'a> {
    SaveRequest::new(path, path, bytes, false, Eol::Lf, baseline)
}

#[test]
fn a_writer_paused_before_commit_detects_the_other_writers_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.txt");
    std::fs::write(&path, "one\n").unwrap();
    let baseline = match chaus_io::load(&path) {
        Ok(loaded) => loaded.meta,
        Err(LoadError::Io { source, .. }) => panic!("fixture load failed: {source}"),
        Err(error) => panic!("fixture load failed: {error}"),
    };

    let (release, gate) = mpsc::channel();
    let (reached_tx, reached_rx) = mpsc::channel();
    set_pre_replace_gate_for_test(Some((gate, Some(reached_tx))));

    let writer_path = path.clone();
    let writer_baseline = baseline.clone();
    let writer = thread::spawn(move || {
        let bytes = b"two\n".to_vec();
        save(request(&writer_path, &bytes, Some(&writer_baseline)))
    });

    reached_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the first writer must reach the pre-commit gate");
    let bytes = b"three\n".to_vec();
    save(request(&path, &bytes, Some(&baseline))).expect("the second writer succeeds");
    release.send(()).expect("the first writer is still gated");
    let result = writer.join().expect("writer thread did not panic");

    assert!(
        matches!(result, Err(SaveError::Conflict(_))),
        "the stale writer must not report success: {result:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "three\n");
    set_pre_replace_gate_for_test(None);
}
