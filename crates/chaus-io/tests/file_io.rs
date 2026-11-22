//! Encoding, newline and conflict-safe file I/O.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chaus_core::eol::Eol;
use chaus_io::{
    check_conflict, encode_for_save, load, save, LoadError, SaveConflict, SaveError, SaveRequest,
};

fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    let mut file = File::create(&path).expect("create fixture");
    file.write_all(bytes).expect("write fixture");
    path
}

#[test]
fn loads_bom_crlf_and_preserves_them() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = write_fixture(directory.path(), "bom.txt", b"\xEF\xBB\xBFone\r\ntwo\r\n");
    let loaded = load(&path).expect("load");
    assert!(loaded.had_utf8_bom);
    assert_eq!(loaded.text, "one\ntwo\n");
    assert_eq!(loaded.eol.len(), 2);
    let bytes = encode_for_save(
        &loaded.eol.apply_to(&loaded.text, loaded.preferred_eol),
        loaded.had_utf8_bom,
    );
    assert_eq!(bytes, b"\xEF\xBB\xBFone\r\ntwo\r\n");
}

#[test]
fn refuses_invalid_utf8_and_utf16_without_touching_the_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    let bad = write_fixture(directory.path(), "bad.txt", &[0x66, 0xFF, 0xFE, 0x67]);
    assert!(matches!(load(&bad), Err(LoadError::InvalidUtf8(_))));
    assert_eq!(fs::read(&bad).expect("read"), vec![0x66, 0xFF, 0xFE, 0x67]);

    let utf16 = write_fixture(directory.path(), "utf16.txt", b"\xFF\xFEh\0i\0");
    assert!(matches!(
        load(&utf16),
        Err(LoadError::UnsupportedUtf16Bom(_))
    ));
}

#[test]
fn detects_binary_nul_data() {
    let directory = tempfile::tempdir().expect("tempdir");
    let binary = write_fixture(directory.path(), "binary.bin", &[0u8; 512]);
    assert!(matches!(load(&binary), Err(LoadError::LikelyBinary(_))));
}

#[test]
fn save_replaces_content_and_returns_a_new_baseline() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = write_fixture(directory.path(), "note.txt", b"old");
    let loaded = load(&path).expect("load");
    let new_meta = save(SaveRequest::new(
        &path,
        &path,
        b"new\r\n",
        false,
        Eol::CrLf,
        Some(&loaded.meta),
    ))
    .expect("save");
    assert_eq!(fs::read(&path).expect("read"), b"new\r\n");
    assert_eq!(new_meta.size, 5);
    check_conflict(&new_meta).expect("freshly saved baseline matches");
}

#[test]
fn save_refuses_to_overwrite_external_changes() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = write_fixture(directory.path(), "note.txt", b"original");
    let loaded = load(&path).expect("load");
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&path, b"modified").expect("external write");
    let error = save(SaveRequest::new(
        &path,
        &path,
        b"ours",
        false,
        Eol::Lf,
        Some(&loaded.meta),
    ))
    .expect_err("must refuse");
    assert!(matches!(
        error,
        SaveError::Conflict(SaveConflict::ChangedOnDisk)
    ));
    assert_eq!(fs::read(&path).expect("read"), b"modified");
}

#[test]
fn save_reports_deleted_target_through_baseline() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = write_fixture(directory.path(), "gone.txt", b"text");
    let loaded = load(&path).expect("load");
    fs::remove_file(&path).expect("remove");
    let error = save(SaveRequest::new(
        &path,
        &path,
        b"text",
        false,
        Eol::Lf,
        Some(&loaded.meta),
    ))
    .expect_err("must refuse");
    assert!(matches!(
        error,
        SaveError::Conflict(SaveConflict::DeletedOnDisk)
    ));
}

#[test]
fn saving_to_a_new_path_works() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("new file.txt");
    save(SaveRequest::new(
        &path,
        &path,
        b"hello",
        false,
        Eol::Lf,
        None,
    ))
    .expect("save new file");
    assert_eq!(fs::read_to_string(&path).expect("read"), "hello");
}
