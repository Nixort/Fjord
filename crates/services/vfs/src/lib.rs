// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # vfs — virtual filesystem + namespacing
//!
//! Deprivileged userspace service running over `fjord-rt`, reachable only via
//! capabilities granted by Helm. See `docs/ARCHITECTURE.md` §8.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// Service entry point; driven by the async runtime.
/// TODO(vfs): handle requests over its IPC endpoint.
pub fn run() -> ! {
    todo!("vfs service loop")
}
