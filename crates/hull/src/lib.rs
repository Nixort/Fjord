// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Hull — Hardware Abstraction Layer
//!
//! Thin, mostly-safe wrappers over CPU, MMU, interrupts, timers and DMA for
//! each supported architecture (x86_64, aarch64). The rest of the system is
//! written against this API so it stays portable. See `docs/ARCHITECTURE.md` §2.
#![no_std]
#![allow(dead_code)]

#[cfg(target_arch = "x86_64")]
pub mod apic;
pub mod arch;
pub mod boot;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub mod context;
#[cfg(target_arch = "aarch64")]
pub mod gic;
pub mod irq;
pub mod irq_hook;
pub mod mmu;
#[cfg(target_arch = "x86_64")]
pub mod paging;
#[cfg(target_arch = "aarch64")]
#[path = "paging_aarch64.rs"]
pub mod paging;
pub mod sched_hook;
pub mod serial;
pub mod timer;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub mod user;

/// Earliest platform bring-up fallback.
///
/// The real boot path enters `keel::kmain` from the architecture shim after it
/// has collected boot information. If a platform accidentally calls this generic
/// fallback, fail closed: emit a clear marker and park instead of pretending that
/// memory discovery or MMU setup succeeded.
pub fn platform_init() -> ! {
    crate::kprintln!("fjord: generic Hull platform_init fallback; parking CPU");
    loop {
        core::hint::spin_loop();
    }
}
