# Build and run chaus

The workspace produces one executable, `chaus` on Linux and `chaus.exe` on Windows. Fonts, Tree-sitter grammars and highlight queries are embedded or statically linked, so no assets are installed beside it.

Pinned inputs come from `rust-toolchain.toml`, `Cargo.toml` and `Cargo.lock`. Always build with `--locked`.

| Input | Value |
| --- | --- |
| Rust | 1.98.1, minimal profile plus rustfmt and clippy |
| Silvestris | `f1c2a010c1c84a90c9ad938eb307f5f2ef5a5ba7` |
| GPUI | `6b720973027999bae402df23d469026f4cced988` |
| Tree-sitter | 0.25.10 |
| Ropey | 1.6.1 |

The commands and CLI are identical once the binary exists:

```sh
chaus --help
chaus --version
chaus
chaus notes.txt
chaus 'notes with spaces.txt'
```

Every positional argument opens in its own window. `--` ends option parsing. Unknown options exit with status 2.

## Mandatory quality gates

These come from `AGENTS.md`. Run them before treating a build as release-ready.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release -p chaus-app --bin chaus --locked
```

`cargo build --release -p chaus-app --bin chaus --locked` builds only the binary. Native GUI behavior is not covered by these commands; a real-OS smoke run is still required.

## Windows

The build verified here used the GNU host `x86_64-pc-windows-gnu` with the 1.98.1 toolchain selected by `rust-toolchain.toml`. Check it with `rustup show`.

### Prerequisites

- Rust/rustup, git, and `windres` on `PATH` for GNU builds.
- A C compiler that targets the exact Rust target. Tree-sitter and its grammars are C sources compiled by the `cc` crate.
- Visual Studio C++ Build Tools and Windows SDK for an MSVC build.

Two repo details matter on Windows:

- `.cargo/config.toml` adds `-C target-feature=+crt-static` for `x86_64-pc-windows-gnu`. Without it the executable imports MinGW runtime DLLs that are absent on a clean machine.
- `crates/chaus-app/build.rs` embeds a Common-Controls 6 application manifest through `windres` on GNU and through `link.exe` on MSVC. `TaskDialogIndirect` is a static import that resolves only in Common-Controls 6; without the manifest the process exits with `0xC0000139` before `main`, including for `--help`. On MSVC the same file is passed with `/MANIFEST:EMBED` and `/MANIFESTINPUT:`.

### C compiler for GNU builds

The compiler must emit objects for `x86_64-pc-windows-gnu`. Two setups seen on this machine do not work:

- The gcc shim shipped inside rustup's self-contained target is a linker driver only and cannot compile `-c`.
- The gcc on `PATH` here is `x86_64-pc-cygwin`. Its objects reference `__getreent`, `__assert_func` and `__locale_ctype_ptr`, and the final MinGW link fails with undefined references.

Use a real MinGW-w64 `x86_64-w64-mingw32-gcc`, or point the target compiler variable at it:

```powershell
$env:CC_x86_64_pc_windows_gnu = 'C:\path\to\mingw64\bin\x86_64-w64-mingw32-gcc.exe'
```

Zig 0.17 can also serve as the C compiler. It ships its own libc headers and cross-compiles to `x86_64-windows-gnu`. The `cc` crate passes the Rust spelling `--target=x86_64-pc-windows-gnu`, which Zig rejects, and probes the compiler with `-E`, so a thin wrapper is needed. Compile this as `x86_64-w64-mingw32-gcc.exe` and set `CC_x86_64_pc_windows_gnu` to its path:

```c
#include <process.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    char **args = (char **)malloc(sizeof(char *) * (argc + 8));
    int n = 0;
    args[n++] = "zig";
    args[n++] = "cc";
    args[n++] = "-target";
    args[n++] = "x86_64-windows-gnu";
    args[n++] = "-fno-sanitize=undefined";
    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--target=x86_64-pc-windows-gnu") == 0) continue;
        if (strcmp(argv[i], "-m64") == 0) continue;
        args[n++] = argv[i];
    }
    args[n] = NULL;
    int code = _spawnvp(_P_WAIT, "zig", args);
    free(args);
    return code;
}
```

Zig turns on UBSan for unoptimized C, which leaves debug and test builds with unresolved `__ubsan_handle_*` symbols; the wrapper disables it. Build it with `zig cc -target x86_64-windows-gnu wrapper.c -o x86_64-w64-mingw32-gcc.exe`. This is a build-environment workaround, not a change to the repository's compiler contract.

### Build and run

```powershell
rustup show
cargo build --release -p chaus-app --bin chaus --locked
.\target\release\chaus.exe --help
.\target\release\chaus.exe --version
.\target\release\chaus.exe
.\target\release\chaus.exe '.\notes with spaces.txt'
```

Release builds use the GUI subsystem. `main.rs` attaches to an existing parent console so `--help`, `--version` and CLI errors print in the invoking terminal; double-clicking does not open a console window.

To check the artifact on a clean machine, inspect its imports and confirm it runs without a development checkout or colocated files. `chaus.exe` should depend on Windows system libraries, not MinGW runtime DLLs.

MSVC builds are wired through `build.rs` but were not executed in the environment used to write this guide. GNU and MSVC are not interchangeable link configurations; validate the binary on a clean machine for whichever ABI you ship.

## Linux

These commands follow `docs/linux-build.md` and the workspace manifest. They were not executed on the Windows machine used to write this guide.

### Prerequisites

Install the Rust toolchain with rustup; `rust-toolchain.toml` selects 1.98.1. Also install `git`, `pkg-config`, a C/C++ toolchain, `cmake` if a locked dependency requires it, and development headers for X11/XCB, Wayland, xkbcommon, fontconfig and the Vulkan loader. Other native libraries may be reported by the pinned dependencies' build scripts. Package names differ by distro, so resolve them against your distribution rather than copying a fixed install line.

`crates/chaus-app/Cargo.toml` already declares `gpui_linux` with `default-features = false` and features `wayland` and `x11`. No workspace feature edit is needed. Confirm the resolved graph with:

```sh
cargo tree -e features -i gpui_platform
```

Build in the oldest glibc environment you intend to support. Prefer x86_64 first; AArch64 needs separate GPUI/Silvestris platform validation.

### Build and run

```sh
rustup show
cargo build --release -p chaus-app --bin chaus --locked
file target/release/chaus
ldd target/release/chaus
readelf -d target/release/chaus
readelf --version-info target/release/chaus | grep GLIBC_ | sort -u
./target/release/chaus --help
./target/release/chaus --version
./target/release/chaus
./target/release/chaus './notes with spaces.txt'
```

Launch GUI windows from a real graphical session with `DISPLAY`, `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` set correctly. Headless CI does not prove window behavior. Test native Wayland and X11 separately, including XWayland where relevant.

Open and Save As use Silvestris's `OpenPanel` and `SavePanel` over the shared `StdFsProvider`, not the GPUI native picker. File → Open is its own application-modal window; Save As is a document sheet. Verify real panel navigation, New Folder, Move to Trash, cancellation, focus restoration, Unicode/non-UTF-8 names, symlinked directories and fractional DPI on a desktop session. Headless tests prove the outcome-to-document state machine, not native window management.

### Artifact checks

The output is one ELF, not a universally static binary. Ordinary glibc, libstdc++, fontconfig, windowing and Vulkan userspace dependencies are expected. Check `ldd` and `readelf` for accidental application-private shared objects and record the actual `GLIBC_*` symbol ceiling from the built binary. No grammar `.so`, font directory or query file may be required next to the executable.

## What a release build does not prove

`cargo test`, `cargo clippy` and a successful link do not cover window creation, focus, IME, clipboard, file dialogs, DPI scaling, theme switching or clean-machine startup. Run a native smoke pass on a machine with a desktop before shipping, and record the toolchain target, dependency pins, source SHA and binary hash with the release.
