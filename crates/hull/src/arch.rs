// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Per-architecture backends (CPU init, registers, context switch).
//!
//! Currently provides x86_64 CPU bring-up: a minimal GDT, TSS and IDT with
//! handlers for CPU exceptions. aarch64 vector-table bring-up is tracked in
//! ROADMAP Phase 1.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

/// Initialise CPU-local descriptor tables for the current boot CPU.
#[cfg(target_arch = "x86_64")]
pub fn init_boot_cpu() {
    // SAFETY: early boot is single-core here; descriptor tables are static and
    // stay alive forever after being loaded into GDTR/IDTR/TR.
    unsafe { x86_64::init_boot_cpu() }
}

/// Portable no-op until the aarch64 backend lands.
#[cfg(not(target_arch = "x86_64"))]
pub fn init_boot_cpu() {}
