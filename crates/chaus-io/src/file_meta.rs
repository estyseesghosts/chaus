//! File identity and the metadata captured at load time.
//!
//! Identity is used to detect a target that was deleted and recreated between
//! load and save. Hard-link count guards a commit that would silently break a
//! link shared with another path. Both are best-effort: a platform that cannot
//! report them falls back to size, mtime and content hash.

#[cfg(unix)]
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chaus_core::eol::Eol;

/// A stable description of the file content and identity at load/save time.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    /// The path actually written (symlinks resolved).
    pub canonical_target: PathBuf,
    /// The path the person asked for, used for titles and errors.
    pub user_requested_path: PathBuf,
    pub had_utf8_bom: bool,
    pub preferred_eol: Eol,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub identity: Option<FileIdentity>,
    /// FNV-1a hash of the raw bytes, independent of size/mtime.
    pub fingerprint: u64,
}

/// OS file identity where available.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume: u64,
    pub index: u64,
}

/// OS identity of `path`, when the platform exposes one.
pub(crate) fn platform_identity(path: &Path) -> Option<FileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).ok()?;
        Some(FileIdentity {
            volume: metadata.dev(),
            index: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        windows_meta::identity(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

/// Number of hard links to `path`, where the platform can report it.
pub(crate) fn hard_link_count(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).ok().map(|metadata| metadata.nlink())
    }
    #[cfg(windows)]
    {
        windows_meta::link_count(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(windows)]
mod windows_meta {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    fn information(path: &Path) -> Option<BY_HANDLE_FILE_INFORMATION> {
        let file = File::open(path).ok()?;
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: `file` is an open handle that stays alive for the whole call,
        // and the output pointer refers to writable, correctly aligned storage.
        let success = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr())
        };
        if success == 0 {
            None
        } else {
            // SAFETY: the API initialized the whole structure on success.
            Some(unsafe { information.assume_init() })
        }
    }

    pub(crate) fn identity(path: &Path) -> Option<super::FileIdentity> {
        let information = information(path)?;
        Some(super::FileIdentity {
            volume: u64::from(information.dwVolumeSerialNumber),
            index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        })
    }

    pub(crate) fn link_count(path: &Path) -> Option<u64> {
        Some(u64::from(information(path)?.nNumberOfLinks))
    }
}
