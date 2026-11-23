# Linux build, CLI installation and release validation

## Shipping target

Ship **one executable** named `chaus` that opens a GPUI window from a terminal. Fonts, icons, app styles, Tree-sitter grammars and highlight queries must be embedded into that executable or linked into it. A single ELF **does not imply a completely static Linux binary**: it normally relies on glibc/libstdc++ as applicable, fontconfig/windowing components, Vulkan userspace/driver stack and host system APIs. Do not promise "runs on every Linux distribution".

The pinned GPUI source's [`gpui_platform` features](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui_platform/Cargo.toml) explicitly include `wayland` and `x11`. Its [`gpui_linux`](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui_linux/Cargo.toml) uses `gpui_wgpu`/window backend dependencies. Silvestris has `gpui_platform` with `default-features = false`, but `crates/chaus-app/Cargo.toml` already declares `gpui_linux` with `default-features = false` and features `wayland` and `x11`, so the app enables both backends. Smoke-test both sessions anyway. Cargo feature unification can change as the repository evolves—inspect `cargo tree -e features` rather than assume.

## Environment baseline

Start with a defined supported baseline (e.g. Ubuntu 22.04 x86_64 for packaging; test Fedora and another rolling distro separately). Prefer building in the **oldest supported glibc environment** so the binary does not accidentally require a newer glibc than advertised. Specify the tested `GLIBC_*` symbol ceiling from `readelf` after release builds, not an inferred value. Rust's `x86_64-unknown-linux-gnu` is the initial target. AArch64 can follow after GPUI/Silvestris platform validation.

Install: Rust/rustup matching workspace `rust-toolchain.toml`, `git`, `pkg-config`, `clang`/C toolchain, `cmake` if required by the selected deps, development headers for X11/XCB, Wayland, xkbcommon, fontconfig, Vulkan loader, and other libraries reported by the **pinned revision's** build scripts. Package names differ by distro; don't freeze a guessed `apt install` line as the truth. Verify graphics with the target session's Vulkan diagnostic tool and driver; loader availability alone is insufficient.

Use **both** Wayland and X11 unless the port has an explicit blocker. chaus does this in `crates/chaus-app/Cargo.toml`:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
gpui_linux = { git = "https://github.com/zed-industries/zed", rev = "6b720973027999bae402df23d469026f4cced988", default-features = false, features = ["wayland", "x11"] }
```

Ensure GPUI and Silvestris use one exact revision; do not accidentally bring two incompatible GPUI crates into the dependency graph.

## Build and command-line smoke

```sh
rustup show
cargo tree -e features -i gpui_platform
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release -p chaus-app --bin chaus --locked
file target/release/chaus
ldd target/release/chaus
readelf -d target/release/chaus
readelf --version-info target/release/chaus | grep GLIBC_ | sort -u
./target/release/chaus --help
./target/release/chaus --version
./target/release/chaus
./target/release/chaus './tests/fixtures/notes with spaces.txt'
```

For window tests, run in an actual user graphical session with the correct `DISPLAY`/`WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` rather than headless CI pretending a window has launched. Test window close/resizing, focus/IME/clipboard, font rendering, dropdown menus, file dialogs, DPI scaling, Save prompts and `Ctrl+Shift+V`. Validate both native Wayland and X11 (including XWayland if relevant), keyboard layouts and a machine without development checkout.

## Release artifact

- Produce `chaus-linux-x86_64` as the **actual ELF**, plus optionally `chaus-linux-x86_64.tar.gz` containing the same ELF and notices. The tarball is optional; the executable must not need adjacent resource files.
- Build release with `--locked`; record binary hash (`sha256sum`), Rust/GPUI/Silvestris SHAs, Linux baseline, tested backends and all third-party font/grammar license notices in the release page (not silently omit notices merely because app is one-file).
- Optional `.desktop` entry and desktop icon installer are separate conveniences, not required for starting the binary. Running `./chaus` remains first-class. No AppImage requirement for v0.1; it can be offered later if true cross-distro portability needs it.
- Check `ldd` and `readelf`; distinguish ordinary system libraries from accidental application-private `.so`/plugin deps. Check actual graphics driver availability. If shader assets are required at runtime by the pinned GPUI build, ensure the build embeds them or explicitly record that a single-file release is **blocked**; do not claim self containment merely because it launches on the build PC.

## Common blockers and how to diagnose

- **No window / panic on startup:** missing X11/Wayland feature, no graphical session, incompatible Vulkan/driver, or a native library load failure. Check environment + `cargo tree -e features` + stderr first.
- **Build succeeds, clean machine fails:** `ldd` missing shared libs, newer glibc symbols, missing graphics loader/driver, or accidentally external font/query asset; reproduce in oldest-supported clean VM/container *with graphics support*.
- **IME or keybindings work on one session only:** test X11 and Wayland separately; don't treat GPUI mock tests as native coverage.
- **Slow start/flicker:** measure cold startup and first paint; don't reintroduce a splash window or asset downloads.

## Do not

- Do not set `x86_64-unknown-linux-musl` and promise a fully static GPUI app without proving backend/font/graphics compatibility.
- Do not silently ship external `.so` grammar modules, fonts or queries next to the executable.
- Do not require an installer or wrapper shell script for ordinary terminal use.
- Do not hardcode a Wayland-only or X11-only build without documenting missing backend support.
- Do not assume container/headless test success proves GUI behavior.

## Sources

[Pinned GPUI Linux features](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui_linux/Cargo.toml), [pinned platform features](https://github.com/zed-industries/zed/blob/6b720973027999bae402df23d469026f4cced988/crates/gpui_platform/Cargo.toml), [Silvestris source](https://github.com/estyseesghosts/silvestris/blob/source-list-shell/Cargo.toml), [Zed Linux documentation (architecture reference)](https://zed.dev/docs/linux).
