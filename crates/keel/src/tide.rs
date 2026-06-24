// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 24 june 2026

//! The scheduler: priority dispatch with MCS scheduling contexts.
//!
//! Each thread carries a [`SchedContext`] granting it `budget` ticks of
//! execution every `period` ticks. The [`Scheduler`] always runs the
//! highest-priority thread that is `Ready` and still has budget. When the
//! running thread exhausts its budget it is passed over until its context is
//! replenished at the next period boundary, so a busy high-priority thread
//! cannot starve the rest of the system — the core MCS guarantee.
//!
//! Heap-free: thread storage is a caller-owned `&mut [Thread]`. Real context
//! switching (register save/restore, `tide` <-> `hull` handoff) and the wakeup
//! wiring to `ipc` are layered on top of this dispatch core.
//!
//! See `docs/ARCHITECTURE.md` §1.

/// A mixed-criticality scheduling context: a periodic execution budget.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SchedContext {
    budget: u64,
    period: u64,
    remaining: u64,
}

impl SchedContext {
    /// A context granting `budget` ticks of execution every `period` ticks.
    /// The budget starts full.
    #[must_use]
    pub const fn new(budget: u64, period: u64) -> Self {
        Self {
            budget,
            period,
            remaining: budget,
        }
    }

    /// Ticks of execution granted per period.
    #[must_use]
    pub const fn budget(self) -> u64 {
        self.budget
    }

    /// Replenishment period in ticks (`0` means never replenished).
    #[must_use]
    pub const fn period(self) -> u64 {
        self.period
    }

    /// Budget left in the current period.
    #[must_use]
    pub const fn remaining(self) -> u64 {
        self.remaining
    }

    /// Whether the budget for this period is spent.
    #[must_use]
    pub const fn depleted(self) -> bool {
        self.remaining == 0
    }
}

/// The run state of a scheduler slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RunState {
    /// An empty slot, holding no thread.
    #[default]
    Inactive,
    /// Runnable: eligible to be dispatched (budget permitting).
    Ready,
    /// Blocked (e.g. waiting on an endpoint); not dispatchable.
    Blocked,
}

/// A schedulable thread: an id, a priority, a budget, and a run state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Thread {
    id: u64,
    prio: u8,
    sc: SchedContext,
    state: RunState,
}

impl Thread {
    /// The thread's identifier.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.id
    }

    /// The thread's static priority (higher wins).
    #[must_use]
    pub const fn prio(self) -> u8 {
        self.prio
    }

    /// The thread's scheduling context.
    #[must_use]
    pub const fn sched_context(self) -> SchedContext {
        self.sc
    }

    /// The thread's run state.
    #[must_use]
    pub const fn state(self) -> RunState {
        self.state
    }
}

/// Why a scheduler operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchedError {
    /// No free thread slot remains.
    Full,
    /// No admitted thread has the given id.
    NotFound,
}

/// A priority dispatcher over a caller-owned table of threads.
pub struct Scheduler<'t> {
    threads: &'t mut [Thread],
    current: Option<usize>,
    ticks: u64,
}

impl<'t> Scheduler<'t> {
    /// Wrap a slice of thread storage, clearing every slot to inactive.
    #[must_use]
    pub fn new(threads: &'t mut [Thread]) -> Self {
        for t in threads.iter_mut() {
            *t = Thread::default();
        }
        Self {
            threads,
            current: None,
            ticks: 0,
        }
    }

    /// Maximum number of threads this scheduler can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.threads.len()
    }

    /// Number of admitted (non-inactive) threads.
    #[must_use]
    pub fn count(&self) -> usize {
        self.threads.iter().filter(|t| t.state != RunState::Inactive).count()
    }

    /// Total ticks elapsed since construction.
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The id of the currently dispatched thread, if any.
    #[must_use]
    pub fn current(&self) -> Option<u64> {
        self.current.map(|i| self.threads[i].id)
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.threads
            .iter()
            .position(|t| t.state != RunState::Inactive && t.id == id)
    }

    /// Admit a thread with the given id, priority, and scheduling context.
    /// It starts `Ready`.
    ///
    /// # Errors
    /// Returns [`SchedError::Full`] if no free slot remains.
    pub fn admit(&mut self, id: u64, prio: u8, sc: SchedContext) -> Result<usize, SchedError> {
        let slot = self
            .threads
            .iter()
            .position(|t| t.state == RunState::Inactive)
            .ok_or(SchedError::Full)?;
        self.threads[slot] = Thread {
            id,
            prio,
            sc,
            state: RunState::Ready,
        };
        Ok(slot)
    }

    /// Mark the thread `id` as blocked (not dispatchable).
    ///
    /// # Errors
    /// Returns [`SchedError::NotFound`] if no such thread is admitted.
    pub fn block(&mut self, id: u64) -> Result<(), SchedError> {
        let idx = self.index_of(id).ok_or(SchedError::NotFound)?;
        self.threads[idx].state = RunState::Blocked;
        Ok(())
    }

    /// Mark the thread `id` as ready (dispatchable).
    ///
    /// # Errors
    /// Returns [`SchedError::NotFound`] if no such thread is admitted.
    pub fn unblock(&mut self, id: u64) -> Result<(), SchedError> {
        let idx = self.index_of(id).ok_or(SchedError::NotFound)?;
        self.threads[idx].state = RunState::Ready;
        Ok(())
    }

    /// Pick the highest-priority `Ready` thread that still has budget. Ties go
    /// to the lowest slot index (a simple round-robin seed).
    fn pick(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (i, t) in self.threads.iter().enumerate() {
            if t.state == RunState::Ready && t.sc.remaining > 0 {
                match best {
                    None => best = Some(i),
                    Some(b) if t.prio > self.threads[b].prio => best = Some(i),
                    Some(_) => {}
                }
            }
        }
        best
    }

    /// Recompute and return the thread that should run now.
    pub fn schedule(&mut self) -> Option<u64> {
        self.current = self.pick();
        self.current()
    }

    /// Advance time by one tick: charge the running thread a unit of budget,
    /// replenish any context at its period boundary, then redispatch. Returns
    /// the thread that should run after the tick.
    pub fn tick(&mut self) -> Option<u64> {
        self.ticks += 1;

        if let Some(i) = self.current {
            let sc = &mut self.threads[i].sc;
            sc.remaining = sc.remaining.saturating_sub(1);
        }

        let now = self.ticks;
        for t in self.threads.iter_mut() {
            if t.state != RunState::Inactive && t.sc.period != 0 && now % t.sc.period == 0 {
                t.sc.remaining = t.sc.budget;
            }
        }

        self.schedule()
    }
}

/// Boot-time self-test exercising priority dispatch, budget depletion and
/// replenishment, and block/unblock.
///
/// # Errors
/// Returns a [`SchedError`] (used as a failure sentinel) if any invariant fails.
pub fn selftest() -> Result<(), SchedError> {
    let mut threads = [Thread::default(); 4];
    let mut sched = Scheduler::new(&mut threads);

    // A: high priority, tiny budget (2 ticks every 4).
    // B: low priority, ample budget.
    let a = sched.admit(1, 10, SchedContext::new(2, 4))?;
    let b = sched.admit(2, 5, SchedContext::new(10, 100))?;
    if a == b || sched.count() != 2 {
        return Err(SchedError::Full);
    }

    // Highest priority is dispatched first.
    if sched.schedule() != Some(1) {
        return Err(SchedError::Full);
    }

    // A runs for its 2-tick budget, depletes, B takes over, then A is
    // replenished at its period boundary (tick 4) and preempts B again.
    if sched.tick() != Some(1) // tick 1: A still has budget
        || sched.tick() != Some(2) // tick 2: A depleted -> B runs
        || sched.tick() != Some(2) // tick 3: B continues
        || sched.tick() != Some(1)
    // tick 4: A replenished -> preempts
    {
        return Err(SchedError::Full);
    }

    // Blocking the current thread yields to the next eligible one.
    sched.block(1)?;
    if sched.schedule() != Some(2) {
        return Err(SchedError::Full);
    }

    // Unblocking restores priority order.
    sched.unblock(1)?;
    if sched.schedule() != Some(1) {
        return Err(SchedError::Full);
    }

    // An unknown thread id is reported, not silently ignored.
    if sched.block(99) != Err(SchedError::NotFound) {
        return Err(SchedError::Full);
    }

    Ok(())
}
