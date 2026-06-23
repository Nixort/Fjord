// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Helm — root supervisor + Cask verifier
//!
//! Helm is the first userspace task and holds the root CSpace. It starts and
//! supervises core services and gatekeeps execution: every [`cask`] is
//! verified (signature, Merkle integrity, license budget, Logbook inclusion)
//! before launch. The effective authority of a launched task is
//! `manifest ∩ license ∩ delegated`. See `docs/ARCHITECTURE.md` §4.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

pub mod supervise;
pub mod launch;

/// Helm entry point, started by Keel with the root capability set.
///
/// TODO(helm): bring up cryptd/timed/storaged/vfs/netd in dependency order,
/// then enter the supervision loop.
pub fn main() -> ! {
    todo!("Helm init + supervision loop — ROADMAP Phase 2/3")
}
