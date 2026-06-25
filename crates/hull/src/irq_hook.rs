// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 25 june 2026

//! An IRQ-dispatch hook: the inversion-of-control seam that lets Keel deliver
//! hardware interrupts as capability-backed notifications to userspace driver
//! threads, without Hull ever depending on Keel.
//!
//! Hull owns the interrupt controllers ([`crate::apic`] / [`crate::gic`]); it
//! must not call up into the capability layer directly. Instead Keel registers
//! an `extern "C" fn(u32)` dispatch callback here with [`set_irq_hook`], and
//! each platform timer dispatcher invokes [`run_irq_hook`] *after* EOI and
//! *after* the scheduler tick hook -- so the interrupt controller is in a clean
//! state and any preemptive context switch has already happened. The hook
//! receives the architecture-specific interrupt number (LAPIC vector on
//! x86_64, GIC INTID on aarch64) so Keel can route it to the right
//! [`Notification`] badge.
//!
//! The hook is stored as a raw function-pointer address in an [`AtomicUsize`]:
//! `0` means "no hook installed" and the dispatcher does nothing. Registration
//! and teardown happen on the single boot CPU with interrupts effectively
//! quiesced, so there is no genuine concurrency; the atomic is purely for the
//! soundness of the ISR-side read.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Installed IRQ-dispatch hook as a raw `extern "C" fn(u32)` address, or `0`.
static IRQ_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Install the IRQ-dispatch hook. Called by Keel before it begins driving
/// interrupts as capabilities; replaces any previously installed hook.
pub fn set_irq_hook(hook: extern "C" fn(u32)) {
    IRQ_HOOK.store(hook as usize, Ordering::SeqCst);
}

/// Remove the IRQ-dispatch hook so subsequent interrupts do not reach Keel.
pub fn clear_irq_hook() {
    IRQ_HOOK.store(0, Ordering::SeqCst);
}

/// Invoke the installed IRQ-dispatch hook with `irq`, if any. Called by the
/// timer dispatchers as their last action, after EOI and after the scheduler
/// tick hook.
///
/// Unlike the tick hook, this hook must *not* perform a context switch: it is
/// a non-blocking notification delivery. The hook signals a [`Notification`]
/// (OR-ing a badge word) and returns; the driver thread wakes later when the
/// scheduler dispatches it.
#[inline]
pub fn run_irq_hook(irq: u32) {
    let raw = IRQ_HOOK.load(Ordering::SeqCst);
    if raw != 0 {
        // SAFETY: `raw` is non-zero only after `set_irq_hook` stored the
        // address of a real `extern "C" fn(u32)`; the transmute reconstructs
        // that exact function-pointer type from its address.
        let hook: extern "C" fn(u32) =
            unsafe { core::mem::transmute::<usize, extern "C" fn(u32)>(raw) };
        hook(irq);
    }
}
