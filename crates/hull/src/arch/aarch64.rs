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

/// Make instructions written to `addr..addr+len` through a *data* mapping
/// fetchable as code: clean the data cache to the Point of Unification and
/// invalidate the instruction cache over the range, with the required
/// barriers. aarch64 I- and D-caches are not coherent, so this is mandatory
/// before executing freshly-written code (e.g. a user program copied into a
/// fresh frame).
///
/// # Safety
/// `addr..addr+len` must be a readable, currently-mapped range; the maintenance
/// ops only publish already-written bytes to the I-side.
pub unsafe fn sync_instruction_cache(addr: u64, len: usize) {
    // A 16-byte stride is the architectural minimum cache-line size, so it is
    // always safe (at worst redundant) regardless of the real line length.
    let end = addr + len as u64;
    let base = addr & !15;
    // SAFETY: cache maintenance by VA over a mapped range; see contract.
    unsafe {
        let mut p = base;
        while p < end {
            asm!("dc cvau, {p}", p = in(reg) p, options(nostack, preserves_flags));
            p += 16;
        }
        asm!("dsb ish", options(nostack, preserves_flags));
        let mut p = base;
        while p < end {
            asm!("ic ivau, {p}", p = in(reg) p, options(nostack, preserves_flags));
            p += 16;
        }
        asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

unsafe extern "C" {
    /// Drop to EL0 at `user_entry` with `user_stack` in SP_EL0, returning once
    /// the EL0 program issues `svc #0` and `el0_sync` unwinds the trip.
    fn fjord_aarch64_enter_user(user_entry: u64, user_stack: u64);
}

/// Saved EL1 stack pointer across an EL0 excursion; `el0_sync` restores it.
#[no_mangle]
static mut FJORD_AARCH64_KERNEL_SP: u64 = 0;
/// Value the EL0 program leaves in `x0` at `svc #0`, captured by `el0_sync`.
#[no_mangle]
static mut FJORD_AARCH64_SVC_ARG: u64 = 0;

/// Drop to EL0, run the user program until it traps via `svc #0`, and return
/// the value it left in `x0`.
///
/// # Safety
/// `user_entry`/`user_stack` must reference EL0-accessible mappings in the live
/// address space, and the boot vector table's lower-EL synchronous slot must
/// route `svc` to `el0_sync` (it does). Single-shot, early-boot use only.
pub unsafe fn enter_user(user_entry: u64, user_stack: u64) -> u64 {
    // SAFETY: see contract; the asm trampoline saves/restores all callee-saved
    // registers plus DAIF and unwinds through `el0_sync`.
    unsafe {
        fjord_aarch64_enter_user(user_entry, user_stack);
        core::ptr::read_volatile(&raw const FJORD_AARCH64_SVC_ARG)
    }
}

// EL0 entry trampoline and the lower-EL synchronous-exception handler.
//
// `fjord_aarch64_enter_user` stashes the callee-saved registers, the current
// DAIF mask, and the EL1 stack pointer, then programs SPSR_EL1 (EL0t, DAIF
// masked — the lower-EL IRQ vector is still `exc_halt`, so EL0 must not take
// asynchronous exceptions), ELR_EL1 and SP_EL0, and `eret`s to EL0. The EL0
// program runs and executes `svc #0`, trapping into `el0_sync` (wired from the
// boot vector table's "Lower EL, AArch64: Synchronous" slot). If the syndrome
// is an SVC we capture x0, restore DAIF + callee-saved state, and unwind back
// into `fjord_aarch64_enter_user`'s caller; any other lower-EL synchronous
// fault falls through to the fatal `el1_sync` reporter for debuggability.
global_asm!(
    r#"
.section .text.vectors, "ax"
.global fjord_aarch64_enter_user
fjord_aarch64_enter_user:
    stp     x19, x20, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x29, x30, [sp, #-16]!
    mrs     x9, daif
    stp     x9, x9, [sp, #-16]!
    adrp    x9, FJORD_AARCH64_KERNEL_SP
    add     x9, x9, #:lo12:FJORD_AARCH64_KERNEL_SP
    mov     x10, sp
    str     x10, [x9]
    msr     sp_el0, x1
    msr     elr_el1, x0
    mov     x10, #0x3c0
    msr     spsr_el1, x10
    eret

.global el0_sync
el0_sync:
    mrs     x9, esr_el1
    lsr     x10, x9, #26
    and     x10, x10, #0x3f
    cmp     x10, #0x15
    b.ne    el1_sync
    adrp    x9, FJORD_AARCH64_SVC_ARG
    add     x9, x9, #:lo12:FJORD_AARCH64_SVC_ARG
    str     x0, [x9]
    adrp    x9, FJORD_AARCH64_KERNEL_SP
    add     x9, x9, #:lo12:FJORD_AARCH64_KERNEL_SP
    ldr     x10, [x9]
    mov     sp, x10
    ldp     x9, x10, [sp], #16
    msr     daif, x9
    ldp     x29, x30, [sp], #16
    ldp     x27, x28, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x19, x20, [sp], #16
    ret
"#
);
