// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # cryptd — key custody + crypto operations
//!
//! Deprivileged userspace service running over `fjord-rt`, reachable only via
//! capabilities granted by Helm. See `docs/ARCHITECTURE.md` §8.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// Runtime state exposed to Helm health checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    /// Service has initialized and is accepting messages.
    Ready,
    /// Service parked because the IPC transport is not available yet.
    Parked,
}

/// Returns the static service name used by Helm supervision.
#[must_use]
pub const fn service_name() -> &'static str {
    "cryptd"
}

/// Returns the static role description used in diagnostics.
#[must_use]
pub const fn service_role() -> &'static str {
    "key custody + crypto operations"
}

/// Service entry point; driven by the async runtime.
pub fn run() -> ! {
    let _state = ServiceState::Parked;
    loop {
        core::hint::spin_loop();
    }
}
