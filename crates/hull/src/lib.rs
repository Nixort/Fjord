// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Hull — Hardware Abstraction Layer
//!
//! Thin, mostly-safe wrappers over CPU, MMU, interrupts, timers and DMA for
//! each supported architecture (x86_64, aarch64). The rest of the system is
//! written against this API so it stays portable. See `docs/ARCHITECTURE.md` §2.
#![no_std]
#![allow(dead_code)]

pub mod arch;
pub mod boot;
pub mod mmu;
#[cfg(target_arch = "x86_64")]
pub mod paging;
#[cfg(target_arch = "aarch64")]
#[path = "paging_aarch64.rs"]
pub mod paging;
pub mod irq;
pub mod sched_hook;
pub mod timer;
pub mod serial;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub mod context;
#[cfg(target_arch = "x86_64")]
pub mod apic;
#[cfg(target_arch = "aarch64")]
pub mod gic;

/// Earliest platform bring-up: called before Keel.
///
/// TODO(hull): discover memory map, set up early console, enable the MMU,
/// then jump to [`keel::kmain`].
pub fn platform_init() -> ! {
    todo!("Hull platform bring-up — ROADMAP Phase 1")
}
