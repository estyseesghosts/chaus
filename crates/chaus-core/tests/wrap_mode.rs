//! `wrap=word|char|none` parsing and legacy boolean compatibility.

use chaus_core::settings::{parse_rc, update_rc, LineRole, SettingKey, Settings, WrapMode};

#[test]
fn word_wrapping_is_the_default() {
    assert_eq!(Settings::default().wrap, WrapMode::Word);
}

#[test]
fn the_three_modes_parse_from_key_values() {
    assert_eq!(parse_rc("set wrap=word\n").settings.wrap, WrapMode::Word);
    assert_eq!(
        parse_rc("set wrap=char\n").settings.wrap,
        WrapMode::Character
    );
    assert_eq!(
        parse_rc("set wrap=character\n").settings.wrap,
        WrapMode::Character
    );
    assert_eq!(parse_rc("set wrap=none\n").settings.wrap, WrapMode::None);
    assert_eq!(parse_rc("set wrap=nowrap\n").settings.wrap, WrapMode::None);
}

#[test]
fn legacy_booleans_keep_their_meaning() {
    assert_eq!(parse_rc("set wrap\n").settings.wrap, WrapMode::Word);
    assert_eq!(parse_rc("set nowrap\n").settings.wrap, WrapMode::None);
    assert_eq!(
        parse_rc("set wrap\nset nowrap\nset wrap\n").settings.wrap,
        WrapMode::Word,
        "the last setting wins"
    );
}

#[test]
fn an_unknown_mode_is_reported_and_keeps_the_previous_value() {
    let parsed = parse_rc("set wrap=char\nset wrap=sideways\n");
    assert_eq!(parsed.settings.wrap, WrapMode::Character);
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].line, 2);
    assert!(parsed.diagnostics[0].message.contains("sideways"));
}

#[test]
fn wrap_lines_are_recognized_as_settings() {
    let parsed = parse_rc("set wrap=word\nset wrap=char\n");
    assert_eq!(parsed.lines[0].role, LineRole::Setting(SettingKey::Wrap));
    assert_eq!(parsed.lines[1].role, LineRole::Setting(SettingKey::Wrap));
}

#[test]
fn updating_unrelated_settings_does_not_corrupt_wrap_lines() {
    let source = "\" my settings\nset wrap=char\nset theme=geist-dark\n";
    let mut settings = parse_rc(source).settings;
    settings.theme = chaus_core::settings::ThemeName::AquaDark;
    let updated = update_rc(source, &settings);
    assert!(updated.contains("set wrap=char\n"));
    assert!(updated.contains("set theme=aqua-dark"));
}
