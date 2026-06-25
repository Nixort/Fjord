// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 24 june 2026

//! aarch64 CPU bring-up: EL1 exception vectors and the synchronous-exception
//! dispatcher.
//!
//! This is the aarch64 counterpart of [`super::x86_64`]: where x86 loads a
//! GDT/TSS/IDT, aarch64 points `VBAR_EL1` at the boot vector table and provides
//! a Rust handler for synchronous exceptions (aborts, alignment faults, illegal
//! `SVC`s). Hardware IRQs are handled separately by the GIC backend in
//! [`crate::gic`], mirroring how x86 routes IRQs through [`crate::apic`].
//!
//! Synchronous exceptions are fatal for now: there is no user space and no
//! demand paging yet, so any abort means a kernel bug. The dispatcher decodes
//! `ESR_EL1`, reports the faulting state over the early serial console, and
//! halts.

use core::arch::{asm, global_asm};

unsafe extern "C" {
    /// EL1 vector table defined by the aarch64 boot shim (`boot-aarch64.s`).
    static __vectors: u8;
}

/// Point `VBAR_EL1` at the EL1 vector table and serialise the write.
///
/// The boot shim installs the same table early (so exceptions are catchable
/// before this runs); re-affirming it here makes the `arch` module the
/// documented owner of CPU vector bring-up, symmetric with the x86_64 path that
/// loads the IDT in its own `init_boot_cpu`.
///
/// # Safety
/// Must be called once during early boot, single-core, before interrupts are
/// unmasked. `__vectors` is a static, 2 KiB-aligned table that lives forever.
pub unsafe fn init_boot_cpu() {
    // SAFETY: `__vectors` is the static, 2 KiB-aligned EL1 vector table; writing
    // VBAR_EL1 + ISB installs it for all subsequent synchronous/IRQ exceptions.
    unsafe {
        let vbar = &raw const __vectors as u64;
        asm!(
            "msr vbar_el1, {vbar}",
            "isb",
            vbar = in(reg) vbar,
            options(nomem, nostack, preserves_flags),
        );
    }

    crate::kprintln!("hull: aarch64 EL1 vectors installed (VBAR_EL1)");
}

/// Human-readable name for the `ESR_EL1.EC` exception-class field (the AArch64
/// subset we expect to see in kernel mode).
fn ec_name(ec: u32) -> &'static str {
    match ec {
        0x00 => "unknown reason",
        0x07 => "SIMD/FP access trap",
        0x0E => "illegal execution state",
        0x15 => "SVC (AArch64)",
        0x18 => "MSR/MRS/system trap",
        0x20 => "instruction abort (lower EL)",
        0x21 => "instruction abort (same EL)",
        0x22 => "PC alignment fault",
        0x24 => "data abort (lower EL)",
        0x25 => "data abort (same EL)",
        0x26 => "SP alignment fault",
        0x2C => "trapped FP exception",
        0x30 => "breakpoint (same EL)",
        0x3C => "BRK instruction",
        _ => "other synchronous exception",
    }
}

/// Rust side of the EL1 synchronous-exception vector, invoked by the `el1_sync`
/// assembly trampoline with the syndrome registers already read into arguments.
///
/// Synchronous exceptions are fatal in the current kernel: report the decoded
/// fault and halt. The signature matches the trampoline's argument order.
#[no_mangle]
extern "C" fn fjord_aarch64_sync(esr: u64, far: u64, elr: u64, spsr: u64) -> ! {
    let ec = ((esr >> 26) & 0x3F) as u32;
    let iss = esr & 0x01FF_FFFF;
    let name = ec_name(ec);

    crate::kprintln!("\n[CPU EXCEPTION] aarch64 synchronous");
    crate::kprintln!("  EC    = 0x{ec:02x}  {name}");
    crate::kprintln!("  ESR   = 0x{esr:016x}  ISS = 0x{iss:06x}");
    crate::kprintln!("  FAR   = 0x{far:016x}");
    crate::kprintln!("  ELR   = 0x{elr:016x}");
    crate::kprintln!("  SPSR  = 0x{spsr:016x}");

    // Decode the abort fault-status fields so a W^X violation or a bad mapping
    // is legible from the log alone, without an external ESR decoder.
    if ec == 0x24 || ec == 0x25 {
        let wnr = (esr >> 6) & 1;
        let dfsc = esr & 0x3F;
        crate::kprintln!(
            "  data abort: {} access, DFSC = 0x{dfsc:02x}",
            if wnr == 1 { "write" } else { "read" }
        );
    } else if ec == 0x20 || ec == 0x21 {
        let ifsc = esr & 0x3F;
        crate::kprintln!("  instruction abort: IFSC = 0x{ifsc:02x}");
    }

    halt_forever()
}

/// Park the CPU forever with interrupts masked.
fn halt_forever() -> ! {
    loop {
        // SAFETY: mask DAIF and `wfe`; the architectural low-power wait parks
        // the core until an (already-masked) event, so it idles here.
        unsafe {
            asm!("msr daifset, #0xf", "wfe", options(nomem, nostack, preserves_flags));
        }
    }
}

// EL1 synchronous-exception trampoline, branched to from the boot vector
// table's "Current EL" synchronous slots. The handler never returns, so it
// reads the syndrome registers straight into the AAPCS argument registers and
// tail-calls the Rust dispatcher on the active EL1 stack.
global_asm!(r#"
.section .text.vectors, "ax"
.global el1_sync
el1_sync:
    mrs     x0, esr_el1
    mrs     x1, far_el1
    mrs     x2, elr_el1
    mrs     x3, spsr_el1
    b       fjord_aarch64_sync
"#);
