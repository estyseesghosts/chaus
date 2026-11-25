//! The four-theme and editor-font product contract.

use chaus_core::settings::{Settings, ThemeName};
use chaus_editor::Editor;
use gpui::{AppContext as _, TestAppContext};
use silvestris::{ActiveTheme as _, ThemeId, ThemePreference};

/// The four names in the rc file and Preferences map one-to-one onto the four
/// Silvestris theme ids; no alias silently resolves to the wrong appearance.
#[test]
fn the_four_product_themes_map_onto_four_distinct_silvestris_ids() {
    let expected = [
        (ThemeName::AquaLight, ThemeId::AQUA_LIGHT),
        (ThemeName::AquaDark, ThemeId::AQUA_DARK),
        (ThemeName::GeistLight, ThemeId::GEIST_LIGHT),
        (ThemeName::GeistDark, ThemeId::GEIST_DARK),
    ];
    let mapped: Vec<ThemeId> = ThemeName::ALL
        .into_iter()
        .map(chaus_app::theme_choice::theme_id)
        .collect();
    assert_eq!(mapped.len(), 4);
    for (name, id) in expected {
        assert_eq!(chaus_app::theme_choice::theme_id(name), id);
    }
    for (index, id) in mapped.iter().enumerate() {
        assert!(
            !mapped[..index].contains(id),
            "each product theme must map to a distinct Silvestris id"
        );
    }
}

fn editor(app: &mut TestAppContext, font_family: Option<&str>) -> gpui::Entity<Editor> {
    let mut settings = Settings::default();
    settings.font_family = font_family.map(str::to_owned);
    app.update(|cx| {
        silvestris::test_support::initialize(ThemePreference::explicit(ThemeId::AQUA_LIGHT), cx);
        cx.new(|cx| Editor::new(cx, None, settings))
    })
}

/// Switching family changes the implicit mono face; switching light/dark
/// inside one family does not. An explicit user family survives all four.
#[test]
fn theme_default_mono_follows_the_family_and_an_explicit_family_survives() {
    let mut app = TestAppContext::single();
    let implicit = editor(&mut app, None);
    app.update(|cx| {
        let expected = [
            (ThemeId::AQUA_LIGHT, "Source Code Pro"),
            (ThemeId::AQUA_DARK, "Source Code Pro"),
            (ThemeId::GEIST_LIGHT, "Geist Mono"),
            (ThemeId::GEIST_DARK, "Geist Mono"),
        ];
        for (id, mono) in expected {
            silvestris::set_theme_preference(ThemePreference::explicit(id), cx);
            assert_eq!(
                implicit.read(cx).editor_font_family(cx.theme()),
                mono,
                "theme default follows {id:?}"
            );
        }
    });

    let explicit = editor(&mut app, Some("Chaus Custom Mono"));
    app.update(|cx| {
        for id in ThemeId::ALL {
            silvestris::set_theme_preference(ThemePreference::explicit(id), cx);
            assert_eq!(
                explicit.read(cx).editor_font_family(cx.theme()),
                "Chaus Custom Mono",
                "an explicit editor family must survive {id:?}"
            );
        }
    });
}

/// The Geist UI override is a chrome face; it must not become the editor font
/// when the user has not chosen one.
#[test]
fn the_geist_ui_override_does_not_change_the_editor_mono_face() {
    let mut app = TestAppContext::single();
    let editor = editor(&mut app, None);
    app.update(|cx| {
        silvestris::set_font_overrides(
            ThemeId::GEIST_DARK,
            silvestris::FontOverrides::new().ui(chaus_app::fonts::GEIST_SANS),
            cx,
        );
        silvestris::set_theme_preference(ThemePreference::explicit(ThemeId::GEIST_DARK), cx);
        assert_eq!(cx.theme().type_.ui, chaus_app::fonts::GEIST_SANS);
        assert_eq!(
            editor.read(cx).editor_font_family(cx.theme()),
            "Geist Mono",
            "the UI override is not an editor-font override"
        );
    });
}
