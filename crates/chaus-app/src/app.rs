//! Window lifecycle and application startup.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chaus_core::settings::Settings;
use chaus_editor::Editor;
use chaus_io::LoadedFile;
use gpui::{
    actions, px, size, App, AppContext, Bounds, Entity, Focusable, KeyBinding, PathPromptOptions,
    WindowHandle,
};
use silvestris::core::command::CommandDescriptor;
use silvestris::{chrome, ThemePreference};

use crate::preferences::open_preferences;
use crate::settings_global::SettingsGlobal;
use crate::theme_choice::theme_id;

actions!(chaus_app, [NewWindow, OpenFile, Preferences]);

static WINDOW_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Result of reading and validating the rc file.
pub(crate) struct SettingsRead {
    pub(crate) settings: Settings,
    /// Human-readable `line N: message` warnings for the Preferences window.
    pub(crate) diagnostics: Vec<String>,
    pub(crate) path: Option<PathBuf>,
}

/// Parse the rc file from the conventional location, reporting diagnostics.
/// A missing file is not an error; it yields defaults and the path where the
/// file would be created.
#[must_use]
pub(crate) fn read_settings() -> SettingsRead {
    let Some(path) = settings_path() else {
        return SettingsRead {
            settings: Settings::default(),
            diagnostics: Vec::new(),
            path: None,
        };
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SettingsRead {
                settings: Settings::default(),
                diagnostics: Vec::new(),
                path: Some(path),
            };
        }
        Err(error) => {
            return SettingsRead {
                settings: Settings::default(),
                diagnostics: vec![format!("Could not read {}: {error}", path.display())],
                path: Some(path),
            };
        }
    };
    let parsed = chaus_core::settings::parse_rc(&source);
    let diagnostics = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| format!("line {}: {}", diagnostic.line, diagnostic.message))
        .collect();
    SettingsRead {
        settings: parsed.settings,
        diagnostics,
        path: Some(path),
    }
}

/// Parse the rc file from the conventional location, ignoring a missing file.
#[must_use]
pub fn load_settings() -> Settings {
    let read = read_settings();
    if let Some(path) = &read.path {
        for diagnostic in &read.diagnostics {
            eprintln!("chaus: {}: {diagnostic}", path.display());
        }
    }
    read.settings
}

pub(crate) fn settings_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("_chausrc"));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".chausrc"))
}

/// Persist Preferences-owned settings to the rc file, preserving comments and
/// unrelated settings.
pub(crate) fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let Some(path) = settings_path() else {
        return Ok(());
    };
    chaus_io::write_settings_rc(&path, settings)
}

/// Push a settings snapshot to every open editor window.
///
/// Preferences changes must not recreate document entities: `set_settings`
/// only replaces layout/typography settings and rebuilds the row cache, so the
/// document, selection, undo history, focus and file metadata stay put. The
/// live Vim mode is intentionally not touched; `settings.vim` is only the
/// default for future windows.
pub(crate) fn apply_to_open_editors(settings: &Settings, cx: &mut App) {
    for window in cx.windows() {
        let Some(handle) = window.downcast::<Editor>() else {
            continue;
        };
        let settings = settings.clone();
        let _ = handle.update(cx, |editor, _, cx| {
            editor.set_settings(settings);
            cx.notify();
        });
    }
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
        // Geist themes want Geist Sans for application chrome. It is not part
        // of Silvestris, so chaus bundles it and installs it through the public
        // per-theme font-override API. Aqua keeps Lato.
        match crate::fonts::register(cx) {
            Ok(()) => {
                for id in [
                    silvestris::ThemeId::GEIST_LIGHT,
                    silvestris::ThemeId::GEIST_DARK,
                ] {
                    silvestris::set_font_overrides(
                        id,
                        silvestris::FontOverrides::new().ui(crate::fonts::GEIST_SANS),
                        cx,
                    );
                }
            }
            Err(error) => eprintln!("chaus: {error}; using the theme's default UI font"),
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
            // GPUI owns the native picker and answers on a oneshot channel, so
            // the Win32 modal loop never runs inside this editor/app update.
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open".into()),
            });
            cx.spawn(async move |cx| {
                let selection = match receiver.await {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    Ok(Ok(None)) => None,
                    Ok(Err(error)) => {
                        eprintln!("chaus: Open failed: {error:#}");
                        None
                    }
                    Err(_) => None,
                };
                let Some(path) = selection else {
                    return;
                };
                let load_path = path.clone();
                let task = cx
                    .background_executor()
                    .spawn(async move { chaus_io::load(&load_path) });
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

/// Register application commands on a window's registry so the File menu and
/// keyboard shortcuts invoke the same handlers.
fn register_app_commands(editor: &Entity<Editor>, cx: &mut App) {
    let commands = editor.read(cx).commands().clone();
    commands.update(cx, |registry, cx| {
        registry
            .register(
                CommandDescriptor::action("file.new", "New").shortcut("secondary-n"),
                |_, _, cx| cx.dispatch_action(&NewWindow),
                cx,
            )
            .expect("file.new is unique");
        registry
            .register(
                CommandDescriptor::action("file.open", "Open…").shortcut("secondary-o"),
                |_, _, cx| cx.dispatch_action(&OpenFile),
                cx,
            )
            .expect("file.open is unique");
        registry
            .register(
                CommandDescriptor::action("settings.open", "Preferences…").shortcut("secondary-,"),
                |_, _, cx| cx.dispatch_action(&Preferences),
                cx,
            )
            .expect("settings.open is unique");
    });
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
    let correlation = WINDOW_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let handle = chrome::open_window(options, cx, move |window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, loaded.clone(), settings.clone()));
        register_app_commands(&editor, cx);
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
        crate::diagnostics::event("window", correlation, "opened");
        window.on_window_should_close(cx, {
            let editor = editor.clone();
            move |window, cx| {
                let allow = editor.update(cx, |editor, cx| editor.confirm_close(window, cx));
                if allow {
                    crate::diagnostics::event("window", correlation, "closed");
                }
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
