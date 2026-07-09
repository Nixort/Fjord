// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Monotonic timer + one-shot deadlines for the Tide scheduler.
//!
//! Architecture timer drivers (`apic` on x86_64 and `gic` on aarch64) own the
//! hardware programming. This module provides the small architecture-neutral
//! state machine they share: a monotonic tick counter and one armed deadline.
//! It is intentionally conservative and heap-free, so it can be used during
//! early boot and from interrupt context.

use core::sync::atomic::{AtomicU64, Ordering};

/// No deadline is currently armed.
pub const NO_DEADLINE: u64 = u64::MAX;

static NOW_TICKS: AtomicU64 = AtomicU64::new(0);
static DEADLINE_TICKS: AtomicU64 = AtomicU64::new(NO_DEADLINE);

/// Returns the architecture-neutral monotonic tick counter.
#[must_use]
pub fn now_ticks() -> u64 {
    NOW_TICKS.load(Ordering::Acquire)
}

/// Advances the monotonic counter by `delta` ticks.
///
/// Interrupt drivers call this from the timer ISR after acknowledging the
/// hardware interrupt. Saturating arithmetic preserves monotonicity even if an
/// emulated timer accidentally reports a very large delta.
pub fn advance_ticks(delta: u64) -> u64 {
    let mut cur = NOW_TICKS.load(Ordering::Relaxed);
    loop {
        let next = cur.saturating_add(delta);
        match NOW_TICKS.compare_exchange_weak(cur, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(actual) => cur = actual,
        }
    }
}

/// Arms a one-shot deadline at an absolute tick value.
pub fn arm_at(deadline: u64) {
    DEADLINE_TICKS.store(deadline, Ordering::Release);
}

/// Arms a one-shot deadline `delta` ticks from the current counter.
pub fn arm_after(delta: u64) -> u64 {
    let deadline = now_ticks().saturating_add(delta);
    arm_at(deadline);
    deadline
}

/// Disarms the current one-shot deadline.
pub fn disarm() {
    DEADLINE_TICKS.store(NO_DEADLINE, Ordering::Release);
}

/// Returns the currently armed deadline, if any.
#[must_use]
pub fn deadline() -> Option<u64> {
    match DEADLINE_TICKS.load(Ordering::Acquire) {
        NO_DEADLINE => None,
        tick => Some(tick),
    }
}

/// Returns whether the armed deadline has expired at the current tick.
#[must_use]
pub fn expired() -> bool {
    match deadline() {
        Some(deadline) => now_ticks() >= deadline,
        None => false,
    }
}

/// Atomically consumes an expired deadline.
///
/// Returns the expired deadline tick once; later calls return `None` until a new
/// deadline is armed. This is what the scheduler tick path should use before it
/// wakes a timed-out thread.
pub fn take_if_expired() -> Option<u64> {
    let deadline = deadline()?;
    if now_ticks() < deadline {
        return None;
    }
    match DEADLINE_TICKS.compare_exchange(
        deadline,
        NO_DEADLINE,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => Some(deadline),
        Err(_) => None,
    }
}

/// Boot-time self-test for the software timer state machine.
pub fn selftest() -> Result<(), ()> {
    disarm();
    let start = now_ticks();
    let deadline = arm_after(3);
    if expired() || deadline < start {
        return Err(());
    }
    advance_ticks(2);
    if expired() || take_if_expired().is_some() {
        return Err(());
    }
    advance_ticks(1);
    if !expired() || take_if_expired() != Some(deadline) || deadline().is_some() {
        return Err(());
    }
    Ok(())
}
