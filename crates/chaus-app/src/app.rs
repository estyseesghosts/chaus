//! Window lifecycle and application startup.

use std::path::PathBuf;

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use gpui::{actions, px, size, App, AppContext, Bounds, Focusable, KeyBinding, WindowHandle};
use silvestris::{chrome, ThemePreference};

use crate::settings_global::SettingsGlobal;
use crate::theme_choice::theme_id;

actions!(chaus_app, [NewWindow, OpenFile]);

/// Parse the rc file from the conventional location, ignoring a missing file.
#[must_use]
pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(source) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    let parsed = chaus_core::settings::parse_rc(&source);
    for diagnostic in &parsed.diagnostics {
        eprintln!("chaus: {}: {diagnostic}", path.display());
    }
    parsed.settings
}

fn settings_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("_chausrc"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".chausrc"))
}

/// Start the application. CLI-only flags are handled before this is called.
pub fn run(paths: Vec<PathBuf>, new_window: bool) {
    let settings = load_settings();
    crate::platform::application().run(move |cx| {
        let preference = ThemePreference::explicit(theme_id(settings.theme));
        if let Err(error) = silvestris::init(preference, cx) {
            eprintln!("chaus: could not initialize Silvestris: {error:#}");
            cx.quit();
            return;
        }
        cx.set_global(SettingsGlobal(settings.clone()));
        chaus_editor::bind_keys(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-n", NewWindow, None),
            KeyBinding::new("ctrl-o", OpenFile, None),
        ]);
        cx.on_action(|_: &NewWindow, cx| {
            open_untitled(cx);
        });
        cx.on_action(|_: &OpenFile, cx| {
            let Some(path) = chaus_dialog::choose_file(chaus_dialog::Choice::Open, "Open", None)
            else {
                return;
            };
            open_file(&path, cx);
        });
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let mut opened = 0;
        for path in &paths {
            if open_file(path, cx).is_some() {
                opened += 1;
            }
        }
        if new_window || opened == 0 {
            open_untitled(cx);
        }
        cx.activate(true);
    });
}

fn window_for(
    title: &str,
    loaded: Option<chaus_io::LoadedFile>,
    message: Option<String>,
    cx: &mut App,
) -> Option<WindowHandle<Editor>> {
    let settings = cx.global::<SettingsGlobal>().0.clone();
    let bounds = Bounds::centered(None, size(px(900.), px(620.)), cx);
    let options = chrome::window_options(title, bounds, size(px(420.), px(300.)), cx);
    let handle = chrome::open_window(options, cx, move |window, cx| {
        let editor = cx.new(|cx| Editor::new(window, cx, loaded.clone(), settings.clone()));
        if let Some(message) = message.clone() {
            let editor_handle = editor.clone();
            editor_handle.update(cx, |editor, _| editor.show_message(message));
        }
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.on_window_should_close(cx, {
            let editor = editor.clone();
            move |window, cx| {
                let allow = editor.update(cx, |editor, cx| {
                    let allow = editor.confirm_close();
                    if !allow {
                        editor.show_message("Unsaved changes: close again to discard");
                        cx.notify();
                    }
                    allow
                });
                if !allow {
                    window.activate_window();
                }
                allow
            }
        });
        editor
    });
    match handle {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("chaus: could not open window: {error:#}");
            None
        }
    }
}

/// Open an empty untitled window.
pub fn open_untitled(cx: &mut App) -> Option<WindowHandle<Editor>> {
    window_for("Untitled", None, None, cx)
}

fn open_file(path: &std::path::Path, cx: &mut App) -> Option<WindowHandle<Editor>> {
    match chaus_io::load(path) {
        Ok(loaded) => {
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            window_for(&title, Some(loaded), None, cx)
        }
        Err(error) => {
            eprintln!("chaus: {error}");
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".to_string());
            window_for(&title, None, Some(error.to_string()), cx)
        }
    }
}
