# File I/O: CLI, encoding, newlines, safe save and recovery

## External contract

`[scratchpad] [--new] [--help] [--version] [--] [path ...]` starts **a native graphical application from a terminal**. No file → one empty window. One path → one file window. Several paths → one window per path, **not tabs**. Do not hide the CLI in a separate helper executable. The one binary must also handle `--help`/`--version` without initializing GPUI, fonts or GPU. An invalid flag returns nonzero and prints useful stderr. Use `std::env::args_os` + an `OsString`-aware parser; accept absolute/relative paths and spaces. Consider whether directory arguments produce errors (recommended) rather than treating them as projects.

Opening a missing path should show a clear error (or explicitly offer a new file at that path, without auto-creating it). On launching with multiple files, one failed open must not discard successfully opened files; report error and choose a documented overall exit behavior if the process remains running. Windows file association / Linux `.desktop` integration is **optional** and must call this same executable.

## Document file metadata

Store separately from Rope text:

```text
FileMeta { canonical_target, user_requested_path, encoding, had_utf8_bom,
           eol_by_break, preferred_eol, permissions, original_size,
           identity, modified_time, content_hash, last_saved_revision }
```

`user_requested_path` is the path for title/display; `canonical_target` is the actual save target after symlink policy; identity is OS-specific (device/inode on Linux, file ID/volume identity on Windows where available). **Size/mtime alone are not sufficient** for edit conflict detection; hash or identity/version should guard the write. Do not assume canonicalization always succeeds for a new file.

## Read/decode policy

- **v0.1 supports UTF-8 text** with or without BOM. Detect UTF-8 BOM (`EF BB BF`) and record it; strip BOM from the editable text; preserve it on normal save unless user changes the encoding option.
- If invalid UTF-8, NUL-heavy likely-binary data, or UTF-16 BOM is encountered, **do not lossy-decode silently**. Display a clear unsupported-encoding error and leave original untouched. Explicit UTF-16 import/export may be a later feature; don't imply it works today.
- Read off the UI thread. For large files, report transient loading state and allow cancel if supported. Preserve OS errors and path in the error dialog without dumping entire file bytes to logs.
- Detect newline kinds (`LF`, `CRLF`, lone `CR`) and any mixture before constructing the Rope. Decide and document text normalization consistently with [ropey.md](ropey.md).

**Recommended EOL implementation:** normalize hard breaks to `\n` **in the Rope**, while storing a parallel `EolMap` entry for every hard line break with kind `LF | CRLF | CR`. Each inserted hard break gets the document's preferred EOL; untouched line-break entries retain their original kind. Update the map as part of the **same transaction** as Rope mutations and as part of undo/redo. At save, stream Rope chunks, substituting line breaks from the map, and write the original UTF-8 BOM only when required. Assert `eol_map.len() == count_hard_breaks` and test it after every mutation. This lets normal edits preserve mixed-EOL files without building an entire second String.

File format changes (`Convert Line Endings → LF/CRLF`) are explicit undoable transactions that update the EOL map and mark the document dirty. If preserving obscure text separators or unusual byte sequences is not possible, reject or warn; do not claim byte-identical preservation for unsupported formats. Handle `\r\n` split across I/O chunks correctly.

## Save algorithm and conflicts

1. On save, create an immutable snapshot of text + EOL map + metadata at revision `R`; resolve output encoding; **never** mutate the live document in a worker.
2. Recheck the on-disk target's identity/mtime/length/hash against the load/save baseline. If externally changed, show **Reload / Save As / Overwrite / Cancel**. Overwrite must be explicit and warn about the change; never auto-merge blindly.
3. Construct output **streaming**, not `rope.to_string()` for every save. In the destination's parent directory, create a unique temporary file with restrictive initial permissions. Write bytes, flush, `sync_all`/fsync, preserve original permissions where feasible. Keep temp in the same filesystem as destination.
4. Commit with OS-correct replacement: Linux/POSIX `rename` over existing regular file plus parent-directory fsync; Windows use verified replacement behavior (`ReplaceFileW` for existing targets or appropriate move-with-replace API when necessary) and handle target locks, antivirus sharing violations and rollback. Use a tested abstraction; never assume `std::fs::rename` semantics are identical across OSes. **Never delete the original as a prelude to replace.**
5. If commit succeeds, update disk baseline to the committed content. Clear dirty only if live document is still at revision `R`; if user edited during save, keep dirty and allow a later save. On any failure, retain original, preserve dirty state, and report where a recovery temp was left if applicable.

**Symlinks and hard links:** replacing a path with rename can replace the symlink itself and break a hard link relationship. Explicit policy: detect these cases and warn/resolve the actual target before replacement, or refuse atomic save until a safe behavior is chosen. Test the policy. Permissions, read-only attributes, ACLs and executable bits require preservation tests; do not mistake preserving POSIX mode bits for preserving every metadata field.

**Cancel/close handling:** closing a dirty window offers Save / Discard / Cancel. Save As cancellation leaves window open and dirty. During save, disable duplicate Save or coalesce requests; after a failed save, never close automatically. If several windows have unsaved documents, prompt each or offer an aggregate decision with per-document clarity. Modified/title state must be driven from the document checkpoint, not save-button callbacks.

## External modifications and file watching

On window focus and before Save, stat/hash the file. Optionally add `notify` watcher as an optimization, but watcher events can be coalesced, duplicated or missed; always validate target on Save. If external changes are detected and local document is clean, offer Reload. If dirty, show conflict choices; don't silently reload or discard. A rename/delete of the target must be handled separately from a modified file. Distinguish the window's path spelling from canonical target and avoid watching an entire directory recursively for each file without bounds.

## Crash recovery (after core save reliability)

Use OS state directories (Linux `$XDG_STATE_HOME/scratchpad/` or `~/.local/state/scratchpad/`; Windows `%LOCALAPPDATA%\Scratchpad\Recovery\`), **not** original file sidecars by default. Debounce a write of dirty-document snapshots with metadata (original path, save baseline, document revision, timestamp, encoding/EOL); use an app-controlled storage limit and deletion policy. On launch, offer Recover / Discard per unsaved snapshot. Recovery never silently overwrites original documents. Prefer write-ahead or versioned recovery records so a crash during recovery write leaves previous snapshot intact. Do not market recovery as guaranteeing no data loss until power-failure tests pass.

## Dialog and CLI test matrix

- `scratchpad`, `scratchpad --help`, `scratchpad "a b.txt"`, `scratchpad -- --leading-dash.txt`, multiple paths, missing path, folder path, invalid UTF-8 OsString path on Linux.
- Round-trip UTF-8+BOM, LF, CRLF, mixed LF/CRLF, final newline, no final newline, empty file, long line, Unicode graphemes. For a no-op open/save, output bytes should match input for supported encodings/EOL.
- Edit first/middle/last line of mixed-EOL file and undo: unaffected line separators unchanged. Inserting/deleting line breaks updates EolMap count and undo perfectly.
- Read-only file; lost permissions; file locked by another Windows process; target replaced externally with same mtime/length but different hash; symlink target; hard links; disk full; failed temp commit.
- User types while background save runs: `R` checkpoint, dirty state after completion and file contents correct.
- On close: Save succeeds, Save fails, Save As cancelled, Discard, Cancel. App quits only when all windows are closed and confirmation paths resolved.

## Do not

- Do not use `fs::write(original_path, bytes)` for a potentially destructive save.
- Do not clear dirty before durable replacement succeeds; never silently overwrite external modifications.
- Do not convert UTF-16 or invalid UTF-8 to `�` without asking.
- Do not assume `rename` overwrites consistently or symlinks remain intact.
- Do not call external formatter against the original file path; format against snapshot/stdin and apply via editor transaction.

## Sources

[`tempfile::NamedTempFile`](https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html), [Windows `ReplaceFileW`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew), [Linux `rename(2)`](https://man7.org/linux/man-pages/man2/rename.2.html), [Ropey](https://docs.rs/ropey/latest/ropey/struct.Rope.html).
