//! # Hull — Hardware Abstraction Layer
//!
//! Thin, mostly-safe wrappers over CPU, MMU, interrupts, timers and DMA for
//! each supported architecture (x86_64, aarch64). The rest of the system is
//! written against this API so it stays portable. See `docs/ARCHITECTURE.md` §2.
#![no_std]
#![allow(dead_code)]

pub mod arch;
pub mod mmu;
pub mod irq;
pub mod timer;
pub mod serial;

/// Earliest platform bring-up: called before Keel.
///
/// TODO(hull): discover memory map, set up early console, enable the MMU,
/// then jump to [`keel::kmain`].
pub fn platform_init() -> ! {
    todo!("Hull platform bring-up — ROADMAP Phase 1")
}
