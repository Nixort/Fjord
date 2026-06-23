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
