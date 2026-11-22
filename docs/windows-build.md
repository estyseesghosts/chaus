# Windows build: one GUI `.exe` launchable from terminal

## Shipping target and architecture

Deliver **one `chaus.exe`**, runnable by double click or directly through PowerShell/CMD/Windows Terminal. A bare invocation creates a new document window; path arguments open files in separate windows; `--help`/`--version` print to terminal without opening GPUI. No required `.dll`, grammar file, font directory, config file, shell script or installer next to `.exe`. Application resources must be embedded. A Windows executable still uses standard Windows system DLLs and installed graphics drivers. Optional external formatters remain optional.

The currently inspected [Silvestris README](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/README.md) says its Windows workspace uses the **GNU Rust host/toolchain**. Prefer the **same ABI and toolchain** for initial integration to avoid diverging dependencies, then separately evaluate MSVC release builds if desired. Windows GNU and MSVC are **not interchangeable build/link configurations**; use a single target per CI job and test the binary on a clean machine. Modern Rust recommends MSVC for general Windows integration, but that is not evidence the pinned Silvestris GNU build is broken.

## Development setup

- Install the Rust toolchain matching `rust-toolchain.toml`; for GNU builds also install the matching MinGW/GCC linker/C compiler and native tool dependencies. For MSVC builds use Visual Studio C++ Build Tools and Windows SDK; see [Rust Windows MSVC targets](https://doc.rust-lang.org/stable/rustc/platform-support/windows-msvc.html) and [Microsoft's setup guide](https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup).
- Keep GPUI and Silvestris pinned to the **same revision graph** and compile Silvestris gallery/explorer before implementing application code. Confirm linker/search path requirements of C-based Tree-sitter parsers and any shader resource embedding.
- Build native release executable with `--locked` and explicit target. Example for the environment already used by Silvestris:

```powershell
rustup show
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release -p chaus-app --bin chaus --locked
.\target\release\chaus.exe --help
.\target\release\chaus.exe --version
.\target\release\chaus.exe
.\target\release\chaus.exe '.\tests\fixtures\notes with spaces.txt'
```

If cross-compiling with a named target, adjust the release path under `target/<triple>/release/` and confirm compatible compiler/linker. Pin target triple per release CI; don't assume the host default target is stable.

## Subsystem-specific Windows considerations

**Console behavior:** Windows GUI subsystem binaries generally do not attach stdout/stderr to the invoking terminal automatically. The specification requires `--help` and `--version` to print there. Choose and test an approach: (A) build as console subsystem and suppress/no-console behavior appropriately for Explorer launch, or (B) use GUI subsystem with deliberate parent console attachment and stream handling for CLI-only flags. Neither policy should leave a visible console window behind after GUI double-click. Validate PowerShell, CMD and Windows Terminal; if clean one-exe console behavior is platform constrained, document the accepted behavior before release rather than claiming both without testing.

**File paths:** use `args_os`/`OsString`/`PathBuf`; Windows Unicode paths, network locations, path separators, case-insensitivity and file locking are relevant. Do not pass file names through a shell or assume UTF-8 strings. Open failures must preserve attempted path and OS error.

**Window and theme:** use Silvestris `chrome::window_options/open_window/WindowFrame` so its Windows hit-testing, snap, titlebar and close behavior are retained. Start Geist Dark explicitly. Test DPI movement across monitors, 125/150/200%, minimize/maximize, mouse selection and cursor hit-test after scale changes.

**Save safety:** treat Windows file share modes, antivirus locks and ReplaceFileW/MoveFileExW semantics explicitly; see [file-io.md](file-io.md). A same-directory temp is needed, but rename/replace may fail for open targets and metadata/ACL handling differs from POSIX. Failure must leave original and dirty document intact.

**Fonts/resources:** Silvestris already bundles Geist Mono; do not download/install it. Link Tree-sitter grammar C code into `.exe` as static objects/crates and embed `.scm` highlight queries with `include_str!`. Embed icon/manifest/version metadata using a supported build-time resource process; no runtime `.rc`, SVG or font lookups outside the binary. Keep bundled licenses/notices in distributed release metadata.

## Clean-machine packaging audit

1. Build release in CI, copy **only** `chaus.exe` to a fresh Windows 11 x64 VM without Rust/MinGW/dev checkout; run both double-click and from a terminal.
2. Inspect imported DLLs with `dumpbin /DEPENDENTS` for MSVC or a PE dependency inspection tool for GNU. Verify all non-Windows DLL imports are resolved on clean supported Windows. If a GNU build pulls in MinGW runtime DLLs, link them appropriately or choose an ABI/build that actually yields the one-file artifact; **do not ship DLLs beside it** while calling it self-contained.
3. Check that all four themes and highlighting work offline; no hidden file/resource lookups, popups for missing DLL, or auto-install steps.
4. Test CLI `--help`, `--version`, multiple file arguments, relative paths from an arbitrary working directory, paths with spaces and Unicode characters, unknown flags and binary/invalid file inputs.
5. Test Ctrl+Shift+V in Standard/Normal/Insert/Visual, non-US layout, AltGr/IME, clipboard, Windows DPI, close dirty prompt, Save As cancellation and locked-file save failure.
6. Hash release (`Get-FileHash -Algorithm SHA256`) and record toolchain target, dependency pins, source SHA, and clean-VM results.

## Optional installer and association (post-v0.1)

An `.msi`, `winget` manifest, shell context menu or registered "Open with" association may be useful later, but must call the same `.exe` and are **not required** for its normal operation. A CLI invocation must continue to work without an installer and without network access.

## Do not

- Do not assume an `.exe` is self-contained simply because it runs on the build machine.
- Do not require a colocated grammar DLL, font directory, theme JSON, installer, PowerShell launcher or Rust runtime.
- Do not let the GUI subsystem decision silently break `--help` output or make double-click open an unwanted console.
- Do not migrate the ABI to MSVC mid-implementation without first revalidating the full GPUI/Silvestris dependency graph.
- Do not bypass Silvestris window frame to create custom Windows-only chrome.

## Sources

[Silvestris README/toolchain](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/README.md), [Silvestris Windows chrome](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/crates/silvestris/src/chrome.rs), [Rust Windows/MSVC target guide](https://doc.rust-lang.org/stable/rustc/platform-support/windows-msvc.html), [Windows ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew).
