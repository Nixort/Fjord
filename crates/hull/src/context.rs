// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! Low-level cooperative CPU context switching.
//!
//! A [`Context`] captures exactly the callee-saved CPU state a *voluntary*
//! switch must preserve (the ABI lets a function clobber the rest). [`switch`]
//! saves the live state into one context and resumes another; [`init_context`]
//! seeds a fresh context so its first switch begins executing an entry function
//! on its own stack.
//!
//! This is the mechanism the Tide scheduler's preemptive handoff is layered on:
//! the timer ISR will eventually pick the next thread and `switch` into it. The
//! switch routine itself is written in assembly (via `global_asm!`, mirroring
//! the ISR stubs in [`crate::arch`]) because it must run with a hand-rolled
//! calling convention that Rust cannot express directly.
//!
//! Both backends save: the stack pointer plus the architecture's callee-saved
//! general registers. On x86_64 the resume address rides on the saved stack
//! (popped by `ret`); on aarch64 it rides in the saved link register `lr`.

#[cfg(target_arch = "x86_64")]
pub use self::x86::{init_context, switch, Context};

#[cfg(target_arch = "aarch64")]
pub use self::arm::{init_context, switch, Context};

#[cfg(target_arch = "x86_64")]
mod x86 {
    use core::arch::global_asm;
    use core::mem::offset_of;

    /// Saved callee-saved CPU state for a cooperative x86_64 context switch.
    ///
    /// The System V ABI lets a function freely clobber the caller-saved
    /// registers, so a voluntary switch only needs to preserve `rbx`, `rbp`,
    /// `r12`–`r15` and the stack pointer. The instruction pointer is implicit:
    /// [`switch`] ends in `ret`, returning into whatever the restored `rsp`
    /// points at.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Context {
        /// Saved stack pointer; the rest of the resume state hangs off it.
        sp: u64,
        rbx: u64,
        rbp: u64,
        r12: u64,
        r13: u64,
        r14: u64,
        r15: u64,
    }

    impl Context {
        /// A fully zeroed context. Pair with [`init_context`] before the first
        /// [`switch`] into it.
        #[must_use]
        pub const fn zeroed() -> Self {
            Self { sp: 0, rbx: 0, rbp: 0, r12: 0, r13: 0, r14: 0, r15: 0 }
        }
    }

    // The assembly below addresses these fields by byte offset; if the layout
    // ever changes, these compile-time checks fail before anything boots.
    const _: () = assert!(offset_of!(Context, sp) == 0x00);
    const _: () = assert!(offset_of!(Context, rbx) == 0x08);
    const _: () = assert!(offset_of!(Context, rbp) == 0x10);
    const _: () = assert!(offset_of!(Context, r12) == 0x18);
    const _: () = assert!(offset_of!(Context, r13) == 0x20);
    const _: () = assert!(offset_of!(Context, r14) == 0x28);
    const _: () = assert!(offset_of!(Context, r15) == 0x30);

    unsafe extern "C" {
        fn fjord_ctx_switch(prev: *mut Context, next: *const Context);
    }

    /// Switch CPU context: save the live callee-saved state into `*prev`, then
    /// load and resume `*next`.
    ///
    /// Control does not return to the caller here; it resumes wherever `*next`
    /// was suspended (or at its entry function, for a freshly [`init_context`]ed
    /// context). The original caller resumes only when some context later
    /// switches back into `*prev`.
    ///
    /// # Safety
    /// `prev` and `next` must point to valid, distinct `Context`s. `*next` must
    /// have been produced by [`init_context`] or by a prior `switch` that saved
    /// into it, and the stack it names must still be live.
    pub unsafe fn switch(prev: *mut Context, next: *const Context) {
        // SAFETY: the contract is forwarded to the caller; the routine only
        // touches the two contexts and the two stacks they describe.
        unsafe { fjord_ctx_switch(prev, next) }
    }

    /// Seed `ctx` so the first [`switch`] into it starts executing `entry` on a
    /// fresh stack whose exclusive top is `stack_top`.
    ///
    /// # Safety
    /// `ctx` must be writable. `stack_top` must be the top address of an owned
    /// stack region with at least a few KiB of usable space below it; `entry`
    /// runs on that stack and must never return.
    pub unsafe fn init_context(
        ctx: *mut Context,
        stack_top: usize,
        entry: extern "C" fn() -> !,
    ) {
        // 16-byte align the stack and place the entry address so the switch's
        // terminating `ret` pops it into rip. After that `ret`, rsp == sp + 8,
        // matching the (rsp % 16 == 8) state the ABI expects at a fn entry.
        let sp = (stack_top & !0xF) - 16;
        // SAFETY: `sp` lies inside the caller-owned stack; `ctx` is writable.
        unsafe {
            core::ptr::write(sp as *mut u64, entry as usize as u64);
            core::ptr::write(ctx, Context::zeroed());
            (*ctx).sp = sp as u64;
        }
    }

    global_asm!(
        r#"
.global fjord_ctx_switch
fjord_ctx_switch:
    mov [rdi + 0x00], rsp
    mov [rdi + 0x08], rbx
    mov [rdi + 0x10], rbp
    mov [rdi + 0x18], r12
    mov [rdi + 0x20], r13
    mov [rdi + 0x28], r14
    mov [rdi + 0x30], r15
    mov rsp, [rsi + 0x00]
    mov rbx, [rsi + 0x08]
    mov rbp, [rsi + 0x10]
    mov r12, [rsi + 0x18]
    mov r13, [rsi + 0x20]
    mov r14, [rsi + 0x28]
    mov r15, [rsi + 0x30]
    ret
"#
    );
}

#[cfg(target_arch = "aarch64")]
mod arm {
    use core::arch::global_asm;
    use core::mem::offset_of;

    /// Saved callee-saved CPU state for a cooperative aarch64 context switch.
    ///
    /// AAPCS64 makes `x19`–`x28`, the frame pointer `x29` and the link register
    /// `x30` callee-saved, alongside the stack pointer. Unlike x86, the resume
    /// address lives in a register (`lr`/`x30`), so a fresh context simply sets
    /// `lr` to its entry function; [`switch`] ends in `ret`, branching to it.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct Context {
        /// Saved stack pointer (must stay 16-byte aligned).
        sp: u64,
        x19: u64,
        x20: u64,
        x21: u64,
        x22: u64,
        x23: u64,
        x24: u64,
        x25: u64,
        x26: u64,
        x27: u64,
        x28: u64,
        /// Frame pointer (`x29`).
        fp: u64,
        /// Link register (`x30`): the address `ret` branches to on resume.
        lr: u64,
    }

    impl Context {
        /// A fully zeroed context. Pair with [`init_context`] before the first
        /// [`switch`] into it.
        #[must_use]
        pub const fn zeroed() -> Self {
            Self {
                sp: 0,
                x19: 0,
                x20: 0,
                x21: 0,
                x22: 0,
                x23: 0,
                x24: 0,
                x25: 0,
                x26: 0,
                x27: 0,
                x28: 0,
                fp: 0,
                lr: 0,
            }
        }
    }

    // The assembly below addresses these fields by byte offset; if the layout
    // ever changes, these compile-time checks fail before anything boots.
    const _: () = assert!(offset_of!(Context, sp) == 0x00);
    const _: () = assert!(offset_of!(Context, x19) == 0x08);
    const _: () = assert!(offset_of!(Context, fp) == 0x58);
    const _: () = assert!(offset_of!(Context, lr) == 0x60);

    unsafe extern "C" {
        fn fjord_ctx_switch(prev: *mut Context, next: *const Context);
    }

    /// Switch CPU context: save the live callee-saved state into `*prev`, then
    /// load and resume `*next`.
    ///
    /// Control does not return to the caller here; it resumes wherever `*next`
    /// was suspended (or at its entry function, for a freshly [`init_context`]ed
    /// context). The original caller resumes only when some context later
    /// switches back into `*prev`.
    ///
    /// # Safety
    /// `prev` and `next` must point to valid, distinct `Context`s. `*next` must
    /// have been produced by [`init_context`] or by a prior `switch` that saved
    /// into it, and the stack it names must still be live.
    pub unsafe fn switch(prev: *mut Context, next: *const Context) {
        // SAFETY: the contract is forwarded to the caller; the routine only
        // touches the two contexts and the two stacks they describe.
        unsafe { fjord_ctx_switch(prev, next) }
    }

    /// Seed `ctx` so the first [`switch`] into it starts executing `entry` on a
    /// fresh stack whose exclusive top is `stack_top`.
    ///
    /// # Safety
    /// `ctx` must be writable. `stack_top` must be the top address of an owned
    /// stack region with at least a few KiB of usable space below it; `entry`
    /// runs on that stack and must never return.
    pub unsafe fn init_context(
        ctx: *mut Context,
        stack_top: usize,
        entry: extern "C" fn() -> !,
    ) {
        // aarch64 requires a 16-byte aligned SP; the entry address rides in lr.
        let sp = stack_top & !0xF;
        // SAFETY: `ctx` is writable and `sp` names the top of an owned stack.
        unsafe {
            core::ptr::write(ctx, Context::zeroed());
            (*ctx).sp = sp as u64;
            (*ctx).lr = entry as usize as u64;
        }
    }

    global_asm!(
        r#"
.global fjord_ctx_switch
fjord_ctx_switch:
    mov x2, sp
    str x2,  [x0, #0x00]
    stp x19, x20, [x0, #0x08]
    stp x21, x22, [x0, #0x18]
    stp x23, x24, [x0, #0x28]
    stp x25, x26, [x0, #0x38]
    stp x27, x28, [x0, #0x48]
    stp x29, x30, [x0, #0x58]
    ldr x2,  [x1, #0x00]
    mov sp, x2
    ldp x19, x20, [x1, #0x08]
    ldp x21, x22, [x1, #0x18]
    ldp x23, x24, [x1, #0x28]
    ldp x25, x26, [x1, #0x38]
    ldp x27, x28, [x1, #0x48]
    ldp x29, x30, [x1, #0x58]
    ret
"#
    );
}
