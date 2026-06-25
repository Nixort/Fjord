// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! A single timer-tick hook: the inversion-of-control seam that lets the
//! scheduler (which lives *above* Hull, in `keel::tide`) preempt the running
//! context from the platform timer ISR without Hull ever depending on Keel.
//!
//! Hull owns the interrupt controllers ([`crate::apic`] / [`crate::gic`]); it
//! must not call up into the scheduler directly. Instead Keel registers an
//! `extern "C"` callback here with [`set_tick_hook`], and each timer dispatcher
//! invokes [`run_tick_hook`] as its very last action -- *after* the
//! end-of-interrupt write -- so a context switch performed inside the hook
//! leaves the interrupt controller in a clean, re-armable state.
//!
//! The hook is stored as a raw function-pointer address in an [`AtomicUsize`]:
//! `0` means "no hook installed" and the dispatcher does nothing. Registration
//! and teardown happen on the single boot CPU with interrupts effectively
//! quiesced, so there is no genuine concurrency; the atomic is purely for the
//! soundness of the ISR-side read.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Installed tick hook as a raw `extern "C" fn()` address, or `0` if none.
static TICK_HOOK: AtomicUsize = AtomicUsize::new(0);

/// Install the per-tick scheduler hook. Called by Keel before it begins
/// driving the timer; replaces any previously installed hook.
pub fn set_tick_hook(hook: extern "C" fn()) {
    TICK_HOOK.store(hook as usize, Ordering::SeqCst);
}

/// Remove the tick hook so subsequent ticks are no-ops at the Hull seam.
pub fn clear_tick_hook() {
    TICK_HOOK.store(0, Ordering::SeqCst);
}

/// Invoke the installed tick hook, if any. Called by the timer dispatchers as
/// their last action, after EOI.
///
/// The installed hook may perform a context switch and therefore not return to
/// the dispatcher until *this* thread is later resumed; that is the entire
/// point. When no hook is installed this is a single relaxed-ordering load and
/// a branch.
#[inline]
pub fn run_tick_hook() {
    let raw = TICK_HOOK.load(Ordering::SeqCst);
    if raw != 0 {
        // SAFETY: `raw` is non-zero only after `set_tick_hook` stored the
        // address of a real `extern "C" fn()`; the transmute reconstructs that
        // exact function-pointer type from its address.
        let hook: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(raw) };
        hook();
    }
}
