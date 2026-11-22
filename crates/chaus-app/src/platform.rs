//! Platform construction.
//!
//! `gpui_platform` force-enables GPUI's Windows resource-manifest feature,
//! whose build script passes backslash paths to the MinGW `windres` on this
//! toolchain. Building the platform object directly keeps the same pinned GPUI
//! backend without that build step.

use gpui::Application;

#[cfg(windows)]
mod windows {
    use std::rc::Rc;

    use gpui::Application;
    use gpui_windows::WindowsPlatform;

    pub(crate) fn application() -> Application {
        let platform =
            WindowsPlatform::new(false).expect("could not initialize the Windows GPUI platform");
        Application::with_platform(Rc::new(platform))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use gpui::Application;

    pub(crate) fn application() -> Application {
        Application::with_platform(gpui_linux::current_platform(false))
    }
}

/// The native application for the current target.
#[cfg(windows)]
pub(crate) fn application() -> Application {
    windows::application()
}

/// The native application for the current target.
#[cfg(target_os = "linux")]
pub(crate) fn application() -> Application {
    linux::application()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn application() -> Application {
    compile_error!("chaus supports Windows and Linux in v0.1");
}
