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
            asm!(
                "msr daifset, #0xf",
                "wfe",
                options(nomem, nostack, preserves_flags)
            );
        }
    }
}

// EL1 synchronous-exception trampoline, branched to from the boot vector
// table's "Current EL" synchronous slots. The handler never returns, so it
// reads the syndrome registers straight into the AAPCS argument registers and
// tail-calls the Rust dispatcher on the active EL1 stack.
global_asm!(
    r#"
.section .text.vectors, "ax"
.global el1_sync
el1_sync:
    mrs     x0, esr_el1
    mrs     x1, far_el1
    mrs     x2, elr_el1
    mrs     x3, spsr_el1
    b       fjord_aarch64_sync
"#
);

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

use core::mem::offset_of;

/// A resumable EL0 register frame: everything needed to (re)enter a user task
/// exactly where it last trapped. [`user_run`] `eret`s to EL0 from this frame;
/// the `svc #0` handler saves the live EL0 state back into it.
///
/// The kernel treats it as a coroutine resume point. The syscall ABI carries
/// the number in `x0`, arguments in `x1`/`x2`, and the return value back in `x0`
/// (see `crate::user`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserFrame {
    /// General registers `x0..=x30`.
    x: [u64; 31],
    /// `SP_EL0` — the EL0 stack pointer.
    sp: u64,
    /// `ELR_EL1` — the EL0 PC to resume at.
    elr: u64,
    /// `SPSR_EL1` — the saved EL0 processor state.
    spsr: u64,
}

// The assembly addresses these fields by byte offset; the checks fail the build
// before boot if the layout ever drifts.
const _: () = assert!(offset_of!(UserFrame, x) == 0x00);
const _: () = assert!(offset_of!(UserFrame, sp) == 0xF8);
const _: () = assert!(offset_of!(UserFrame, elr) == 0x100);
const _: () = assert!(offset_of!(UserFrame, spsr) == 0x108);

/// `SPSR_EL1` value for entering EL0t with DAIF masked (the lower-EL IRQ vector
/// is still the fatal reporter, so EL0 must not take asynchronous exceptions).
const SPSR_EL0T_MASKED: u64 = 0x3c0;

impl UserFrame {
    /// A fresh frame that, on the first [`user_run`], begins executing at
    /// `entry` with `SP_EL0 = stack`, EL0t, DAIF masked, and all GPRs zeroed.
    #[must_use]
    pub const fn new(entry: u64, stack: u64) -> Self {
        Self {
            x: [0; 31],
            sp: stack,
            elr: entry,
            spsr: SPSR_EL0T_MASKED,
        }
    }

    /// The syscall number the task trapped with (`x0`).
    #[must_use]
    pub const fn syscall_nr(&self) -> u64 {
        self.x[0]
    }

    /// The first syscall argument (`x1`).
    #[must_use]
    pub const fn arg0(&self) -> u64 {
        self.x[1]
    }

    /// The second syscall argument (`x2`).
    #[must_use]
    pub const fn arg1(&self) -> u64 {
        self.x[2]
    }

    /// Set the value the resumed task sees as its syscall return (`x0`).
    pub fn set_ret(&mut self, value: u64) {
        self.x[0] = value;
    }
}

unsafe extern "C" {
    fn fjord_aarch64_user_run(frame: *mut UserFrame);
}

/// Saved EL1 stack pointer across an EL0 excursion; `el0_sync` restores it.
#[no_mangle]
static mut FJORD_AARCH64_KERNEL_SP: u64 = 0;
/// Pointer to the [`UserFrame`] of the task currently at EL0; `el0_sync` saves
/// the trapping register state through it.
#[no_mangle]
static mut FJORD_AARCH64_CURRENT_FRAME: u64 = 0;

/// Drop to EL0 from `frame` and run until the task issues `svc #0`, which saves
/// the live EL0 state back into `*frame` and returns here. Works for both the
/// first entry and every resume — the frame fully describes the (re)entry point.
///
/// # Safety
/// `frame`'s code/stack must reference EL0-accessible mappings in the live
/// address space, and the boot vector table's lower-EL synchronous slot must
/// route `svc` to `el0_sync` (it does). `frame` must outlive the call.
pub unsafe fn user_run(frame: *mut UserFrame) {
    // SAFETY: see contract; the trampoline saves/restores all callee-saved
    // registers plus DAIF and unwinds through `el0_sync`, having written the
    // trapping state into `*frame`.
    unsafe { fjord_aarch64_user_run(frame) }
}

// EL0 entry trampoline and the lower-EL synchronous-exception handler.
//
// `fjord_aarch64_user_run` stashes the callee-saved registers, the current DAIF
// mask, and the EL1 stack pointer, records the frame pointer, then programs
// SP_EL0, ELR_EL1 and SPSR_EL1 from the frame, loads the user GPRs and `eret`s
// to EL0. The EL0 program runs and executes `svc #0`, trapping into `el0_sync`
// (wired from the boot vector table's "Lower EL, AArch64: Synchronous" slot).
// If the syndrome is an SVC we save the full EL0 register state into the current
// frame, restore DAIF + callee-saved state, and unwind back into the
// `fjord_aarch64_user_run` caller; any other lower-EL synchronous fault falls
// through to the fatal `el1_sync` reporter for debuggability.
global_asm!(
    r#"
.section .text.vectors, "ax"
.global fjord_aarch64_user_run
fjord_aarch64_user_run:
    stp     x19, x20, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x29, x30, [sp, #-16]!
    mrs     x9, daif
    stp     x9, x9, [sp, #-16]!
    adrp    x10, FJORD_AARCH64_KERNEL_SP
    add     x10, x10, #:lo12:FJORD_AARCH64_KERNEL_SP
    mov     x11, sp
    str     x11, [x10]
    adrp    x10, FJORD_AARCH64_CURRENT_FRAME
    add     x10, x10, #:lo12:FJORD_AARCH64_CURRENT_FRAME
    str     x0, [x10]
    // Program the EL0 return state from the frame.
    ldr     x9, [x0, #0xF8]
    msr     sp_el0, x9
    ldr     x9, [x0, #0x100]
    msr     elr_el1, x9
    ldr     x9, [x0, #0x108]
    msr     spsr_el1, x9
    // Load the user GPRs (x0 last, since it is the base pointer).
    ldp     x1, x2,   [x0, #0x08]
    ldp     x3, x4,   [x0, #0x18]
    ldp     x5, x6,   [x0, #0x28]
    ldp     x7, x8,   [x0, #0x38]
    ldp     x9, x10,  [x0, #0x48]
    ldp     x11, x12, [x0, #0x58]
    ldp     x13, x14, [x0, #0x68]
    ldp     x15, x16, [x0, #0x78]
    ldp     x17, x18, [x0, #0x88]
    ldp     x19, x20, [x0, #0x98]
    ldp     x21, x22, [x0, #0xA8]
    ldp     x23, x24, [x0, #0xB8]
    ldp     x25, x26, [x0, #0xC8]
    ldp     x27, x28, [x0, #0xD8]
    ldp     x29, x30, [x0, #0xE8]
    ldr     x0, [x0, #0x00]
    eret

.global el0_sync
el0_sync:
    // Running at EL1 on the kernel stack. Free x9/x10 as scratch (saving the
    // user values), then dispatch on the exception class.
    stp     x9, x10, [sp, #-16]!
    mrs     x9, esr_el1
    lsr     x10, x9, #26
    and     x10, x10, #0x3f
    cmp     x10, #0x15                  // EC == SVC (AArch64)?
    b.ne    2f
    // Save the full EL0 register state into the current frame.
    adrp    x9, FJORD_AARCH64_CURRENT_FRAME
    add     x9, x9, #:lo12:FJORD_AARCH64_CURRENT_FRAME
    ldr     x9, [x9]
    stp     x0, x1,   [x9, #0x00]
    stp     x2, x3,   [x9, #0x10]
    stp     x4, x5,   [x9, #0x20]
    stp     x6, x7,   [x9, #0x30]
    str     x8,       [x9, #0x40]
    str     x11,      [x9, #0x58]
    stp     x12, x13, [x9, #0x60]
    stp     x14, x15, [x9, #0x70]
    stp     x16, x17, [x9, #0x80]
    stp     x18, x19, [x9, #0x90]
    stp     x20, x21, [x9, #0xA0]
    stp     x22, x23, [x9, #0xB0]
    stp     x24, x25, [x9, #0xC0]
    stp     x26, x27, [x9, #0xD0]
    stp     x28, x29, [x9, #0xE0]
    str     x30,      [x9, #0xF0]
    // Recover the user x9/x10 stashed on the kernel stack and store them.
    ldp     x0, x1, [sp]
    str     x0, [x9, #0x48]
    str     x1, [x9, #0x50]
    // System registers: SP_EL0, ELR_EL1 (PC after svc), SPSR_EL1.
    mrs     x0, sp_el0
    str     x0, [x9, #0xF8]
    mrs     x0, elr_el1
    str     x0, [x9, #0x100]
    mrs     x0, spsr_el1
    str     x0, [x9, #0x108]
    add     sp, sp, #16                 // drop the scratch slot
    // Restore kernel callee-saved state and unwind to the user_run caller.
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
2:
    add     sp, sp, #16                 // drop the scratch slot before the fatal path
    b       el1_sync
"#
);
