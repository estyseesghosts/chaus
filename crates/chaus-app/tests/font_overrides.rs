//! Bundled Geist Sans and the per-theme override wiring the app installs.

use gpui::TestAppContext;
use silvestris::{ActiveTheme as _, FontOverrides, ThemeId, ThemePreference};

fn install(app: &mut TestAppContext, theme: ThemeId) {
    app.update(|cx| {
        silvestris::test_support::initialize(ThemePreference::explicit(theme), cx);
        for id in [ThemeId::GEIST_LIGHT, ThemeId::GEIST_DARK] {
            silvestris::set_font_overrides(
                id,
                FontOverrides::new().ui(chaus_app::fonts::GEIST_SANS),
                cx,
            );
        }
    });
}

#[test]
fn geist_themes_use_geist_sans_for_ui_and_geist_mono_for_documents() {
    let mut app = TestAppContext::single();
    install(&mut app, ThemeId::GEIST_DARK);
    app.update(|cx| {
        let theme = cx.theme();
        assert_eq!(theme.type_.ui, "Geist");
        assert_eq!(theme.type_.mono, "Geist Mono");
    });
}

#[test]
fn aqua_keeps_its_typography_and_the_geist_override_survives_a_switch() {
    let mut app = TestAppContext::single();
    install(&mut app, ThemeId::GEIST_LIGHT);
    app.update(|cx| {
        silvestris::set_theme_preference(ThemePreference::explicit(ThemeId::AQUA_LIGHT), cx);
        assert_eq!(cx.theme().type_.ui, "Lato");
        assert_eq!(cx.theme().type_.mono, "Source Code Pro");

        silvestris::set_theme_preference(ThemePreference::explicit(ThemeId::GEIST_LIGHT), cx);
        assert_eq!(cx.theme().type_.ui, "Geist");
        assert_eq!(cx.theme().type_.mono, "Geist Mono");
    });
}

#[test]
fn an_empty_override_leaves_the_measured_families_alone() {
    let app = TestAppContext::single();
    app.update(|cx| {
        silvestris::test_support::initialize(ThemePreference::explicit(ThemeId::GEIST_DARK), cx);
        assert_eq!(
            cx.theme().type_.ui,
            "Inter",
            "without an override the Geist theme keeps Silvestris's bundled default"
        );
    });
}
