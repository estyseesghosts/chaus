//! Ex writes without `!` must never replace an existing path.

use chaus_core::eol::Eol;
use chaus_io::{save, SaveError, SaveRequest};

#[test]
fn an_existing_target_is_refused_and_left_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("taken.txt");
    std::fs::write(&path, "original\n").unwrap();
    let bytes = b"replacement\n".to_vec();
    let result =
        save(SaveRequest::new(&path, &path, &bytes, false, Eol::Lf, None).refusing_existing());
    assert!(
        matches!(result, Err(SaveError::Exists { .. })),
        "got {result:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");
    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".chaus-save-")
        })
        .collect();
    assert!(leftovers.is_empty(), "no temporary file is left behind");
}

#[test]
fn a_missing_target_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fresh.txt");
    let bytes = b"new\n".to_vec();
    let meta =
        save(SaveRequest::new(&path, &path, &bytes, false, Eol::Lf, None).refusing_existing())
            .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    assert_eq!(meta.size, 4);
}

#[test]
fn without_the_flag_an_existing_target_is_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("taken.txt");
    std::fs::write(&path, "original\n").unwrap();
    let bytes = b"replacement\n".to_vec();
    save(SaveRequest::new(&path, &path, &bytes, false, Eol::Lf, None)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement\n");
}
