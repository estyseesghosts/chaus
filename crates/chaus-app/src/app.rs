//! Window lifecycle and application startup.

use std::path::{Path, PathBuf};

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::LoadedFile;
use gpui::{actions, px, size, App, AppContext, Bounds, Focusable, KeyBinding, WindowHandle};
use silvestris::{chrome, ThemePreference};

use crate::preferences::open_preferences;
use crate::settings_global::SettingsGlobal;
use crate::theme_choice::theme_id;

actions!(chaus_app, [NewWindow, OpenFile, Preferences]);

/// Parse the rc file from the conventional location, ignoring a missing file.
#[must_use]
pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Settings::default();
        }
        Err(error) => {
            eprintln!("chaus: could not read {}: {error}", path.display());
            return Settings::default();
        }
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

/// Persist the theme and Vim default to the rc file, preserving comments and
/// unrelated settings.
pub(crate) fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    chaus_io::write_settings_rc(&path, settings)
}

/// Start the application and return the process exit status.
///
/// Positional files are loaded before GPUI starts. That makes failures visible
/// with a nonzero status instead of opening a misleading untitled window, and
/// it removes the startup race where every window could fail and the process
/// still looked successful.
pub fn run(paths: Vec<PathBuf>, new_window: bool) -> i32 {
    let settings = load_settings();
    let mut failures = 0;
    let mut loaded = Vec::new();
    for path in paths {
        match chaus_io::load(&path) {
            Ok(file) => loaded.push((path, Some(file))),
            Err(error) => {
                failures += 1;
                eprintln!("chaus: {error}");
                loaded.push((path, None));
            }
        }
    }
    let opened_files = loaded.iter().filter(|(_, file)| file.is_some()).count();
    if opened_files == 0 && !new_window && failures > 0 {
        return 1;
    }
    let exit_code = i32::from(failures > 0);
    let has_paths = !loaded.is_empty();
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
            KeyBinding::new("ctrl-,", Preferences, None),
        ]);
        cx.on_action(|_: &NewWindow, cx| {
            open_untitled(cx);
        });
        cx.on_action(|_: &Preferences, cx| open_preferences(cx));
        cx.on_action(|_: &OpenFile, cx| {
            if !chaus_dialog::is_available() {
                eprintln!("chaus: no native file picker is available on this platform yet");
                return;
            }
            let Some(path) = chaus_dialog::choose_file(chaus_dialog::Choice::Open, "Open", None)
            else {
                return;
            };
            let load_path = path.clone();
            let task = cx
                .background_executor()
                .spawn(async move { chaus_io::load(&load_path) });
            cx.spawn(async move |cx| {
                let result = task.await;
                let _ = cx.update(|cx| open_loaded_result(&path, result, cx));
            })
            .detach();
        });
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let recovery_records = chaus_io::recovery::list();
        if !has_paths {
            if let Some(handle) = open_untitled(cx) {
                if !recovery_records.is_empty() {
                    let _ = handle.update(cx, |editor, window, cx| {
                        editor.queue_recovery(recovery_records, window, cx);
                    });
                }
            }
            cx.activate(true);
            return;
        }

        let mut first = None;
        for (path, file) in loaded {
            let Some(file) = file else {
                continue;
            };
            if let Some(handle) = window_for(&display_name(&path), Some(file), None, cx) {
                if first.is_none() {
                    first = Some(handle);
                }
            }
        }
        if new_window {
            open_untitled(cx);
        }
        if let Some(handle) = first {
            let _ = handle.update(cx, |editor, window, cx| {
                editor.queue_recovery(recovery_records, window, cx);
            });
        }
        cx.activate(true);
    });
    exit_code
}

fn window_for(
    title: &str,
    loaded: Option<LoadedFile>,
    message: Option<String>,
    cx: &mut App,
) -> Option<WindowHandle<Editor>> {
    let settings = cx.global::<SettingsGlobal>().0.clone();
    let bounds = Bounds::centered(None, size(px(900.), px(620.)), cx);
    let options = chrome::window_options(title, bounds, size(px(420.), px(300.)), cx);
    let handle = chrome::open_window(options, cx, move |window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, loaded.clone(), settings.clone()));
        if let Some(message) = message.clone() {
            let editor_handle = editor.clone();
            editor_handle.update(cx, |editor, _| editor.show_message(message));
        }
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let vim_editor = editor.clone();
        window
            .on_focus_out(&focus, cx, move |_, _, cx| {
                vim_editor.update(cx, |editor, cx| editor.cancel_vim_prefix(cx));
            })
            .detach();
        window.on_window_should_close(cx, {
            let editor = editor.clone();
            move |window, cx| {
                let allow = editor.update(cx, |editor, cx| editor.confirm_close(window, cx));
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

/// Create a window for a load result. Startup and Ctrl+O call this after a
/// background load finishes.
fn open_loaded_result(
    path: &Path,
    result: Result<LoadedFile, chaus_io::LoadError>,
    cx: &mut App,
) -> Option<WindowHandle<Editor>> {
    match result {
        Ok(loaded) => {
            let title = display_name(path);
            window_for(&title, Some(loaded), None, cx)
        }
        Err(error) => {
            eprintln!("chaus: {error}");
            // The file did not open, so the window must not pretend it did by
            // taking the failed path as its title.
            window_for(
                "Untitled",
                None,
                Some(format!("Could not open {}: {error}", path.display())),
                cx,
            )
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
