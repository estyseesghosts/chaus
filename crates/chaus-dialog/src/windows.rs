//! Win32 common file dialogs.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;

use crate::Choice;

const MAX_PATH_UNITS: usize = 32_768;

pub(crate) fn is_available() -> bool {
    true
}

pub(crate) fn choose_file(
    choice: Choice,
    title: &str,
    suggested: Option<&Path>,
) -> Option<PathBuf> {
    let mut buffer = vec![0u16; MAX_PATH_UNITS];
    if let Some(suggested) = suggested {
        let units: Vec<u16> = suggested.as_os_str().encode_wide().collect();
        let length = units.len().min(buffer.len() - 1);
        buffer[..length].copy_from_slice(&units[..length]);
    }
    let filter: Vec<u16> = "Text files\0*.txt;*.md;*.rs;*.toml;*.json;*.log\0All files\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let mut flags = OFN_EXPLORER | OFN_NOCHANGEDIR;
    flags |= match choice {
        Choice::Open => OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        Choice::Save => OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
    };
    // SAFETY: GetActiveWindow has no preconditions and returns a null handle
    // when no window is active; a null owner is valid for these dialogs.
    let owner = unsafe { GetActiveWindow() };
    let mut options = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: flags,
        ..Default::default()
    };
    // SAFETY: `options` points at live, correctly sized UTF-16 buffers for the
    // duration of the call, and the Win32 dialog returns only after the person
    // has accepted or cancelled.
    let accepted = unsafe {
        match choice {
            Choice::Open => GetOpenFileNameW(&mut options),
            Choice::Save => GetSaveFileNameW(&mut options),
        }
    };
    if accepted == 0 {
        return None;
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Some(PathBuf::from(OsString::from_wide(&buffer[..length])))
}
