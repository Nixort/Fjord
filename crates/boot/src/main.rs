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
//! The freestanding ELF that a bootloader loads and jumps into. Its whole job
//! is to provide the real `_start` symbol, establish a stack, install a panic
//! handler wired to the early serial console, and transfer control to
//! [`keel::kmain`].
//!
//! Build (nightly + `rust-src`):
//! ```sh
//! cargo build -p boot --target boot/x86_64-fjord.json
//! ```
//!
//! NOTE: loader handoff (limine / UEFI `bootloader` crate, long-mode entry,
//! memory map) is the next sub-step — see `boot/README.md` and ROADMAP Phase 1.
//! Until that lands, `_start` assumes it is entered in 64-bit long mode by an
//! external loader.
#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(not(target_arch = "x86_64"))]
core::compile_error!(
    "the boot shim currently provides `_start` only for x86_64 \
     (aarch64 entry is tracked in ROADMAP Phase 1)"
);

/// Size of the statically reserved boot stack (64 KiB).
const STACK_SIZE: usize = 64 * 1024;

/// 16-byte-aligned backing storage for the boot stack (SysV x86_64 ABI).
#[repr(align(16))]
struct BootStack([u8; STACK_SIZE]);

static mut BOOT_STACK: BootStack = BootStack([0; STACK_SIZE]);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".section .text._start",
    ".global _start",
    "_start:",
    "    lea rsp, [rip + {stack}]",  // base of the boot stack
    "    add rsp, {size}",            // move to the top (stack grows down)
    "    and rsp, -16",               // honour 16-byte ABI alignment
    "    xor rbp, rbp",               // terminate stack-unwind backtraces
    "    call {entry}",
    "2:  hlt",                          // `entry` is `!`; guard just in case
    "    jmp 2b",
    stack = sym BOOT_STACK,
    size = const STACK_SIZE,
    entry = sym rust_entry,
);

/// Rust-side entry, called once by the assembly `_start` shim.
#[no_mangle]
extern "C" fn rust_entry() -> ! {
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
