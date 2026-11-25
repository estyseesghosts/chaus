//! Embeds the Windows application manifest and icon into the `chaus` executable.
//!
//! The manifest is what makes the loader bind Common-Controls 6 instead of the
//! legacy 5.82 library. `TaskDialogIndirect` exists only in version 6, and a
//! statically imported function is resolved before `main` runs, so without this
//! resource every launch ends in `STATUS_ENTRYPOINT_NOT_FOUND`.
//!
//! The icon is resource ID 1 because GPUI's Windows platform calls `LoadImageW`
//! with that ID when it registers its window class. Without it Explorer and the
//! taskbar show the generic executable icon instead of the Chaus mark.
//!
//! GPUI ships an equivalent manifest behind its `windows-manifest` feature, but
//! that feature's resource compilation breaks with the devkitPro `windres` on
//! this toolchain, so chaus-app embeds its own copy. Paths are passed with
//! forward slashes because that `windres` release treats backslashes in the
//! resource path as escape characters.

use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let build_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=CHAUS_BUILD_SHA={build_sha}");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let resources = manifest_dir.join("resources/windows");
    let resource_file = resources.join("chaus.rc");
    let manifest_file = resources.join("chaus.manifest.xml");
    let icon_file = resources.join("chaus.ico");

    // MSVC has no windres. Its rc.exe compiles the whole resource script, so
    // the manifest and icon go into one object instead of separate linker
    // flags. Skipping them is the STATUS_ENTRYPOINT_NOT_FOUND failure above.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rerun-if-changed={}", resource_file.display());
        println!("cargo:rerun-if-changed={}", manifest_file.display());
        println!("cargo:rerun-if-changed={}", icon_file.display());
        let resource_object = out_dir.join("chaus-resources.res");
        let result = Command::new("rc.exe")
            .args([
                "/nologo",
                "/i",
                &forward_slashes(&resources),
                "/fo",
                &forward_slashes(&resource_object),
                &forward_slashes(&resource_file),
            ])
            .output()
            .expect("rc.exe is required to embed the Windows resources");
        if !result.status.success() {
            panic!(
                "rc.exe could not compile {}:\n{}",
                resource_file.display(),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let resource_arg = forward_slashes(&resource_object);
        // Integration tests link the application library directly and need the
        // same Common-Controls 6 manifest as the binary.
        println!("cargo:rustc-link-arg-bins={resource_arg}");
        println!("cargo:rustc-link-arg-tests={resource_arg}");
        return;
    }

    let object_file = out_dir.join("chaus-manifest.o");

    println!("cargo:rerun-if-changed={}", resource_file.display());
    println!("cargo:rerun-if-changed={}", manifest_file.display());
    println!("cargo:rerun-if-changed={}", icon_file.display());

    let input = forward_slashes(&resource_file);
    let include_dir = forward_slashes(&resources);
    let output = forward_slashes(&object_file);

    let result = Command::new("windres")
        .args([
            "--input",
            &input,
            "--output",
            &output,
            "--include-dir",
            &include_dir,
        ])
        .output()
        .expect("windres is required to embed the Windows resources");

    if !result.status.success() {
        panic!(
            "windres could not compile {}:\n{}",
            resource_file.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    println!("cargo:rustc-link-arg-bins={output}");
    // GPUI-linked integration tests need the same Common-Controls 6 resource;
    // without it they abort before the harness with STATUS_ENTRYPOINT_NOT_FOUND.
    println!("cargo:rustc-link-arg-tests={output}");
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
