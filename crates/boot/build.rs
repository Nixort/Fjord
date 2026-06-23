// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Build script: hand the kernel link script to the linker and re-run when it
//! (or this script) changes.

use std::path::PathBuf;

fn main() {
    // <repo>/crates/boot -> <repo>/boot/linker.ld
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let linker_script = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join("boot/linker.ld"))
        .expect("failed to locate boot/linker.ld relative to crate manifest");

    println!("cargo:rustc-link-arg=-T{}", linker_script.display());
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rerun-if-changed=build.rs");
}
