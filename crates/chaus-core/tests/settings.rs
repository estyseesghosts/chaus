//! Rc-file parsing, defaults and diagnostics.

use chaus_core::settings::{parse_rc, LineRole, SettingKey, ThemeName, WrapMode};

const SAMPLE: &str = r#"" chaus settings
set theme=geist-dark
set font=GeistMono
set fontsize=14
set lineheight=1.4
set wrap
set tabstop=4
set expandtab
set novim
let mapleader = " "
nnoremap <leader>w :w<CR>
"#;

#[test]
fn parses_the_documented_subset() {
    let parsed = parse_rc(SAMPLE);
    assert!(
        parsed.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        parsed.diagnostics
    );
    assert_eq!(parsed.settings.theme, ThemeName::GeistDark);
    assert_eq!(
        parsed.settings.font_family.as_deref(),
        Some("GeistMono"),
        "an explicit `set font` overrides the theme face"
    );
    assert_eq!(parsed.settings.font_size, 14.0);
    assert_eq!(parsed.settings.tab_stop, 4);
    assert!(parsed.settings.expand_tabs);
    assert!(!parsed.settings.vim);
    assert_eq!(parsed.settings.mapleader, ' ');
    assert_eq!(parsed.settings.normal_mappings.len(), 1);
    assert_eq!(parsed.lines.len(), 11);
    assert_eq!(parsed.lines[0].role, LineRole::Comment);
    assert_eq!(parsed.lines[1].role, LineRole::Setting(SettingKey::Theme));
}

#[test]
fn invalid_lines_do_not_discard_valid_settings() {
    let parsed =
        parse_rc("set fontsize=16\nsource ~/.vimrc\nset theme=nope\nset wrap\nset tabstop=0\n");
    assert_eq!(parsed.settings.font_size, 16.0);
    assert_eq!(
        parsed.settings.font_family, None,
        "without `set font`, the active theme supplies the face"
    );
    assert_eq!(parsed.settings.wrap, WrapMode::Word);
    assert_eq!(parsed.settings.tab_stop, 4, "invalid value keeps default");
    assert_eq!(parsed.settings.theme, ThemeName::GeistDark);
    assert_eq!(parsed.diagnostics.len(), 3);
    assert_eq!(parsed.diagnostics[0].line, 2);
}

#[test]
fn boolean_negation_works_for_every_boolean() {
    let parsed = parse_rc("set novim\nset nowrap\nset noexpandtab\n");
    assert!(!parsed.settings.vim);
    assert_eq!(parsed.settings.wrap, WrapMode::None);
    assert!(!parsed.settings.expand_tabs);
    let parsed = parse_rc("set vim\nset wrap\nset expandtab\n");
    assert!(parsed.settings.vim);
    assert_eq!(parsed.settings.wrap, WrapMode::Word);
    assert!(parsed.settings.expand_tabs);
}

#[test]
fn rejects_malformed_mapleader_and_mappings() {
    let parsed =
        parse_rc("let mapleader = \"ab\"\nnnoremap\nnnoremap a\nnnoremap abcdefghijklmnopq x\n");
    assert_eq!(parsed.diagnostics.len(), 4);
    assert!(parsed.settings.normal_mappings.is_empty());
}

#[test]
fn comments_and_blank_lines_are_preserved_verbatim() {
    let source = "\n\" a comment with set tabstop=99\n   \n";
    let parsed = parse_rc(source);
    assert_eq!(parsed.settings.tab_stop, 4);
    let rebuilt: Vec<&str> = parsed.lines.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(rebuilt, vec!["", "\" a comment with set tabstop=99", "   "]);
}
