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
//! Provides per-CPU bring-up for each supported architecture:
//! - x86_64: a minimal GDT, TSS and IDT with handlers for CPU exceptions.
//! - aarch64: `VBAR_EL1` vector-table install plus a synchronous-exception
//!   dispatcher (`ESR_EL1` decode). Hardware IRQs live in [`crate::gic`].

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

/// Initialise CPU-local descriptor/vector tables for the current boot CPU.
#[cfg(target_arch = "x86_64")]
pub fn init_boot_cpu() {
    // SAFETY: early boot is single-core here; descriptor tables are static and
    // stay alive forever after being loaded into GDTR/IDTR/TR.
    unsafe { x86_64::init_boot_cpu() }
}

/// Initialise CPU-local descriptor/vector tables for the current boot CPU.
#[cfg(target_arch = "aarch64")]
pub fn init_boot_cpu() {
    // SAFETY: early boot is single-core here; the EL1 vector table is static
    // and stays mapped for the lifetime of the kernel.
    unsafe { aarch64::init_boot_cpu() }
}

/// Portable no-op for architectures without a CPU backend yet.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn init_boot_cpu() {}
