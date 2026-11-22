//! Preferences must round-trip through the rc file without losing user lines.

use chaus_core::settings::{parse_rc, update_rc, Settings, ThemeName, WrapMode};

#[test]
fn every_preference_survives_a_save_and_reload() {
    let source = "\" keep me\nset tabstop=8\nset theme=geist-dark\nset wrap\n\
                  source ~/.vimrc\nnnoremap <leader>w :w<CR>\nset font=OldMono\n";
    let mut settings = Settings::default();
    settings.theme = ThemeName::AquaDark;
    settings.font_family = Some("Geist Mono".to_string());
    settings.font_size = 16.0;
    settings.line_height = 1.6;
    settings.wrap = WrapMode::Character;
    settings.tab_stop = 2;
    settings.expand_tabs = false;
    settings.vim = true;
    let updated = update_rc(source, &settings);
    assert!(updated.contains("\" keep me\n"));
    assert!(updated.contains("source ~/.vimrc\n"));
    assert!(updated.contains("nnoremap <leader>w :w<CR>\n"));
    assert!(updated.contains("set font=\"Geist Mono\"\n"));
    assert!(updated.contains("set fontsize=16\n"));
    assert!(updated.contains("set lineheight=1.6\n"));
    assert!(updated.contains("set tabstop=2\n"));
    assert!(updated.contains("set wrap=char\n"));
    assert!(updated.contains("set noexpandtab\n"));
    assert!(updated.contains("set vim\n"));

    let reloaded = parse_rc(&updated).settings;
    assert_eq!(reloaded.theme, settings.theme);
    assert_eq!(reloaded.font_family, settings.font_family);
    assert_eq!(reloaded.font_size, settings.font_size);
    assert_eq!(reloaded.line_height, settings.line_height);
    assert_eq!(reloaded.wrap, settings.wrap);
    assert_eq!(reloaded.tab_stop, settings.tab_stop);
    assert!(!reloaded.expand_tabs);
    assert!(reloaded.vim);
    assert_eq!(
        reloaded.normal_mappings,
        parse_rc(source).settings.normal_mappings,
        "rc-only mappings survive the rewrite"
    );
}

#[test]
fn saving_twice_is_idempotent() {
    let mut settings = Settings::default();
    settings.wrap = WrapMode::None;
    settings.tab_stop = 3;
    settings.font_family = Some("Geist Mono".to_string());
    let once = update_rc("\" header\n", &settings);
    let twice = update_rc(&once, &settings);
    assert_eq!(once, twice);
}

#[test]
fn legacy_wrap_lines_are_preserved_or_migrated_only_when_the_mode_changes() {
    let mut settings = Settings::default();
    assert_eq!(
        update_rc("set wrap\n", &settings),
        "set wrap\nset theme=geist-dark\nset novim\n"
    );
    settings.wrap = WrapMode::None;
    let updated = update_rc("set nowrap\n", &settings);
    assert!(updated.starts_with("set nowrap\n"), "legacy nowrap stays");
    settings.wrap = WrapMode::Character;
    let updated = update_rc("set wrap\nset nowrap\n", &settings);
    assert_eq!(updated.matches("set wrap=char").count(), 2);
    assert!(!updated.contains("\nset wrap\n"));
}

#[test]
fn clearing_the_font_removes_a_stale_override() {
    let mut settings = Settings::default();
    settings.font_family = None;
    let updated = update_rc("\" face\nset font=OldMono\nset tabstop=4\n", &settings);
    assert!(!updated.contains("set font="));
    assert!(updated.contains("\" face\n"));
    assert!(updated.contains("set tabstop=4\n"));
}

#[test]
fn invalid_external_values_are_reported_for_the_reload_ui() {
    let parsed = parse_rc("set wrap=sideways\nset fontsize=nope\nset theme=nope\n");
    assert_eq!(parsed.diagnostics.len(), 3);
    assert_eq!(parsed.settings.wrap, WrapMode::Word);
    assert_eq!(parsed.settings.font_size, 14.0);
    for diagnostic in &parsed.diagnostics {
        assert!(diagnostic.line >= 1);
        assert!(!diagnostic.message.is_empty());
    }
}
