//! Preferences window for app-wide defaults.
//!
//! The form edits one validated `Settings` value. Applying it updates the rc
//! file and every open editor in place: document, selection, undo history,
//! focus and file state are never recreated. The Vim checkbox is the default
//! for new windows; the current window's live mode stays on Ctrl+Shift+V /
//! View → Vim Mode.

use std::path::PathBuf;

use chaus_core::settings::{Settings, ThemeName, WrapMode};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, size, App, AppContext as _, Bounds, Context, Entity, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    WindowHandle,
};
use silvestris::controls::{
    Button, CheckState, Checkbox, Segment, SegmentedControl, Stepper, TextField, TextFieldEvent,
};
use silvestris::preferences::{Form, FormRow};
use silvestris::{chrome, ActiveTheme, ThemePreference};

use crate::app::{apply_to_open_editors, read_settings, save_settings, settings_path};
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
    let bounds = Bounds::centered(None, size(px(560.), px(660.)), cx);
    let options = chrome::fixed_window_options("Preferences", bounds, cx);
    match chrome::open_window(options, cx, |window, cx| {
        cx.new(|cx| Preferences::new(window, cx))
    }) {
        Ok(handle) => cx.set_global(PreferencesWindow(Some(handle))),
        Err(error) => eprintln!("chaus: could not open preferences: {error:#}"),
    }
}

struct Preferences {
    settings: Settings,
    font_field: Entity<TextField>,
    rc_path: Option<PathBuf>,
    diagnostics: Vec<String>,
    save_error: Option<String>,
}

impl Preferences {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let font_field = cx.new(|cx| TextField::new(window, cx));
        let settings = cx.global::<SettingsGlobal>().0.clone();
        font_field.update(cx, |field, cx| {
            field.set_text(settings.font_family.clone().unwrap_or_default(), cx);
            field.set_label("Font", cx);
            field.set_placeholder("Theme default (Geist Mono)", cx);
        });
        cx.subscribe(
            &font_field,
            |preferences, field, event: &TextFieldEvent, cx| {
                if matches!(
                    event,
                    TextFieldEvent::Submitted | TextFieldEvent::EndedEditing
                ) {
                    let family = field.read(cx).text().trim().to_string();
                    preferences.settings.font_family = (!family.is_empty()).then_some(family);
                    preferences.apply(cx);
                }
            },
        )
        .detach();
        let read = read_settings();
        // The global is the canonical in-memory model; `read` supplies the
        // path and warnings without discarding unsaved session values.
        let settings = cx.global::<SettingsGlobal>().0.clone();
        Self {
            settings,
            font_field,
            rc_path: read.path,
            diagnostics: read.diagnostics,
            save_error: None,
        }
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        cx.set_global(SettingsGlobal(self.settings.clone()));
        silvestris::set_theme_preference(
            ThemePreference::explicit(theme_id(self.settings.theme)),
            cx,
        );
        apply_to_open_editors(&self.settings, cx);
        self.save_error = save_settings(&self.settings)
            .err()
            .map(|error| format!("Could not save preferences: {error}"));
        cx.notify();
    }

    /// Re-read the rc file after an external edit, adopting its values and
    /// surfacing invalid lines instead of silently overwriting them.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read = read_settings();
        self.settings = read.settings;
        self.rc_path = read.path;
        self.diagnostics = read.diagnostics;
        let family = self.settings.font_family.clone().unwrap_or_default();
        self.font_field
            .update(cx, |field, cx| field.set_text(family, cx));
        cx.set_global(SettingsGlobal(self.settings.clone()));
        silvestris::set_theme_preference(
            ThemePreference::explicit(theme_id(self.settings.theme)),
            cx,
        );
        apply_to_open_editors(&self.settings, cx);
        self.save_error = None;
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

        let wrap_entity = entity.clone();
        let wrap = SegmentedControl::new(
            "preferences-wrap",
            "Wrap",
            vec![
                Segment::new("word", "Word"),
                Segment::new("char", "Character"),
                Segment::new("none", "No Wrap"),
            ],
        )
        .selected(self.settings.wrap.as_str())
        .on_select(move |id, _, cx| {
            let mode = WrapMode::parse(id).unwrap_or_default();
            wrap_entity.update(cx, |preferences, cx| {
                preferences.settings.wrap = mode;
                preferences.apply(cx);
            });
        });

        let spaces_entity = entity.clone();
        let spaces = Checkbox::new("preferences-expand-tabs", "Insert spaces for Tab")
            .checked(self.settings.expand_tabs)
            .on_change(move |state: &CheckState, _, cx| {
                let enabled = *state == CheckState::On;
                spaces_entity.update(cx, |preferences, cx| {
                    preferences.settings.expand_tabs = enabled;
                    preferences.apply(cx);
                });
            });

        let vim_entity = entity.clone();
        let vim = Checkbox::new("preferences-vim", "Enable Vim in newly opened windows")
            .checked(self.settings.vim)
            .on_change(move |state: &CheckState, _, cx| {
                let enabled = *state == CheckState::On;
                vim_entity.update(cx, |preferences, cx| {
                    preferences.settings.vim = enabled;
                    preferences.apply(cx);
                });
            });

        let font_size = self.settings.font_size.round() as i32;
        let font_size_control = stepper_row(
            "preferences-font-size",
            "Font size",
            format!("{font_size} px"),
            font_size > 9,
            font_size < 48,
            cx.listener(move |preferences, direction: &i32, _, cx| {
                let next = (preferences.settings.font_size.round() as i32 + direction).clamp(9, 48);
                preferences.settings.font_size = next as f32;
                preferences.apply(cx);
            }),
        );

        let line_tenths = (self.settings.line_height * 10.0).round() as i32;
        let line_height_control = stepper_row(
            "preferences-line-height",
            "Line height",
            format!("{:.1}", line_tenths as f32 / 10.0),
            line_tenths > 10,
            line_tenths < 30,
            cx.listener(move |preferences, direction: &i32, _, cx| {
                let next = ((preferences.settings.line_height * 10.0).round() as i32 + direction)
                    .clamp(10, 30);
                preferences.settings.line_height = next as f32 / 10.0;
                preferences.apply(cx);
            }),
        );

        let tab_stop = self.settings.tab_stop as i32;
        let tab_stop_control = stepper_row(
            "preferences-tab-stop",
            "Tab stop",
            format!("{tab_stop}"),
            tab_stop > 1,
            tab_stop < 32,
            cx.listener(move |preferences, direction: &i32, _, cx| {
                let next = (preferences.settings.tab_stop as i32 + direction).clamp(1, 32);
                preferences.settings.tab_stop = next as usize;
                preferences.apply(cx);
            }),
        );

        let mappings = self.settings.normal_mappings.len();
        let mapping_control = div()
            .text_size(px(12.0))
            .text_color(silvestris::convert::hsla(cx.theme().colors.text_secondary))
            .child(if mappings == 0 {
                "None loaded from the rc file".to_string()
            } else {
                format!("{mappings} loaded from the rc file")
            });

        let path = settings_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "No home directory".to_string());
        let reload_entity = entity.clone();
        let files_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1().text_size(px(11.0)).child(path))
            .child(
                Button::new("preferences-reload", "Reload").on_click(move |_, _, cx| {
                    reload_entity.update(cx, |preferences, cx| preferences.reload(cx));
                }),
            );

        chrome::WindowFrame::new("Preferences").fixed_size().child(
            div()
                .size_full()
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(
                    Form::new(150.0)
                        .row(FormRow::new("Theme", theme_control))
                        .row(
                            FormRow::new("Font", self.font_field.clone())
                                .note("A family installed in the system or bundled with the app."),
                        )
                        .row(FormRow::new("Font size", font_size_control))
                        .row(FormRow::new("Line height", line_height_control))
                        .row(FormRow::new("Wrap", wrap))
                        .row(FormRow::new("Tab stop", tab_stop_control))
                        .row(FormRow::control(spaces))
                        .row(FormRow::control(vim).note(
                            "The current window keeps its live mode; Ctrl+Shift+V toggles it.",
                        ))
                        .row(
                            FormRow::new("Mappings", mapping_control)
                                .note("Normal-mode mappings are edited in the rc file."),
                        )
                        .row(
                            FormRow::new("Settings file", files_control)
                                .note("Reload after editing the file externally."),
                        ),
                )
                .when_some(self.save_error.clone(), |element, error| {
                    element.child(
                        div()
                            .id("preferences-save-error")
                            .debug_selector(|| "preferences-save-error".to_string())
                            .role(gpui::Role::AlertDialog)
                            .text_size(px(12.0))
                            .text_color(silvestris::convert::hsla(cx.theme().colors.error))
                            .child(error),
                    )
                })
                .when(!self.diagnostics.is_empty(), |element| {
                    element.child(
                        div()
                            .id("preferences-diagnostics")
                            .debug_selector(|| "preferences-diagnostics".to_string())
                            .role(gpui::Role::AlertDialog)
                            .text_size(px(12.0))
                            .text_color(silvestris::convert::hsla(cx.theme().colors.warning))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child("The rc file has values Chaus could not use:")
                            .children(self.diagnostics.iter().cloned()),
                    )
                })
                .child(
                    div().flex().justify_end().child(
                        Button::new("preferences-close", "Close")
                            .on_click(|_, window, _| window.remove_window()),
                    ),
                ),
        )
    }
}

fn stepper_row(
    id: &'static str,
    label: &'static str,
    value: String,
    can_decrease: bool,
    can_increase: bool,
    on_step: impl Fn(&i32, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .id(id)
                .debug_selector(move || format!("{id}-value"))
                .text_size(px(12.0))
                .child(value),
        )
        .child(
            Stepper::new(id, label)
                .limits(can_decrease, can_increase)
                .on_step(on_step),
        )
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
