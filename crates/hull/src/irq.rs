// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Interrupt control: CPU-level interrupt masking plus (eventually) the
//! interrupt-controller abstraction (APIC on x86_64, GIC on aarch64).
//!
//! For now this provides [`lock`], an RAII critical section that masks IRQ
//! delivery on the current CPU and restores the previous state on drop. It is
//! the smallest primitive needed to make a multi-byte console write atomic with
//! respect to the timer ISR, and to fence a cooperative context switch.
//!
//! TODO(hull): controller-level mask/unmask, EOI, and routing IRQs to userspace
//! driver capabilities.

/// An RAII critical section: interrupt delivery is masked on the current CPU
/// for as long as the guard is alive, then restored to its previous state.
///
/// Nesting is sound: each guard saves and restores the *prior* interrupt-enable
/// state, so an inner guard taken while interrupts are already masked (for
/// example from within an ISR) correctly leaves them masked on drop.
#[must_use = "interrupts are unmasked again as soon as the guard is dropped"]
pub struct IrqGuard(u64);

impl Drop for IrqGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the token produced by `mask_and_save` on this CPU.
        unsafe { restore(self.0) }
    }
}

/// Mask interrupt delivery on the current CPU until the returned guard drops.
#[must_use = "interrupts are unmasked again as soon as the guard is dropped"]
pub fn lock() -> IrqGuard {
    // SAFETY: reading and masking the per-CPU interrupt-enable state is sound;
    // the returned token restores exactly the prior state on drop.
    IrqGuard(unsafe { mask_and_save() })
}

/// Unconditionally enable IRQ delivery on the current CPU.
///
/// Unlike [`lock`], this neither saves nor restores prior state: it simply
/// unmasks interrupts. It exists for one narrow purpose -- the entry point of a
/// freshly built scheduler context. Such a context is first entered via a
/// cooperative [`crate::context::switch`] performed *inside* the timer ISR, so
/// it begins life with interrupts still masked (the ISR epilogue, which would
/// have restored them, never ran for this stack). The new thread must therefore
/// re-enable interrupts itself, or it could never be preempted.
///
/// # Safety
/// The caller must be ready to take interrupts immediately: a valid stack and
/// installed interrupt handlers. Intended only for scheduler context entry.
#[cfg(target_arch = "x86_64")]
pub unsafe fn force_enable() {
    // SAFETY: enabling IF on a CPU with a loaded IDT and a valid stack is sound.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
}

/// See the x86_64 [`force_enable`].
///
/// # Safety
/// See the x86_64 [`force_enable`].
#[cfg(target_arch = "aarch64")]
pub unsafe fn force_enable() {
    // SAFETY: clearing DAIF.I unmasks IRQs; the GIC and vectors are already up.
    unsafe {
        core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags));
    }
}

/// See the x86_64 [`force_enable`].
///
/// # Safety
/// See the x86_64 [`force_enable`].
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub unsafe fn force_enable() {}

#[cfg(target_arch = "x86_64")]
unsafe fn mask_and_save() -> u64 {
    let flags: u64;
    // SAFETY: pushfq/pop copy RFLAGS into a scratch register; cli then clears
    // IF. The sequence touches only the machine stack and the output register.
    unsafe {
        core::arch::asm!("pushfq", "pop {f}", "cli", f = out(reg) flags, options(nomem));
    }
    flags
}

#[cfg(target_arch = "x86_64")]
unsafe fn restore(token: u64) {
    // Re-enable interrupts only if IF (RFLAGS bit 9) was set when we masked.
    if token & (1 << 9) != 0 {
        // SAFETY: restoring the prior interrupt-enable state on this CPU.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn mask_and_save() -> u64 {
    let daif: u64;
    // SAFETY: snapshot DAIF, then set the I bit (`daifset #2`) to mask IRQs.
    unsafe {
        core::arch::asm!(
            "mrs {d}, daif",
            "msr daifset, #2",
            d = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
    }
    daif
}

#[cfg(target_arch = "aarch64")]
unsafe fn restore(token: u64) {
    // SAFETY: restore the exact DAIF snapshot taken in `mask_and_save`.
    unsafe {
        core::arch::asm!(
            "msr daif, {d}",
            d = in(reg) token,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
unsafe fn mask_and_save() -> u64 {
    0
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
unsafe fn restore(_token: u64) {}
