//! Durable commit of a same-directory temporary file onto its target.
//!
//! The original is never unlinked before replacement. On Unix the rename is
//! followed by a parent-directory sync; on Windows `ReplaceFileW` preserves the
//! target's ACLs and attributes, and a missing target falls back to
//! `MoveFileExW` with replacement. All unsafe platform calls live here.

use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tempfile::Builder;

/// Failure mode of an OS replacement. A committed rename whose directory sync
/// failed is distinct from a replacement that never happened.
#[derive(Debug)]
pub enum ReplaceError {
    Io(std::io::Error),
    DurabilityUnconfirmed(std::io::Error),
}

impl ReplaceError {
    #[must_use]
    pub fn into_io(self) -> std::io::Error {
        match self {
            Self::Io(error) | Self::DurabilityUnconfirmed(error) => error,
        }
    }
}

static FAIL_DIR_SYNC: AtomicBool = AtomicBool::new(false);

/// Test seam for post-rename directory sync failure (Unix only).
pub fn set_fail_dir_sync_for_test(fail: bool) {
    FAIL_DIR_SYNC.store(fail, Ordering::Relaxed);
}

/// Replace `target` with `temporary` using OS-appropriate atomic semantics.
///
/// Shared by file saves and crash-recovery snapshots. The temporary file must
/// live on the same filesystem as the target. On Unix the parent directory is
/// synced after the rename; on Windows `ReplaceFileW` refuses to run when the
/// target is missing, so that case falls back to a move-with-replace.
pub(crate) fn replace_path(temporary: &Path, target: &Path) -> Result<(), ReplaceError> {
    #[cfg(unix)]
    {
        fs::rename(temporary, target).map_err(ReplaceError::Io)?;
        if let Some(parent) = target.parent() {
            if FAIL_DIR_SYNC.load(Ordering::Relaxed) {
                return Err(ReplaceError::DurabilityUnconfirmed(std::io::Error::other(
                    "injected directory sync failure",
                )));
            }
            let directory = File::open(parent).map_err(ReplaceError::DurabilityUnconfirmed)?;
            directory
                .sync_all()
                .map_err(ReplaceError::DurabilityUnconfirmed)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        windows_replace::replace(temporary, target).map_err(ReplaceError::Io)
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::rename(temporary, target).map_err(ReplaceError::Io)
    }
}

/// Replace `target` with `bytes` atomically.
///
/// The temporary file lives in the target's directory and is synced before the
/// OS-appropriate replacement. A failed write leaves the previous file intact.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = Builder::new().prefix(".chaus-write-").tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if let Ok(existing) = fs::metadata(target) {
        let _ = fs::set_permissions(temporary.path(), existing.permissions());
    }
    let temporary_path = temporary.into_temp_path();
    replace_path(&temporary_path, target).map_err(|error| error.into_io())
}

#[cfg(windows)]
mod windows_replace {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub(crate) fn replace(temporary: &Path, target: &Path) -> std::io::Result<()> {
        let temporary_wide = wide(temporary);
        let target_wide = wide(target);
        // ReplaceFileW preserves the target's ACLs and attributes and writes a
        // backup-free replacement. It requires the target to exist.
        //
        // SAFETY: both arguments are NUL-terminated UTF-16 buffers that remain
        // alive for the duration of the call; ReplaceFileW does not retain
        // them. The remaining arguments are documented to accept null.
        let replaced = unsafe {
            ReplaceFileW(
                target_wide.as_ptr(),
                temporary_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if replaced != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_FILE_NOT_FOUND as i32) {
            return Err(error);
        }
        // Target did not exist: a normal move-with-replace is correct.
        //
        // SAFETY: both arguments are NUL-terminated UTF-16 buffers that remain
        // alive for the duration of the call; MoveFileExW does not retain them.
        let moved = unsafe {
            MoveFileExW(
                temporary_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING,
            )
        };
        if moved == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}
