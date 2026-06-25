// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Fjord kernel binary (boot shim)
//!
//! The freestanding ELF that a PVH-aware loader (`qemu -kernel`, Xen,
//! cloud-hypervisor) loads and jumps into. The assembly shim in `boot.s`
//! provides `_start`, switches the CPU into 64-bit long mode with an
//! identity-mapped low 1 GiB, then calls [`rust_entry`], which parses the PVH
//! memory map and hands off to [`keel::kmain`].
//!
//! Build / boot (nightly + `rust-src`):
//! ```sh
//! cargo shipwright -- build
//! cargo shipwright -- qemu
//! ```
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
core::compile_error!(
    "the boot shim supports x86_64 (PVH) and aarch64 (QEMU virt) only"
);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("boot.s"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("boot-aarch64.s"));

/// Rust entry point on x86_64, called by the assembly boot shim once long mode
/// is active.
///
/// `pvh_start_info` is the physical address of the PVH `hvm_start_info`
/// structure the loader passed in `ebx`. We parse it into a memory map and
/// hand the result to the kernel.
#[cfg(target_arch = "x86_64")]
#[no_mangle]
extern "C" fn rust_entry(pvh_start_info: u64) -> ! {
    let _ = hull::serial::Serial::init();
    hull::arch::init_boot_cpu();

    // SAFETY: the loader passes a valid `hvm_start_info` physical address in
    // `ebx` (or 0); `parse_pvh` validates the magic before trusting it.
    let boot_info = unsafe { hull::boot::parse_pvh(pvh_start_info) };

    keel::kmain(&boot_info)
}

/// Rust entry point on aarch64, called by the assembly boot shim from EL1.
///
/// QEMU `virt` passes the physical address of the flattened device tree in
/// `x0`. We parse it into the physical memory map and hand the result to the
/// kernel, mirroring the PVH path on x86_64.
#[cfg(target_arch = "aarch64")]
#[no_mangle]
extern "C" fn rust_entry(dtb_paddr: u64) -> ! {
    let _ = hull::serial::Serial::init();
    hull::arch::init_boot_cpu();

    // SAFETY: QEMU `virt` passes the physical address of a flattened device
    // tree in `x0` (or 0); `parse_dtb` validates the FDT magic before trusting
    // any field, and the MMU-off identity map makes the DTB directly readable.
    let boot_info = unsafe { hull::boot::parse_dtb(dtb_paddr) };

    keel::kmain(&boot_info)
}

/// Last-resort panic handler: report over the early serial console, then halt.
#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    // `init` is idempotent, so this is safe even mid-boot.
    let _ = hull::serial::Serial::init();
    hull::kprintln!("\n[KERNEL PANIC] {info}");
    loop {
        core::hint::spin_loop();
    }
}
