//! Encoding-aware, conflict-safe document file I/O for chaus.
//!
//! Loads only UTF-8 (with or without a BOM), never lossily decodes, records each
//! original newline kind, and writes through a same-directory temporary file
//! followed by an OS-appropriate atomic replacement. A save is checked against
//! the identity and content hash captured at load time.
//!
//! The leaf modules keep separate concerns separate: `loaded_file` validates
//! and hashes bytes, `file_meta` captures platform identity, `conflict` checks
//! the save baseline, `save` orchestrates a write and `atomic_replace` owns the
//! platform commit. `recovery` remains here because it reuses the document
//! loader and the same atomic replacement contract.

mod atomic_replace;
mod conflict;
mod file_meta;
mod loaded_file;
mod path;
mod save;

pub mod recovery;

pub use atomic_replace::{set_fail_dir_sync_for_test, write_atomic, ReplaceError};
pub use conflict::{check_conflict, metadata_matches, ConflictCheckError, SaveConflict};
pub use file_meta::{FileIdentity, FileMeta};
pub use loaded_file::{fingerprint, fingerprint_reader, load, LoadError, LoadedFile};
pub use path::expand_home;
pub use save::{encode_for_save, save, set_pre_replace_gate_for_test, SaveError, SaveRequest};
