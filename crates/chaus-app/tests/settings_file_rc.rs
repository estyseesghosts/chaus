//! Settings rc persistence is owned by chaus-app, not by document I/O.

use chaus_app::settings_file::write_rc;
use chaus_core::settings::{Settings, ThemeName};

#[test]
fn updates_theme_and_vim_without_losing_comments_or_crlf() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("_chausrc");
    std::fs::write(
        &path,
        "\" my settings\r\nset tabsize=4\r\nset theme=geist-light\r\n",
    )
    .unwrap();

    let mut settings = Settings::default();
    settings.theme = ThemeName::AquaDark;
    settings.vim = false;
    write_rc(&path, &settings).unwrap();

    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(
        updated.starts_with("\" my settings\r\n"),
        "the leading comment must survive: {updated:?}"
    );
    assert!(updated.contains("set tabsize=4\r\n"), "{updated:?}");
    assert!(updated.contains("set theme=aqua-dark\r\n"), "{updated:?}");
    assert!(updated.contains("set novim\r\n"), "{updated:?}");
    assert!(
        !updated.contains("geist-light"),
        "the old value must be replaced: {updated:?}"
    );
}

#[test]
fn creates_the_first_rc_file_with_both_keys() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("_chausrc");
    let mut settings = Settings::default();
    settings.theme = ThemeName::AquaLight;
    settings.vim = true;

    write_rc(&path, &settings).unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("set theme=aqua-light\n"), "{contents:?}");
    assert!(contents.contains("set vim\n"), "{contents:?}");
}

#[test]
fn an_undecodable_file_is_reported_and_left_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("_chausrc");
    std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();

    let error = write_rc(&path, &Settings::default()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xfe, 0x00]);
}
