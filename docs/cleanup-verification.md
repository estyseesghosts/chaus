# Cleanup pass verification

## Checkout

Work was done in `silvestris-x/chaus`, whose HEAD before the pass was
`8ba0c457b5650436a1a8fd08e945747f10f4b1cb`. That is the exact commit
`docs/cleanup.md` was rechecked against. The other checkout (`workdir/chaus`)
was at the older `d888c5f8c838242fdf93a2912a5d5d34633dd276` and its diff was
pure CRLF churn, so it was left untouched.

## Automated gates

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test --workspace --locked`: 155 test suites, 633 tests, zero failures.
- Windows release build: succeeded; `chaus.exe --version` prints `chaus 0.1.0`.
- Linux `cargo check --workspace --locked`: succeeded under Fedora 44 WSL.
- Linux `cargo test --workspace --locked`: exit 0, 155 test suites, zero
  failures. `cargo build --release --workspace --locked`: exit 0, producing a
  51,815,152-byte `chaus`; `chaus --version` prints `chaus 0.1.0`.
- The first Linux attempts failed for environmental reasons, not code: the
  NTFS mount was too slow, then the disk filled completely and corrupted a
  rustc incremental cache. Verification was rerun from a native ext4 copy of
  the same tree after removing ~49 GB of generated debug artifacts.

The host has no working MSVC linker (GNU `link` shadows `link.exe`), so the
Windows commands use the GNU-host toolchain explicitly:

```text
rustup run 1.98.1-x86_64-pc-windows-gnu cargo <subcommand> --target x86_64-pc-windows-gnu
```

The release build emits one MinGW linker warning about multiple non-default
manifests; it does not affect the produced executable.

## GUI smoke tests (Task 11) — not performed

The interactive smoke matrix in `docs/cleanup.md` Task 11 requires typing, IME,
drag autoscroll, modal dialogs, preferences and theme changes in real windows on
both platforms. This session has no GUI automation or screen-control tool, so
those steps cannot be executed here. Automated coverage includes GPUI
`TestAppContext` tests for close/conflict/recovery sheets, find and replace,
pointer/scroll, settings and theme behavior, but those are not a substitute for
native window smoke runs. A reviewer with desktop access needs to perform the
Task 11 matrix before declaring the GUI portion complete.

## Structural audit

- `Editor` has 20 fields: `Document`, `Selection`, `Settings`, focus handle,
  11 private subsystem owners and the find/close/conflict prompt state.
- `chaus-io/src/lib.rs` is a 25-line facade; `chaus-layout/src/lib.rs` is a
  75-line facade over flat leaves.
- No `find_bar_width` remains in `chaus-layout`; no whole-line Rope conversion
  remains in `editor_paint.rs`.
- One settings persistence owner (`SettingsStore`) and one document mutation
  authority (`Document::apply`) remain.
