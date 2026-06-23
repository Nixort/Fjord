// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Shipwright — host build orchestrator
//!
//! Runs on the developer host (not no_std). It drives the whole build: compile
//! the workspace for the bare-metal targets, assign PGO optimization tiers per
//! Cell, seal the resulting `.cask` artifacts, record SLSA/in-toto provenance,
//! and submit signatures to Logbook. See `docs/ARCHITECTURE.md` §10.
//!
//! This is the entry point behind `cargo shipwright -- <command>`.

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "help".into());
    match cmd.as_str() {
        // TODO(shipwright): wire these to real build steps (ROADMAP Phase 0/5).
        "build" => todo!("compile workspace for x86_64/aarch64-unknown-none"),
        "check" => todo!("cargo check across all targets"),
        "test" => todo!("run no_std tests under QEMU"),
        "seal" => todo!("seal a .cask: Merkle + sign + Logbook submit"),
        "qemu" => todo!("boot the image in QEMU"),
        _ => {
            eprintln!("Shipwright — Fjord build orchestrator");
            eprintln!("usage: cargo shipwright -- <build|check|test|seal|qemu>");
        }
    }
}
