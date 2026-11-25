//! Font validation keeps the last valid explicit family.

use chaus_app::font_validation::{decide_font_family, FontDecision};

fn installed<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    move |name| names.iter().any(|item| item.eq_ignore_ascii_case(name))
}

#[test]
fn an_empty_family_is_an_explicit_reset() {
    assert_eq!(
        decide_font_family("  ", Some("Geist Mono"), installed(&[]), "Source Code Pro"),
        FontDecision::Reset
    );
}

#[test]
fn an_installed_family_is_accepted() {
    let available = ["Geist Mono", "JetBrains Mono"];
    assert_eq!(
        decide_font_family(
            " geist mono ",
            None,
            installed(&available),
            "Source Code Pro"
        ),
        FontDecision::Accept("geist mono".to_string())
    );
}

#[test]
fn a_typo_keeps_the_previous_explicit_family() {
    let available = ["Geist Mono"];
    let decision = decide_font_family(
        "Geist Monno",
        Some("JetBrains Mono"),
        installed(&available),
        "Source Code Pro",
    );
    match decision {
        FontDecision::Keep { retained, warning } => {
            assert_eq!(retained.as_deref(), Some("JetBrains Mono"));
            assert!(warning.contains("Geist Monno"));
            assert!(warning.contains("JetBrains Mono"));
        }
        other => panic!("expected Keep, got {other:?}"),
    }
}

#[test]
fn a_typo_with_no_explicit_family_keeps_the_theme_default() {
    let decision = decide_font_family("Nope", None, installed(&[]), "Source Code Pro");
    match decision {
        FontDecision::Keep { retained, warning } => {
            assert_eq!(retained, None);
            assert!(warning.contains("Source Code Pro"));
        }
        other => panic!("expected Keep, got {other:?}"),
    }
}
