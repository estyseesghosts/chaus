//! Streaming change detection: bounded hashing with the original fingerprint.
//!
//! The watcher hashes large files on a forced tick, so conflict checks must
//! not allocate a target-sized buffer. These tests pin the byte-for-byte parity
//! with whole-file FNV-1a, the bounded read size, and the hard-failure contract
//! for a target that cannot be inspected.

use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Result as IoResult, Write as _};
use std::path::Path;

use chaus_core::eol::Eol;
use chaus_io::{
    check_conflict, fingerprint, fingerprint_reader, load, metadata_matches, save,
    ConflictCheckError, SaveConflict, SaveError, SaveRequest,
};

const ORACLE_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const ORACLE_PRIME: u64 = 0x0000_0100_0000_01b3;
const STREAM_LENGTH: u64 = 100 * 1024 * 1024 + 257;

fn oracle_step(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(ORACLE_PRIME)
}

/// Generates its bytes instead of storing them, and records the largest slice
/// the fingerprint asks to fill. No allocation grows with `total_bytes`.
struct GeneratedStream<'a> {
    total_bytes: u64,
    emitted: u64,
    largest_request: &'a mut usize,
}

impl Read for GeneratedStream<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> IoResult<usize> {
        *self.largest_request = (*self.largest_request).max(buffer.len());
        if self.emitted == self.total_bytes {
            return Ok(0);
        }
        let count = (buffer.len() as u64).min(self.total_bytes - self.emitted) as usize;
        for byte in &mut buffer[..count] {
            *byte = (self.emitted % 256) as u8;
            self.emitted += 1;
        }
        Ok(count)
    }
}

fn generated_oracle(length: u64) -> u64 {
    let mut hash = ORACLE_OFFSET;
    for index in 0..length {
        hash = oracle_step(hash, (index % 256) as u8);
    }
    hash
}

fn zero_oracle(length: u64) -> u64 {
    let mut hash = ORACLE_OFFSET;
    for _ in 0..length {
        hash = oracle_step(hash, 0);
    }
    hash
}

#[test]
fn streaming_fingerprint_matches_whole_file_hashing_at_chunk_boundaries() {
    for length in [0_usize, 1, 7, 65_535, 65_536, 65_537, 131_073] {
        let mut bytes = Vec::with_capacity(length);
        for index in 0..length {
            bytes.push(((index * 37) % 256) as u8);
        }
        let whole_file = fingerprint(&bytes);
        let streamed = fingerprint_reader(Cursor::new(&bytes)).expect("in-memory stream reads");
        assert_eq!(
            streamed, whole_file,
            "streamed hash must equal whole-file hash for {length} bytes"
        );
    }
}

#[test]
fn a_100_mib_stream_is_hashed_with_a_bounded_buffer() {
    let mut largest_request = 0_usize;
    let stream = GeneratedStream {
        total_bytes: STREAM_LENGTH,
        emitted: 0,
        largest_request: &mut largest_request,
    };
    let streamed = fingerprint_reader(stream).expect("generated stream reads");

    assert_eq!(
        streamed,
        generated_oracle(STREAM_LENGTH),
        "chunked hashing must produce the original FNV-1a fingerprint"
    );
    assert!(
        largest_request <= 64 * 1024,
        "a read asked for {largest_request} bytes; the whole target must never be buffered"
    );
}

#[test]
fn a_sparse_file_over_100_mib_checks_without_a_target_sized_buffer() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("sparse.bin");
    let file = File::create(&path).expect("create sparse fixture");
    file.set_len(STREAM_LENGTH).expect("extend sparse fixture");
    drop(file);

    let streamed = fingerprint_reader(File::open(&path).expect("open sparse fixture"))
        .expect("sparse fixture reads");
    assert_eq!(
        streamed,
        zero_oracle(STREAM_LENGTH),
        "a sparse file of zero bytes must hash to the same fingerprint"
    );
}

#[test]
fn a_failed_read_is_not_silently_hashed_as_empty() {
    struct FailingStream;

    impl Read for FailingStream {
        fn read(&mut self, _buffer: &mut [u8]) -> IoResult<usize> {
            Err(std::io::Error::other("injected read failure"))
        }
    }

    assert!(
        fingerprint_reader(FailingStream).is_err(),
        "a stream that cannot be read must not be treated as unchanged"
    );
}

#[test]
fn a_same_size_same_mtime_edit_is_still_a_conflict() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("sneaky.txt");
    std::fs::write(&path, b"aaaa\n").expect("write fixture");
    let meta = load(&path).expect("load fixture").meta;
    let original_modified = meta.modified.expect("a fresh file has an mtime");

    let mut file = OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open fixture for edit");
    file.write_all(b"bbbb\n").expect("same-size edit");
    file.flush().expect("flush edit");
    file.set_modified(original_modified).expect("restore mtime");
    drop(file);

    assert!(
        metadata_matches(&meta),
        "the cheap check is allowed to miss a same-size edit"
    );
    assert!(
        matches!(
            check_conflict(&meta),
            Err(ConflictCheckError::Conflict(SaveConflict::ChangedOnDisk))
        ),
        "the authoritative check must hash content, not trust timestamps"
    );
}

#[test]
fn an_unreadable_target_is_a_hard_save_failure() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("target.txt");
    std::fs::write(&path, b"original").expect("write fixture");
    let loaded = load(&path).expect("load fixture");

    replace_with_directory(&path);

    assert!(
        matches!(
            check_conflict(&loaded.meta),
            Err(ConflictCheckError::Io { .. })
        ),
        "an unreadable file must not be reported as unchanged"
    );
    let error = save(SaveRequest::new(
        &path,
        &path,
        b"replacement",
        false,
        Eol::Lf,
        Some(&loaded.meta),
    ))
    .expect_err("save must refuse to overwrite an unreadable target");
    assert!(matches!(&error, SaveError::Io { .. }), "got {error:?}");
    assert!(path.is_dir(), "the unreadable target must be left in place");
}

fn replace_with_directory(path: &Path) {
    std::fs::remove_file(path).expect("remove fixture");
    std::fs::create_dir(path).expect("create directory at fixture path");
}
