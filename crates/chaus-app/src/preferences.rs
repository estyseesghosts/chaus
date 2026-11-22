//! Preferences window for app-wide defaults.

use chaus_core::settings::{Settings, ThemeName};
use gpui::{
    div, px, size, App, AppContext as _, Bounds, Context, Global, IntoElement, ParentElement as _,
    Render, Styled as _, Window, WindowHandle,
};
use silvestris::controls::{Button, CheckState, Checkbox, Segment, SegmentedControl};
use silvestris::preferences::{Form, FormRow};
use silvestris::{chrome, ThemePreference};

use crate::app::save_settings;
use crate::settings_global::SettingsGlobal;
use crate::theme_choice::theme_id;

/// One preferences window per app. A stale handle is replaced when the old
/// window has been closed.
struct PreferencesWindow(Option<WindowHandle<Preferences>>);

impl Global for PreferencesWindow {}

pub(crate) fn open_preferences(cx: &mut App) {
    if let Some(handle) = cx
        .try_global::<PreferencesWindow>()
        .and_then(|global| global.0)
    {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let bounds = Bounds::centered(None, size(px(520.), px(280.)), cx);
    let options = chrome::fixed_window_options("Preferences", bounds, cx);
    match chrome::open_window(options, cx, |_, cx| cx.new(Preferences::new)) {
        Ok(handle) => cx.set_global(PreferencesWindow(Some(handle))),
        Err(error) => eprintln!("chaus: could not open preferences: {error:#}"),
    }
}

struct Preferences {
    settings: Settings,
}

impl Preferences {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            settings: cx.global::<SettingsGlobal>().0.clone(),
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        cx.set_global(SettingsGlobal(self.settings.clone()));
        silvestris::set_theme_preference(
            ThemePreference::explicit(theme_id(self.settings.theme)),
            cx,
        );
        if let Err(error) = save_settings(&self.settings) {
            eprintln!("chaus: could not save preferences: {error}");
        }
        cx.notify();
    }
}

impl Render for Preferences {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let theme_entity = entity.clone();
        let theme_control = SegmentedControl::new(
            "preferences-theme",
            "Theme",
            ThemeName::ALL
                .into_iter()
                .map(|theme| Segment::new(theme.as_str(), theme_label(theme)))
                .collect(),
        )
        .selected(self.settings.theme.as_str())
        .on_select(move |id, _, cx| {
            let theme = ThemeName::parse(id).unwrap_or_default();
            theme_entity.update(cx, |preferences, cx| {
                preferences.settings.theme = theme;
                preferences.apply(cx);
            });
        });
        let vim_entity = entity;
        let vim = Checkbox::new("preferences-vim", "Enable Vim mode by default")
            .checked(self.settings.vim)
            .on_change(move |state: &CheckState, _, cx| {
                let enabled = *state == CheckState::On;
                vim_entity.update(cx, |preferences, cx| {
                    preferences.settings.vim = enabled;
                    preferences.apply(cx);
                });
            });
        chrome::WindowFrame::new("Preferences").fixed_size().child(
            div()
                .size_full()
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(
                    Form::new(110.0)
                        .row(FormRow::new("Theme", theme_control))
                        .row(FormRow::control(vim)),
                )
                .child(
                    div().flex().justify_end().child(
                        Button::new("preferences-close", "Close")
                            .on_click(|_, window, _| window.remove_window()),
                    ),
                ),
        )
    }
}

fn theme_label(theme: ThemeName) -> &'static str {
    match theme {
        ThemeName::GeistDark => "Geist Dark",
        ThemeName::GeistLight => "Geist Light",
        ThemeName::AquaLight => "Aqua Light",
        ThemeName::AquaDark => "Aqua Dark",
        _ => "Theme",
    }
}
