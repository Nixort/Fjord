// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Tide — the scheduler.
//!
//! MCS-style scheduling contexts make CPU time a capability: a thread runs
//! only against a (budget, period) reservation, which bounds interference and
//! priority inversion. See `docs/ARCHITECTURE.md` §1.

/// A reservation of CPU time (budget refilled every period).
pub struct SchedContext {
    /// Execution budget per period, in microseconds.
    pub budget_us: u64,
    /// Replenishment period, in microseconds.
    pub period_us: u64,
}

/// Pick the next runnable thread.
/// TODO(tide): O(1) ready queues per priority; enforce budgets/refills.
pub fn schedule() { todo!("Tide scheduling decision") }
