// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// The code was written for Fjord.

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

/// One future grant of CPU budget in a bounded scheduling context queue.
///
/// A refill becomes consumable at `eligible_at`. Fields stay private so the
/// queue alone can preserve temporal ordering and prevent an earlier refill
/// from being fabricated by a caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refill {
    amount: u64,
    eligible_at: u64,
}

impl Refill {
    const EMPTY: Self = Self {
        amount: 0,
        eligible_at: 0,
    };

    /// Amount of CPU time restored by this refill.
    #[must_use]
    pub const fn amount(self) -> u64 {
        self.amount
    }

    /// Earliest tick at which this refill is eligible to run.
    #[must_use]
    pub const fn eligible_at(self) -> u64 {
        self.eligible_at
    }
}

/// Why a bounded replenishment operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefillError {
    /// The supplied storage has no slots.
    NoStorage,
    /// Budget or period is zero, or budget exceeds period.
    InvalidParameters,
    /// Consuming zero ticks is not a meaningful accounting operation.
    ZeroAmount,
    /// No refill is currently eligible at the requested time.
    NotEligible,
    /// The requested charge exceeds the head refill's available budget.
    InsufficientBudget,
    /// `now + period` overflowed the monotonic tick domain.
    TimeOverflow,
}

/// Heap-free sporadic-server replenishment queue.
///
/// A scheduling context begins with one immediately eligible refill equal to
/// its full budget. Each successful consumption schedules the same amount for
/// `now + period`. Refill storage is caller-owned and bounded. If the queue is
/// full, the implementation merges work into the latest future refill; when the
/// only slot is the currently eligible one, it postpones the remaining head as
/// well. Both cases delay availability rather than creating earlier budget, so
/// saturation may reduce service but never breaks the execution bound.
pub struct ReplenishmentQueue<'r> {
    refills: &'r mut [Refill],
    head: usize,
    len: usize,
    budget: u64,
    period: u64,
}

impl<'r> ReplenishmentQueue<'r> {
    /// Constructs a scheduling-context queue with an initial refill of `budget`
    /// eligible at `now`.
    pub fn new(
        refills: &'r mut [Refill],
        budget: u64,
        period: u64,
        now: u64,
    ) -> Result<Self, RefillError> {
        if refills.is_empty() {
            return Err(RefillError::NoStorage);
        }
        if budget == 0 || period == 0 || budget > period {
            return Err(RefillError::InvalidParameters);
        }
        for refill in refills.iter_mut() {
            *refill = Refill::EMPTY;
        }
        refills[0] = Refill {
            amount: budget,
            eligible_at: now,
        };
        Ok(Self {
            refills,
            head: 0,
            len: 1,
            budget,
            period,
        })
    }

    /// Configured maximum execution budget.
    #[must_use]
    pub const fn budget(&self) -> u64 {
        self.budget
    }

    /// Configured replenishment period.
    #[must_use]
    pub const fn period(&self) -> u64 {
        self.period
    }

    /// Number of live refill entries currently retained.
    #[must_use]
    pub const fn refill_count(&self) -> usize {
        self.len
    }

    /// The current head refill, if one remains.
    #[must_use]
    pub fn head_refill(&self) -> Option<Refill> {
        (self.len != 0).then(|| self.refills[self.head])
    }

    /// Budget immediately eligible for consumption at `now`.
    #[must_use]
    pub fn available(&self, now: u64) -> u64 {
        self.head_refill()
            .filter(|refill| refill.eligible_at <= now)
            .map_or(0, |refill| refill.amount)
    }

    /// Charge `amount` ticks at `now` and schedule their future refill.
    ///
    /// A charge is atomic with respect to validation: invalid amount, ineligible
    /// time, insufficient head budget and timestamp overflow leave queue state
    /// untouched. On bounded-storage saturation the queue delays availability
    /// rather than granting budget earlier than its eligibility time.
    pub fn consume(&mut self, now: u64, amount: u64) -> Result<(), RefillError> {
        if amount == 0 {
            return Err(RefillError::ZeroAmount);
        }
        let eligible = self.head_refill().ok_or(RefillError::NotEligible)?;
        if eligible.eligible_at > now {
            return Err(RefillError::NotEligible);
        }
        if amount > eligible.amount {
            return Err(RefillError::InsufficientBudget);
        }
        let refill_at = now
            .checked_add(self.period)
            .ok_or(RefillError::TimeOverflow)?;

        self.refills[self.head].amount -= amount;
        if self.refills[self.head].amount == 0 {
            self.pop_head();
        }
        self.push_or_defer(Refill {
            amount,
            eligible_at: refill_at,
        });
        Ok(())
    }

    fn pop_head(&mut self) {
        debug_assert!(self.len > 0);
        self.refills[self.head] = Refill::EMPTY;
        self.head = (self.head + 1) % self.refills.len();
        self.len -= 1;
    }

    fn tail_index(&self) -> usize {
        (self.head + self.len - 1) % self.refills.len()
    }

    fn push_or_defer(&mut self, refill: Refill) {
        debug_assert!(refill.amount > 0);
        if self.len == 0 {
            self.refills[self.head] = refill;
            self.len = 1;
            return;
        }

        let tail = self.tail_index();
        if self.refills[tail].eligible_at == refill.eligible_at {
            self.refills[tail].amount = self.refills[tail].amount.saturating_add(refill.amount);
            return;
        }
        if self.len < self.refills.len() {
            let slot = (self.head + self.len) % self.refills.len();
            self.refills[slot] = refill;
            self.len += 1;
            return;
        }

        // No spare refill entry. A future tail can absorb the new refill without
        // becoming earlier. If the only/full tail is currently eligible, move its
        // remaining budget to the new later deadline too; postponement is safe.
        if self.refills[tail].eligible_at < refill.eligible_at {
            self.refills[tail].eligible_at = refill.eligible_at;
        }
        self.refills[tail].amount = self.refills[tail].amount.saturating_add(refill.amount);
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
    // Intrusive links are meaningful only while `in_ready` is true. They keep
    // every priority FIFO caller-owned and avoid allocator-dependent queues in
    // the scheduler's hot path.
    next_ready: Option<usize>,
    prev_ready: Option<usize>,
    in_ready: bool,
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

/// A fixed-priority dispatcher over caller-owned thread storage.
///
/// Each of the 256 priorities owns a FIFO expressed through bounded intrusive
/// slot links. `ready_bitmap` contains one bit per non-empty queue, so selecting
/// the highest runnable priority takes four machine-word probes and one
/// `leading_zeros` instruction instead of inspecting every admitted thread.
pub struct Scheduler<'t> {
    threads: &'t mut [Thread],
    ready_head: [Option<usize>; 256],
    ready_tail: [Option<usize>; 256],
    ready_bitmap: [u64; 4],
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
            ready_head: [None; 256],
            ready_tail: [None; 256],
            ready_bitmap: [0; 4],
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
        self.threads
            .iter()
            .filter(|t| t.state != RunState::Inactive)
            .count()
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

    /// Number of non-empty priority queues; useful for bounded-ready-set tests.
    #[must_use]
    pub fn ready_priorities(&self) -> usize {
        self.ready_bitmap
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.threads
            .iter()
            .position(|t| t.state != RunState::Inactive && t.id == id)
    }

    fn bitmap_word(prio: u8) -> usize {
        usize::from(prio) / 64
    }

    fn bitmap_mask(prio: u8) -> u64 {
        1u64 << (u32::from(prio) % 64)
    }

    fn mark_priority_ready(&mut self, prio: u8) {
        self.ready_bitmap[Self::bitmap_word(prio)] |= Self::bitmap_mask(prio);
    }

    fn clear_priority_if_empty(&mut self, prio: u8) {
        if self.ready_head[usize::from(prio)].is_none() {
            self.ready_bitmap[Self::bitmap_word(prio)] &= !Self::bitmap_mask(prio);
        }
    }

    fn enqueue_ready(&mut self, idx: usize) {
        debug_assert!(self.threads[idx].state == RunState::Ready);
        debug_assert!(!self.threads[idx].in_ready);
        let prio = self.threads[idx].prio;
        let p = usize::from(prio);
        let tail = self.ready_tail[p];
        self.threads[idx].prev_ready = tail;
        self.threads[idx].next_ready = None;
        self.threads[idx].in_ready = true;
        if let Some(tail) = tail {
            self.threads[tail].next_ready = Some(idx);
        } else {
            self.ready_head[p] = Some(idx);
        }
        self.ready_tail[p] = Some(idx);
        self.mark_priority_ready(prio);
    }

    fn remove_ready(&mut self, idx: usize) {
        debug_assert!(self.threads[idx].in_ready);
        let prio = self.threads[idx].prio;
        let p = usize::from(prio);
        let prev = self.threads[idx].prev_ready;
        let next = self.threads[idx].next_ready;
        if let Some(prev) = prev {
            self.threads[prev].next_ready = next;
        } else {
            self.ready_head[p] = next;
        }
        if let Some(next) = next {
            self.threads[next].prev_ready = prev;
        } else {
            self.ready_tail[p] = prev;
        }
        self.threads[idx].next_ready = None;
        self.threads[idx].prev_ready = None;
        self.threads[idx].in_ready = false;
        self.clear_priority_if_empty(prio);
    }

    /// Return the head of the highest non-empty ready priority FIFO.
    fn pick(&self) -> Option<usize> {
        for word_index in (0..self.ready_bitmap.len()).rev() {
            let word = self.ready_bitmap[word_index];
            if word != 0 {
                let bit = 63 - word.leading_zeros() as usize;
                let prio = word_index * 64 + bit;
                return self.ready_head[prio];
            }
        }
        None
    }

    fn make_budget_eligible(&mut self, idx: usize) {
        if self.threads[idx].state == RunState::Ready
            && self.threads[idx].sc.remaining > 0
            && !self.threads[idx].in_ready
        {
            self.enqueue_ready(idx);
        }
    }

    /// Admit a thread with the given id, priority, and scheduling context.
    /// It starts `Ready`; contexts with zero initial budget are retained but not
    /// queued until their first period boundary replenishes them.
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
            next_ready: None,
            prev_ready: None,
            in_ready: false,
        };
        self.make_budget_eligible(slot);
        Ok(slot)
    }

    /// Mark the thread `id` as blocked (not dispatchable).
    ///
    /// # Errors
    /// Returns [`SchedError::NotFound`] if no such thread is admitted.
    pub fn block(&mut self, id: u64) -> Result<(), SchedError> {
        let idx = self.index_of(id).ok_or(SchedError::NotFound)?;
        if self.threads[idx].in_ready {
            self.remove_ready(idx);
        }
        self.threads[idx].state = RunState::Blocked;
        if self.current == Some(idx) {
            self.current = None;
        }
        Ok(())
    }

    /// Mark the thread `id` as ready (dispatchable).
    ///
    /// # Errors
    /// Returns [`SchedError::NotFound`] if no such thread is admitted.
    pub fn unblock(&mut self, id: u64) -> Result<(), SchedError> {
        let idx = self.index_of(id).ok_or(SchedError::NotFound)?;
        self.threads[idx].state = RunState::Ready;
        self.make_budget_eligible(idx);
        Ok(())
    }

    /// Recompute and return the thread that should run now.
    pub fn schedule(&mut self) -> Option<u64> {
        self.current = self.pick();
        self.current()
    }

    /// Advance time by one tick: charge the running thread a unit of budget,
    /// rotate it behind equal-priority peers when it remains eligible, replenish
    /// contexts at their period boundary, then redispatch. Returns the thread
    /// that should run after the tick.
    pub fn tick(&mut self) -> Option<u64> {
        self.ticks += 1;

        if let Some(i) = self.current {
            let was_ready = self.threads[i].in_ready;
            if was_ready {
                self.remove_ready(i);
            }
            self.threads[i].sc.remaining = self.threads[i].sc.remaining.saturating_sub(1);
            self.make_budget_eligible(i);
        }

        let now = self.ticks;
        for idx in 0..self.threads.len() {
            if self.threads[idx].state != RunState::Inactive
                && self.threads[idx].sc.period != 0
                && now % self.threads[idx].sc.period == 0
            {
                self.threads[idx].sc.remaining = self.threads[idx].sc.budget;
                self.make_budget_eligible(idx);
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

/// Failure modes for the cooperative context-switch self-test.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtxError {
    /// The worker context never executed after the switch into it.
    DidNotRun,
    /// The worker handed control back the wrong number of times.
    BadSwitchCount,
}

/// Boot-time self-test exercising a *real* cooperative context switch.
///
/// It builds a second context on its own stack, switches into it, lets it run
/// and hand control straight back, then verifies the round-trip happened
/// exactly once. This proves the callee-saved register save/restore and the
/// stack handoff in [`hull::context`] -- the substrate the timer-tick
/// preemptive scheduler will sit on. Interrupts are masked across the switch so
/// the timer ISR can never observe a half-switched stack; the round-trip is
/// fully deterministic.
///
/// # Errors
/// Returns [`CtxError`] if the worker context did not run, or if it handed
/// control back anything other than exactly once.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn ctx_selftest() -> Result<(), CtxError> {
    ctx::selftest()
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod ctx {
    use super::CtxError;
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use hull::context::{self, Context};

    /// Worker stack size. Generous for debug builds; lives in `.bss`.
    const WORKER_STACK_SIZE: usize = 16 * 1024;

    /// 16-byte aligned backing storage for the worker context's stack.
    #[repr(C, align(16))]
    struct Stack([u8; WORKER_STACK_SIZE]);

    static mut MAIN_CTX: Context = Context::zeroed();
    static mut WORKER_CTX: Context = Context::zeroed();
    static mut WORKER_STACK: Stack = Stack([0; WORKER_STACK_SIZE]);
    static WORKER_RAN: AtomicBool = AtomicBool::new(false);
    static SWITCHES: AtomicU64 = AtomicU64::new(0);

    /// Entry point for the worker context. Records that it ran, then switches
    /// straight back to the main context; control never returns here.
    extern "C" fn worker_entry() -> ! {
        WORKER_RAN.store(true, Ordering::Relaxed);
        SWITCHES.fetch_add(1, Ordering::Relaxed);
        // SAFETY: both statics are live for the duration of the self-test on the
        // single boot CPU. We save our state into WORKER_CTX and resume MAIN_CTX
        // (the suspended `selftest` frame).
        unsafe { context::switch(&raw mut WORKER_CTX, &raw const MAIN_CTX) }
        // `switch` above never returns; park defensively if it somehow did.
        loop {
            core::hint::spin_loop();
        }
    }

    pub(super) fn selftest() -> Result<(), CtxError> {
        WORKER_RAN.store(false, Ordering::Relaxed);
        SWITCHES.store(0, Ordering::Relaxed);

        // SAFETY: the single boot CPU owns these statics. We compute the 16-byte
        // aligned top of the worker stack and seed a fresh context that begins
        // executing `worker_entry` on the first switch into it.
        unsafe {
            let base = (&raw mut WORKER_STACK.0) as *mut u8 as usize;
            let top = base + WORKER_STACK_SIZE;
            context::init_context(&raw mut WORKER_CTX, top, worker_entry);
        }

        // Fence the cooperative round-trip: the timer ISR must not run on a
        // half-switched stack. The interrupt-enable state is a CPU flag the
        // switch does not touch, so it stays masked across the handoff and is
        // restored when `guard` drops.
        let guard = hull::irq::lock();

        // main -> worker -> main. Saves the live frame into MAIN_CTX and resumes
        // here once the worker hands control back.
        // SAFETY: MAIN_CTX receives our live callee-saved state; WORKER_CTX was
        // just initialised to enter `worker_entry` on its own stack.
        unsafe { context::switch(&raw mut MAIN_CTX, &raw const WORKER_CTX) }

        drop(guard);

        if !WORKER_RAN.load(Ordering::Relaxed) {
            return Err(CtxError::DidNotRun);
        }
        if SWITCHES.load(Ordering::Relaxed) != 1 {
            return Err(CtxError::BadSwitchCount);
        }
        Ok(())
    }
}

/// Failure modes for the preemptive timer-driven scheduling self-test.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PreemptError {
    /// One of the worker contexts never ran, so no real preemption happened.
    WorkerIdle,
    /// The timer drove fewer context switches than the schedule requires.
    TooFewSwitches,
}

/// Outcome of a successful [`preempt_selftest`]: how many preemptive switches
/// the timer drove, and how many loop iterations each worker accumulated while
/// it held the CPU.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[derive(Clone, Copy, Debug)]
pub struct PreemptStats {
    /// Number of timer-driven context switches performed.
    pub switches: u64,
    /// Loop iterations accumulated by worker A while it held the CPU.
    pub worker_a: u64,
    /// Loop iterations accumulated by worker B while it held the CPU.
    pub worker_b: u64,
}

/// Boot-time self-test proving *preemptive*, timer-driven context switching.
///
/// [`ctx_selftest`] proved a cooperative round-trip; this proves the real
/// thing. Two worker contexts that never voluntarily yield are interleaved
/// purely by the platform timer interrupt. The boot context registers a tick
/// hook (via [`hull::sched_hook`]) that round-robins boot -> A -> B -> A -> B
/// -> boot across consecutive ticks, then parks spinning until the schedule
/// completes. Each worker only ever spins incrementing a counter, so the only
/// thing that can move the CPU between them is the timer ISR.
///
/// # Errors
/// Returns [`PreemptError`] if either worker never ran, or if the timer drove
/// fewer switches than the schedule needs.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn preempt_selftest() -> Result<PreemptStats, PreemptError> {
    preempt::selftest()
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod preempt {
    use super::{PreemptError, PreemptStats};
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use hull::context::{self, Context};

    /// Worker stack size; lives in `.bss`, generous for debug builds.
    const WORKER_STACK_SIZE: usize = 16 * 1024;

    /// Number of tick-driven switches the schedule performs. The final switch
    /// returns to the boot context, so the demo needs this many live ticks.
    const SCHEDULE_LEN: u64 = 5;

    /// 16-byte aligned backing storage for a worker context's stack.
    #[repr(C, align(16))]
    struct Stack([u8; WORKER_STACK_SIZE]);

    static mut BOOT_CTX: Context = Context::zeroed();
    static mut A_CTX: Context = Context::zeroed();
    static mut B_CTX: Context = Context::zeroed();
    static mut A_STACK: Stack = Stack([0; WORKER_STACK_SIZE]);
    static mut B_STACK: Stack = Stack([0; WORKER_STACK_SIZE]);

    /// Set once the schedule has handed control back to the boot context.
    static DONE: AtomicBool = AtomicBool::new(false);
    /// Count of tick-hook invocations / context switches performed.
    static STEP: AtomicU64 = AtomicU64::new(0);
    /// Loop iterations each worker accumulates while running.
    static A_COUNT: AtomicU64 = AtomicU64::new(0);
    static B_COUNT: AtomicU64 = AtomicU64::new(0);

    /// Re-arm the platform timer so the demo keeps receiving ticks even after
    /// the heartbeat demo in Hull would have masked it.
    #[inline]
    fn rearm_timer() {
        // SAFETY: the timer was brought up during `activate_address_space`.
        unsafe {
            #[cfg(target_arch = "x86_64")]
            hull::apic::rearm_timer();
            #[cfg(target_arch = "aarch64")]
            hull::gic::rearm_timer();
        }
    }

    /// Disable the platform timer once the demo has finished.
    #[inline]
    fn stop_timer() {
        // SAFETY: the timer was brought up during `activate_address_space`.
        unsafe {
            #[cfg(target_arch = "x86_64")]
            hull::apic::mask_timer();
            #[cfg(target_arch = "aarch64")]
            hull::gic::disable_timer();
        }
    }

    extern "C" fn worker_a() -> ! {
        // Entered fresh from inside the timer ISR via a cooperative switch, so
        // interrupts are still masked on this stack (the ISR epilogue that
        // would have restored them never ran for us). Unmask them or this
        // worker could never be preempted off the CPU.
        // SAFETY: a valid stack and a live timer/IDT exist at this point.
        unsafe { hull::irq::force_enable() };
        loop {
            A_COUNT.fetch_add(1, Ordering::Relaxed);
            core::hint::spin_loop();
        }
    }

    extern "C" fn worker_b() -> ! {
        // SAFETY: see `worker_a`.
        unsafe { hull::irq::force_enable() };
        loop {
            B_COUNT.fetch_add(1, Ordering::Relaxed);
            core::hint::spin_loop();
        }
    }

    /// Tick hook: round-robin the boot context and the two workers, one switch
    /// per timer tick. Runs in ISR context, after EOI, on the stack of
    /// whatever thread is currently running.
    extern "C" fn on_tick() {
        if DONE.load(Ordering::SeqCst) {
            return;
        }
        // Keep the timer alive across the heartbeat demo's mask point.
        rearm_timer();

        let k = STEP.fetch_add(1, Ordering::SeqCst);
        // SAFETY: the boot CPU owns these statics for the duration of the demo.
        // Each (prev, next) pair matches the deterministic schedule below, and
        // every `next` context is either freshly initialised (`worker_*`) or
        // previously suspended at its own `switch` call site.
        unsafe {
            let (prev, next): (*mut Context, *const Context) = match k {
                0 => (&raw mut BOOT_CTX, &raw const A_CTX),
                1 => (&raw mut A_CTX, &raw const B_CTX),
                2 => (&raw mut B_CTX, &raw const A_CTX),
                3 => (&raw mut A_CTX, &raw const B_CTX),
                _ => {
                    DONE.store(true, Ordering::SeqCst);
                    (&raw mut B_CTX, &raw const BOOT_CTX)
                }
            };
            context::switch(prev, next);
        }
    }

    pub(super) fn selftest() -> Result<PreemptStats, PreemptError> {
        DONE.store(false, Ordering::SeqCst);
        STEP.store(0, Ordering::SeqCst);
        A_COUNT.store(0, Ordering::SeqCst);
        B_COUNT.store(0, Ordering::SeqCst);

        // SAFETY: the single boot CPU owns these statics. Seed each worker with
        // a 16-byte aligned stack top and its entry point; BOOT_CTX is filled
        // in by the first `switch` away from the boot context.
        unsafe {
            let a_top = (&raw mut A_STACK.0) as *mut u8 as usize + WORKER_STACK_SIZE;
            let b_top = (&raw mut B_STACK.0) as *mut u8 as usize + WORKER_STACK_SIZE;
            context::init_context(&raw mut A_CTX, a_top, worker_a);
            context::init_context(&raw mut B_CTX, b_top, worker_b);
        }

        // Install the hook and re-arm the timer, then spin. The boot context is
        // preempted on the next tick and only resumes here once the schedule
        // routes control back to it (with DONE set).
        hull::sched_hook::set_tick_hook(on_tick);
        rearm_timer();
        while !DONE.load(Ordering::SeqCst) {
            core::hint::spin_loop();
        }

        // Back on the boot context: tear down so later boot stages and idle see
        // a quiet, disabled timer and an inert hook. A tick racing this window
        // hits the `DONE` early-return in `on_tick` (no switch, no re-arm).
        hull::sched_hook::clear_tick_hook();
        stop_timer();

        let switches = STEP.load(Ordering::SeqCst);
        let worker_a = A_COUNT.load(Ordering::SeqCst);
        let worker_b = B_COUNT.load(Ordering::SeqCst);

        if worker_a == 0 || worker_b == 0 {
            return Err(PreemptError::WorkerIdle);
        }
        if switches < SCHEDULE_LEN {
            return Err(PreemptError::TooFewSwitches);
        }
        Ok(PreemptStats {
            switches,
            worker_a,
            worker_b,
        })
    }
}

#[cfg(test)]
mod replenishment_tests {
    extern crate std;

    use super::*;

    #[test]
    fn consumed_budget_returns_only_at_its_period_boundary() {
        let mut storage = [Refill::default(); 4];
        let mut queue = ReplenishmentQueue::new(&mut storage, 4, 10, 0).unwrap();

        assert_eq!(queue.available(0), 4);
        queue.consume(0, 1).unwrap();
        assert_eq!(queue.available(0), 3);
        queue.consume(0, 3).unwrap();
        assert_eq!(queue.available(0), 0);
        assert_eq!(queue.available(9), 0);
        assert_eq!(queue.available(10), 4);
        queue.consume(10, 4).unwrap();
        assert_eq!(queue.available(19), 0);
        assert_eq!(queue.available(20), 4);
    }

    #[test]
    fn bounded_saturation_delays_never_accelerates_budget() {
        let mut storage = [Refill::default(); 1];
        let mut queue = ReplenishmentQueue::new(&mut storage, 4, 10, 0).unwrap();

        queue.consume(0, 1).unwrap();
        // With only one entry, the three unused ticks are deliberately delayed
        // alongside the consumed tick's refill. This loses no safety boundary:
        // budget becomes later, never earlier.
        assert_eq!(queue.available(0), 0);
        assert_eq!(queue.available(9), 0);
        assert_eq!(queue.available(10), 4);
    }

    #[test]
    fn rejected_charge_leaves_queue_unchanged() {
        let mut storage = [Refill::default(); 2];
        let mut queue = ReplenishmentQueue::new(&mut storage, 2, 4, u64::MAX - 2).unwrap();
        let before = queue.head_refill();

        assert_eq!(queue.consume(u64::MAX - 2, 0), Err(RefillError::ZeroAmount));
        assert_eq!(
            queue.consume(u64::MAX - 2, 3),
            Err(RefillError::InsufficientBudget)
        );
        assert_eq!(
            queue.consume(u64::MAX - 2, 1),
            Err(RefillError::TimeOverflow)
        );
        assert_eq!(queue.head_refill(), before);
        assert_eq!(queue.available(u64::MAX - 2), 2);
    }

    #[test]
    fn construction_rejects_invalid_parameters() {
        let mut no_storage: [Refill; 0] = [];
        assert_eq!(
            ReplenishmentQueue::new(&mut no_storage, 1, 1, 0).err(),
            Some(RefillError::NoStorage)
        );
        let mut storage = [Refill::default(); 1];
        assert_eq!(
            ReplenishmentQueue::new(&mut storage, 0, 1, 0).err(),
            Some(RefillError::InvalidParameters)
        );
        assert_eq!(
            ReplenishmentQueue::new(&mut storage, 3, 2, 0).err(),
            Some(RefillError::InvalidParameters)
        );
    }
}

#[cfg(test)]
mod scheduler_tests {
    extern crate std;

    use super::*;

    #[test]
    fn bitmap_selects_highest_priority_and_rotates_fifo_peers() {
        let mut slots = [Thread::default(); 4];
        let mut scheduler = Scheduler::new(&mut slots);
        scheduler.admit(1, 2, SchedContext::new(8, 32)).unwrap();
        scheduler.admit(2, 200, SchedContext::new(8, 32)).unwrap();
        scheduler.admit(3, 200, SchedContext::new(8, 32)).unwrap();

        // Two set bits represent the low queue and the shared high-priority FIFO.
        assert_eq!(scheduler.ready_priorities(), 2);
        assert_eq!(scheduler.schedule(), Some(2));
        // A charged task returns to the FIFO tail, so its equal-priority peer is
        // next without weakening strict priority over task 1.
        assert_eq!(scheduler.tick(), Some(3));

        scheduler.block(3).unwrap();
        assert_eq!(scheduler.schedule(), Some(2));
        scheduler.block(2).unwrap();
        assert_eq!(scheduler.ready_priorities(), 1);
        assert_eq!(scheduler.schedule(), Some(1));

        // Wake restores the high-priority bit and dispatches that queue again.
        scheduler.unblock(2).unwrap();
        assert_eq!(scheduler.ready_priorities(), 2);
        assert_eq!(scheduler.schedule(), Some(2));
    }

    #[test]
    fn scheduler_selftest_preserves_budget_and_blocking_contract() {
        selftest().unwrap();
    }
}
