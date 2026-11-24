//! chaus entry point. CLI parsing happens before GPUI and the GPU initialize.

// A release build is a GUI application: launching it from Explorer must not
// allocate a console window. Debug builds keep the console subsystem so
// `cargo run` and the test harnesses print normally. Attaching to an existing
// parent console below restores `--version`, `--help` and CLI error output
// when the release binary is invoked from a terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use chaus_app::app;

#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    const CONOUT: [u16; 8] = [
        b'C' as u16,
        b'O' as u16,
        b'N' as u16,
        b'O' as u16,
        b'U' as u16,
        b'T' as u16,
        b'$' as u16,
        0,
    ];

    // SAFETY: the calls only attach to the parent's console and install
    // console handles for this process. Failure is expected when launched from
    // Explorer, where there is no parent console.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for std_handle in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let existing = GetStdHandle(std_handle);
            if !existing.is_null() && existing != INVALID_HANDLE_VALUE {
                continue;
            }
            let handle: HANDLE = CreateFileW(
                CONOUT.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE {
                SetStdHandle(std_handle, handle);
            }
        }
    }
}

fn main() {
    // The sink and panic hook must exist before argument parsing and GPUI
    // initialization, which are the phases a GUI launch cannot report aloud.
    chaus_app::diagnostics::init();

    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();

    let command = match chaus_cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("chaus: {error}");
            eprintln!("Try 'chaus --help' for usage.");
            chaus_app::diagnostics::shutdown();
            std::process::exit(2);
        }
    };
    match command {
        chaus_cli::Command::Help => print!("{}", chaus_cli::help_text()),
        chaus_cli::Command::Version => println!("chaus {}", env!("CARGO_PKG_VERSION")),
        chaus_cli::Command::Run { paths, new_window } => {
            let code = app::run(paths, new_window);
            if code != 0 {
                chaus_app::diagnostics::shutdown();
                std::process::exit(code);
            }
        }
        _ => {}
    }
    chaus_app::diagnostics::shutdown();
}
