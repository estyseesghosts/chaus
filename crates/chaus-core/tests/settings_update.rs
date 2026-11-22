use chaus_core::settings::{update_rc, Settings, ThemeName};

#[test]
fn updates_theme_and_vim_without_losing_comments_or_other_settings() {
    let source = "\" my settings\nset wrap\nset theme=aqua-light vim\n";
    let mut settings = Settings::default();
    settings.theme = ThemeName::GeistLight;
    settings.vim = true;
    let updated = update_rc(source, &settings);

    assert!(updated.starts_with("\" my settings\nset wrap\n"));
    assert!(
        updated.contains("set theme=aqua-light vim"),
        "mixed lines stay intact"
    );
    assert!(updated.contains("\nset theme=geist-light\n"));
    assert!(updated.contains("\nset vim\n"));
    assert!(!updated.contains("set novim"));
}

#[test]
fn rewrites_standalone_lines_in_place() {
    let source = "set wrap\nset theme=geist-dark\nset vim\n";
    let mut settings = Settings::default();
    settings.theme = ThemeName::AquaDark;
    settings.vim = false;
    let updated = update_rc(source, &settings);
    assert_eq!(updated, "set wrap\nset theme=aqua-dark\nset novim\n");
}

#[test]
fn preserves_crlf_and_creates_missing_lines() {
    let source = "set wrap\r\n";
    let mut settings = Settings::default();
    settings.theme = ThemeName::GeistDark;
    let updated = update_rc(source, &settings);
    assert_eq!(updated, "set wrap\r\nset theme=geist-dark\r\nset novim\r\n");
}
