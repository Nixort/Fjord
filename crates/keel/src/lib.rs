// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Keel — the Fjord microkernel
//!
//! Keel is a capability-based microkernel (seL4 lineage). It is the only
//! component, besides [`anchor`] and the crypto core, that runs fully
//! privileged. It implements *mechanism, not policy*: capabilities, address
//! spaces, IPC and scheduling. Drivers, filesystems and network stacks all
//! live in userspace.
//!
//! See `docs/ARCHITECTURE.md` §1.
#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod cap;
pub mod vspace;
pub mod ipc;
pub mod tide;
pub mod untyped;

/// Target architecture, resolved at compile time for the boot banner.
#[cfg(target_arch = "x86_64")]
pub const ARCH: &str = "x86_64";
/// Target architecture, resolved at compile time for the boot banner.
#[cfg(target_arch = "aarch64")]
pub const ARCH: &str = "aarch64";
/// Target architecture, resolved at compile time for the boot banner.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub const ARCH: &str = "unknown";

/// Kernel entry point, invoked by the `boot` shim once a stack exists.
///
/// Brings up the early serial console and prints a boot banner so we have a
/// visible sign of life. The real boot sequence (root CSpace, untyped hand-off
/// to `Helm`, first userspace task) lands in ROADMAP Phase 2; until then we
/// park the CPU in a low-power idle loop. Must never return.
pub fn kmain() -> ! {
    let _serial = hull::serial::Serial::init();

    hull::kprintln!();
    hull::kprintln!("Fjord OS — keel v{}", env!("CARGO_PKG_VERSION"));
    hull::kprintln!("  arch    : {ARCH}");
    hull::kprintln!(
        "  profile : {}",
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
    hull::kprintln!("keel: early console up; halting (Phase 2 boot pending).");

    // TODO(keel): init subsystems in order cap -> vspace -> tide -> ipc, then
    // launch the first userspace task.
    idle()
}

/// Park the CPU forever without hammering the bus harder than necessary.
fn idle() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
