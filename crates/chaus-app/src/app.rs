//! Window lifecycle and application startup.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chaus_core::settings::Settings;
use chaus_editor::{Editor, FileDialogs};
use chaus_io::LoadedFile;
use gpui::{px, size, App, AppContext, Bounds, Entity, Focusable, Window, WindowHandle};
use silvestris::core::command::CommandDescriptor;
use silvestris::file_panel::{FilePanel, OpenPanel, PanelOutcome};
use silvestris::modal::ModalResult;
use silvestris::window_role::{role_options, WindowRole};
use silvestris::{chrome, ThemePreference};

use crate::preferences::open_preferences;
use crate::settings_global::SettingsGlobal;
use crate::settings_writer::{install_writer, SettingsWriterGlobal};
use crate::theme_choice::theme_id;

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
            editor.set_settings(settings, cx);
            cx.notify();
        });
    }
}

/// Cheap preflight for positional paths. Document decoding is deferred to the
/// GUI, but missing paths and non-files still produce a truthful CLI status.
pub fn validate_startup_paths(paths: Vec<PathBuf>) -> (Vec<PathBuf>, usize) {
    let mut valid = Vec::new();
    let mut failures = 0;
    for path in paths {
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => valid.push(path),
            Ok(_) => {
                failures += 1;
                eprintln!("chaus: {}: not a regular file", path.display());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                failures += 1;
                eprintln!("chaus: {}: file not found", path.display());
            }
            Err(error) => {
                failures += 1;
                eprintln!("chaus: {}: {error}", path.display());
            }
        }
    }
    (valid, failures)
}

/// Start the application and return the process exit status.
///
/// Only path validation happens before GPUI. Each file is loaded on the
/// background executor into a window that already exists, so first paint does
/// not wait for document decoding or the recovery scan.
pub fn run(paths: Vec<PathBuf>, new_window: bool) -> i32 {
    let settings = load_settings();
    let (valid_paths, failures) = validate_startup_paths(paths);
    if valid_paths.is_empty() && !new_window && failures > 0 {
        return 1;
    }
    let exit_code = i32::from(failures > 0);
    let has_paths = !valid_paths.is_empty();
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
        install_settings(cx, settings.clone());
        install_writer(cx, settings_path());
        crate::ui_preferences::install_file_dialogs(cx);
        chaus_editor::bind_keys(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                flush_and_drain_on_exit(cx);
                cx.quit();
            }
        })
        .detach();

        let recovery_task = cx
            .background_executor()
            .spawn(async { chaus_io::recovery::scan() });

        // Each requested path owns one window from the start. Its title says
        // "Opening …" until the deferred load succeeds, so a failure never
        // masquerades as a successfully opened file.
        for path in &valid_paths {
            let name = display_name(path);
            let message = format!("Opening {name}…");
            let Some(handle) = window_for(&format!("Opening {name}"), None, Some(message), cx)
            else {
                continue;
            };
            let load_path = path.clone();
            let target_path = path.clone();
            let task = cx
                .background_executor()
                .spawn(async move { chaus_io::load(&load_path) });
            cx.spawn(async move |cx| {
                let result = task.await;
                cx.update(|cx| match result {
                    Ok(loaded) => {
                        let _ = handle.update(cx, |editor, _, cx| {
                            editor.adopt_loaded(&target_path, loaded, cx);
                        });
                    }
                    Err(error) => {
                        eprintln!("chaus: {error}");
                        let _ = handle.update(cx, |editor, _, _| {
                            editor.show_message(format!("Open failed: {error}"));
                        });
                    }
                });
            })
            .detach();
        }
        let recovery_host = if !has_paths { open_untitled(cx) } else { None };
        let extra_host = if new_window { open_untitled(cx) } else { None };

        // Recovery never replaces a CLI file, and never replaces text typed
        // while the background scan was running. A pristine host window is the
        // only window that may adopt the queue in place; otherwise records get
        // their own window.
        cx.spawn(async move |cx| {
            let entries = recovery_task.await;
            if entries.is_empty() {
                return;
            }
            cx.update(|cx| {
                let remaining = entries;
                if let Some(handle) = recovery_host {
                    let accepted = handle.update(cx, |editor, window, cx| {
                        editor.queue_recovery(remaining.clone(), window, cx)
                    });
                    if accepted.unwrap_or(false) {
                        return;
                    }
                }
                let fallback = extra_host.or_else(|| open_untitled(cx));
                if let Some(handle) = fallback {
                    let _ = handle.update(cx, |editor, window, cx| {
                        editor.queue_recovery(remaining, window, cx);
                    });
                }
            });
        })
        .detach();
        cx.activate(true);
    });
    exit_code
}

/// Final flush before the process exits.
///
/// Settings use a synchronous write; recovery retirement and snapshot writes
/// are background work with an application-owned ticket, so shutdown waits for
/// them. Without this, closing the last window can exit before a `.rec` file
/// is deleted and every discard comes back on the next launch.
pub fn flush_and_drain_on_exit(cx: &mut App) {
    if let Some(writer) = cx.try_global::<SettingsWriterGlobal>() {
        writer.0.read(cx).flush_on_exit();
    }
    let pending = chaus_io::recovery::pending_operations();
    if pending == 0 {
        return;
    }
    if !chaus_io::recovery::drain_pending(Duration::from_secs(30)) {
        eprintln!(
            "chaus: recovery cleanup did not finish before exit ({} operation(s) pending)",
            chaus_io::recovery::pending_operations()
        );
    }
}

/// Install the application-global settings model. Every document window reads
/// it when it opens, so tests that present windows set it the same way `run`
/// does.
pub fn install_settings(cx: &mut App, settings: Settings) {
    cx.set_global(SettingsGlobal(settings));
}

/// Register application commands on a window's registry so the File menu and
/// keyboard shortcuts invoke the same handlers.
pub fn register_app_commands(editor: &Entity<Editor>, cx: &mut App) {
    let commands = editor.read(cx).commands().clone();
    commands.update(cx, |registry, cx| {
        registry
            .register(
                CommandDescriptor::action("file.new", "New").shortcut("secondary-n"),
                |_, _, cx| {
                    open_untitled(cx);
                },
                cx,
            )
            .expect("file.new is unique");
        let open_target = editor.clone();
        registry
            .register(
                CommandDescriptor::action("file.open", "Open…").shortcut("secondary-o"),
                move |_, window, cx| {
                    open_file_panel(&open_target, window, cx);
                },
                cx,
            )
            .expect("file.open is unique");
        registry
            .register(
                CommandDescriptor::action("settings.open", "Preferences…").shortcut("secondary-,"),
                |_, _, cx| open_preferences(cx),
                cx,
            )
            .expect("settings.open is unique");
    });
}

/// File → Open through the real Silvestris `OpenPanel`. The command callback
/// has the window `run_modal()` needs; the panel starts at the current
/// document's directory, or the provider's home for an untitled buffer.
/// `:e path` and command-line paths never reach this route.
pub fn open_file_panel(
    editor: &Entity<Editor>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<FilePanel> {
    let dialogs = FileDialogs::ensure(cx);
    let absolute = editor
        .read(cx)
        .document_path()
        .map(|path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    let start = absolute
        .as_deref()
        .and_then(Path::parent)
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| dialogs.start_path());
    let mut panel = OpenPanel::new(dialogs.provider())
        .title("Chaus: Open")
        .allows_multiple(false)
        .can_choose_files(true)
        .can_choose_directories(false)
        .state_store(dialogs.open_state())
        .frame_store(dialogs.open_frames())
        .on_finish(open_panel_finished);
    if let Some(directory) = start {
        panel = panel.directory_path(directory);
    }
    let panel = panel.run_modal(window, cx);
    dialogs.remember_panel(&panel);
    panel
}

/// Only an accepted outcome names a file. Cancel, Escape and a closed Open
/// window all report `Rejected`/`Cancelled` and open nothing.
fn open_panel_finished(outcome: &PanelOutcome, cx: &mut App) {
    if outcome.result != ModalResult::Accepted {
        return;
    }
    let Some(path) = outcome.first_path().cloned() else {
        eprintln!("chaus: Open returned no host path");
        return;
    };
    let load_path = path.clone();
    let task = cx
        .background_executor()
        .spawn(async move { chaus_io::load(&load_path) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            open_loaded_result(&path, result, cx);
        });
    })
    .detach();
}

fn window_for(
    title: &str,
    loaded: Option<LoadedFile>,
    message: Option<String>,
    cx: &mut App,
) -> Option<WindowHandle<Editor>> {
    let settings = cx.global::<SettingsGlobal>().0.clone();
    let bounds = Bounds::centered(None, size(px(900.), px(620.)), cx);
    let mut options = role_options(WindowRole::Document, title, bounds, cx);
    // The role's default minimum is smaller; a document window keeps Chaus's
    // practical 420 x 300 editor minimum.
    options.window_min_size = Some(size(px(420.), px(300.)));
    let correlation = WINDOW_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let handle = chrome::open_window(options, cx, move |window, cx| {
        let editor = cx.new(|cx| Editor::new(cx, loaded.clone(), settings.clone()));
        register_app_commands(&editor, cx);
        let commands = editor.read(cx).commands().clone();
        if let Err(error) =
            silvestris::command::apply_bindings(&commands, &[chaus_editor::KEY_CONTEXT], cx)
        {
            eprintln!("chaus: could not bind document commands: {error}");
        }
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
