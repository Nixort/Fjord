// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Build script: hand the kernel link script to the linker and re-run when it
//! (or this script) changes.

use std::path::PathBuf;

fn main() {
    // Pick the per-architecture link script: <repo>/boot/linker{,-aarch64}.ld.
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let script = match arch.as_str() {
        "aarch64" => "boot/linker-aarch64.ld",
        _ => "boot/linker.ld",
    };

    // <repo>/crates/boot -> <repo>/<script>
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let linker_script = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join(script))
        .unwrap_or_else(|| panic!("failed to locate {script} relative to crate manifest"));

    println!("cargo:rustc-link-arg=-T{}", linker_script.display());
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_ARCH");
}
