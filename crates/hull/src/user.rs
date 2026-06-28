// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 28 june 2026

//! Resumable userspace excursions — the cross-arch seam Keel schedules over.
//!
//! Where [`crate::context`] switches between *kernel* contexts, this switches
//! between the kernel and an *unprivileged* task and back, any number of times.
//! A [`UserFrame`] is the task's resume point: the full ring-3 / EL0 register
//! state captured at its last trap. [`run`] (re)enters the task from a frame and
//! returns to the kernel when the task issues a syscall (`int 0x80` / `svc #0`),
//! having written the trapping state back into the frame. Because the frame
//! fully describes the entry point, the same call handles both the first entry
//! and every resume — the foundation Keel's task/syscall dispatch loop sits on.
//!
//! ## Syscall ABI
//!
//! Mirroring the platform's argument registers, with the *number* in the return
//! register so a resumed task reads its result from the same place:
//!
//! | role        | x86_64 | aarch64 |
//! |-------------|--------|---------|
//! | number      | `rax`  | `x0`    |
//! | argument 0  | `rdi`  | `x1`    |
//! | argument 1  | `rsi`  | `x2`    |
//! | return value| `rax`  | `x0`    |
//!
//! The per-arch register save/restore lives in [`crate::arch`]; this module is
//! the portable facade plus the one piece of shared platform state (the kernel
//! stack the x86 CPU switches to on a privilege-raising trap).

#[cfg(target_arch = "aarch64")]
pub use crate::arch::aarch64::UserFrame;
#[cfg(target_arch = "x86_64")]
pub use crate::arch::x86_64::UserFrame;

/// The `int N` vector the ring-3 syscall traps through on x86_64. aarch64 traps
/// through the architectural `svc` vector and needs no vector number.
#[cfg(target_arch = "x86_64")]
pub const SYSCALL_VECTOR: u8 = 0x80;

#[cfg(target_arch = "x86_64")]
mod platform {
    use core::mem::size_of;

    /// Ring-0 stack the CPU switches to (via `TSS.rsp0`) on every ring-3 → ring-0
    /// trap. One shared stack is correct: a trap unwinds back into the kernel
    /// before the next `run`, so it is never live for two tasks at once.
    #[repr(C, align(16))]
    struct KernelSyscallStack([u8; 32 * 1024]);
    static mut KERNEL_SYSCALL_STACK: KernelSyscallStack = KernelSyscallStack([0; 32 * 1024]);

    /// Install the syscall trap gate and point `TSS.rsp0` at the kernel syscall
    /// stack. Idempotent; call once during early boot before the first [`run`].
    pub fn init() {
        // SAFETY: single-core early boot owns the TSS/IDT; the stack is a static
        // that outlives every userspace transition.
        unsafe {
            let base = &raw const KERNEL_SYSCALL_STACK as *const u8;
            let top = base.add(size_of::<KernelSyscallStack>()) as u64;
            crate::arch::x86_64::set_kernel_stack(top);
            crate::arch::x86_64::install_syscall_gate(super::SYSCALL_VECTOR);
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod platform {
    /// The boot vector table already routes `svc` from EL0 to `el0_sync`, and
    /// the EL1 handler runs on the active kernel stack, so there is nothing to
    /// set up. Present for API symmetry with x86_64.
    pub fn init() {}
}

/// Prepare the resumable user path. Call once during early boot before [`run`].
pub fn init() {
    platform::init();
}

/// Drop into (or resume) the task described by `frame` and run until it traps
/// via a syscall, at which point the trapping register state is written back
/// into `*frame` and control returns here.
///
/// # Safety
/// [`init`] must have run. The code/stack pages the frame names must be mapped
/// user-accessible (code executable, stack writable + non-executable) in the
/// active address space, and `frame` must remain valid for the call.
pub unsafe fn run(frame: *mut UserFrame) {
    // SAFETY: contract forwarded to the caller; the per-arch routine performs
    // the privilege transition and writes the trap state back into `*frame`.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        crate::arch::x86_64::user_run(frame);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::aarch64::user_run(frame);
    }
}
