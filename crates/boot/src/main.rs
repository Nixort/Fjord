// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Fjord kernel binary (boot shim)
//!
//! The freestanding ELF that a PVH-aware loader (`qemu -kernel`, Xen,
//! cloud-hypervisor) loads and jumps into. The assembly shim in `boot.s`
//! provides `_start`, switches the CPU into 64-bit long mode with an
//! identity-mapped low 1 GiB, then calls [`rust_entry`], which hands off to
//! [`keel::kmain`].
//!
//! Build / boot (nightly + `rust-src`):
//! ```sh
//! cargo shipwright -- build
//! cargo shipwright -- qemu
//! ```
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(not(target_arch = "x86_64"))]
core::compile_error!(
    "the boot shim currently targets x86_64 only (aarch64 is ROADMAP Phase 1)"
);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("boot.s"));

/// Rust entry point, called by the assembly boot shim once long mode is active.
///
/// `pvh_start_info` is the physical address of the PVH `hvm_start_info`
/// structure the loader passed in `ebx`. It is preserved but not consumed yet;
/// memory-map parsing arrives with the frame allocator in ROADMAP Phase 1.
#[no_mangle]
extern "C" fn rust_entry(_pvh_start_info: u64) -> ! {
    let _ = hull::serial::Serial::init();
    hull::arch::init_boot_cpu();
    keel::kmain()
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
