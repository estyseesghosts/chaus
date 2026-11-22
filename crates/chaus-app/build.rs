//! Embeds the Windows application manifest into the `chaus` executable.
//!
//! The manifest is what makes the loader bind Common-Controls 6 instead of the
//! legacy 5.82 library. `TaskDialogIndirect` exists only in version 6, and a
//! statically imported function is resolved before `main` runs, so without this
//! resource every launch ends in `STATUS_ENTRYPOINT_NOT_FOUND`.
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

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        println!(
            "cargo:warning=chaus embeds its Windows manifest with windres; \
             the {} environment has no manifest step yet",
            env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default()
        );
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let resources = manifest_dir.join("resources/windows");
    let resource_file = resources.join("chaus.rc");
    let manifest_file = resources.join("chaus.manifest.xml");
    let object_file = out_dir.join("chaus-manifest.o");

    println!("cargo:rerun-if-changed={}", resource_file.display());
    println!("cargo:rerun-if-changed={}", manifest_file.display());

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
        .expect("windres is required to embed the Windows manifest");

    if !result.status.success() {
        panic!(
            "windres could not compile {}:\n{}",
            resource_file.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    println!("cargo:rustc-link-arg-bins={output}");
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
