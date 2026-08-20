//! Task control blocks and the first live task lifecycle.
//!
//! A [`TaskControlBlock`] binds the state that was previously demonstrated by
//! independent Phase 2 models: a task identity, a root capability space, an
//! inactive or active hardware VSpace root, its resumable [`hull::user::UserFrame`],
//! and a small run-state machine. It is deliberately heap-free and contains no
//! scheduler policy; Tide owns admission and dispatch, while IPC will use the
//! blocked state in the next integration slice.
//!
//! The TCB does not yet execute a task. It establishes the stable kernel object
//! boundary that later patches bind to a [`crate::tide::SchedContext`] and use to
//! safely move a task between Ready, Running, Blocked, and Exited states.

use crate::cap::{CapType, Capability};
use crate::tide::SchedContext;
use hull::user::UserFrame;

/// The observable lifecycle state of a task.
///
/// A task starts [`Ready`](Self::Ready), may be dispatched as
/// [`Running`](Self::Running), and can be blocked by IPC or a fault. An exited
/// task is terminal: it cannot be resumed or made runnable again without
/// retyping a fresh TCB object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// Eligible for scheduler dispatch once it has a scheduling context.
    Ready,
    /// Currently executing in the kernel or unprivileged context.
    Running,
    /// Waiting for a kernel event such as an endpoint rendezvous.
    Blocked,
    /// The task trapped with a fault awaiting a supervising policy decision.
    Faulted,
    /// The task has exited and can no longer transition.
    Exited,
}

/// Why a task lifecycle or construction operation was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskError {
    /// The supplied root capability is not a CNode capability.
    InvalidCspaceRoot,
    /// The supplied hardware VSpace root is zero or not 4 KiB aligned.
    InvalidVspaceRoot,
    /// The requested run-state transition is not valid from the current state.
    InvalidTransition,
    /// The task has already reached its terminal exited state.
    Exited,
    /// The scheduling context has no valid positive budget/period configuration.
    InvalidSchedContext,
    /// The task has no scheduling context to unbind or mutate.
    MissingSchedContext,
}

/// A heap-free kernel task object.
///
/// `cspace_root` names the task's root CNode and `vspace_root` is the physical
/// root table that a later handoff installs in CR3 or TTBR0. The user frame is a
/// complete architecture-specific resume point. These three resources travel
/// together so a future scheduler never combines one task's execution state
/// with another task's authority or address space.
#[derive(Clone, Copy, Debug)]
pub struct TaskControlBlock {
    id: u64,
    cspace_root: Capability,
    vspace_root: u64,
    user_frame: UserFrame,
    sched_context: Option<SchedContext>,
    state: TaskState,
}

impl TaskControlBlock {
    /// Constructs a fresh, ready task.
    ///
    /// The root capability must name a CNode. `vspace_root` must be a non-zero,
    /// 4 KiB-aligned page-table root; this is the common alignment requirement
    /// for the x86_64 CR3 and aarch64 TTBR0 roots used by Hull.
    pub fn new(
        id: u64,
        cspace_root: Capability,
        vspace_root: u64,
        user_frame: UserFrame,
    ) -> Result<Self, TaskError> {
        if cspace_root.cap_type() != CapType::CNode {
            return Err(TaskError::InvalidCspaceRoot);
        }
        if vspace_root == 0 || vspace_root & 0xfff != 0 {
            return Err(TaskError::InvalidVspaceRoot);
        }
        Ok(Self {
            id,
            cspace_root,
            vspace_root,
            user_frame,
            sched_context: None,
            state: TaskState::Ready,
        })
    }

    /// Kernel-stable task identifier.
    #[must_use]
    pub const fn id(self) -> u64 {
        self.id
    }

    /// Capability naming this task's CSpace root CNode.
    #[must_use]
    pub const fn cspace_root(self) -> Capability {
        self.cspace_root
    }

    /// Physical root address for this task's hardware VSpace.
    #[must_use]
    pub const fn vspace_root(self) -> u64 {
        self.vspace_root
    }

    /// Current task lifecycle state.
    #[must_use]
    pub const fn state(self) -> TaskState {
        self.state
    }

    /// Immutable access to the resumable unprivileged register frame.
    #[must_use]
    pub const fn user_frame(&self) -> &UserFrame {
        &self.user_frame
    }

    /// Mutable access to the saved unprivileged register frame.
    ///
    /// The caller must only mutate it while this TCB is exclusively owned by the
    /// kernel. A later scheduler integration enforces that ownership boundary.
    pub fn user_frame_mut(&mut self) -> &mut UserFrame {
        &mut self.user_frame
    }

    /// Bind a configured scheduling context to this task.
    ///
    /// The context must carry a positive budget and period with `budget <= period`.
    /// Binding replaces a previously bound context only before the task exits;
    /// the scheduler will later own the context's mutable budget accounting.
    pub fn bind_sched_context(&mut self, sc: SchedContext) -> Result<(), TaskError> {
        if self.state == TaskState::Exited {
            return Err(TaskError::Exited);
        }
        if sc.budget() == 0 || sc.period() == 0 || sc.budget() > sc.period() {
            return Err(TaskError::InvalidSchedContext);
        }
        self.sched_context = Some(sc);
        Ok(())
    }

    /// Remove and return the task's scheduling context.
    pub fn unbind_sched_context(&mut self) -> Result<SchedContext, TaskError> {
        if self.state == TaskState::Exited {
            return Err(TaskError::Exited);
        }
        self.sched_context
            .take()
            .ok_or(TaskError::MissingSchedContext)
    }

    /// The scheduling context currently bound to this task, if any.
    #[must_use]
    pub const fn sched_context(&self) -> Option<SchedContext> {
        self.sched_context
    }

    /// Mutable access to the bound scheduling context for Tide accounting.
    pub fn sched_context_mut(&mut self) -> Result<&mut SchedContext, TaskError> {
        if self.state == TaskState::Exited {
            return Err(TaskError::Exited);
        }
        self.sched_context
            .as_mut()
            .ok_or(TaskError::MissingSchedContext)
    }

    /// Whether this task is eligible for Tide dispatch.
    ///
    /// A task must be ready, have a bound configured scheduling context and still
    /// carry budget. This keeps task eligibility local and prevents later Tide
    /// queues from dispatching authority-less or budget-less tasks.
    #[must_use]
    pub fn is_schedulable(&self) -> bool {
        self.state == TaskState::Ready
            && self
                .sched_context
                .is_some_and(|sc| sc.budget() > 0 && sc.period() > 0 && !sc.depleted())
    }

    /// Mark a ready task as currently running.
    pub fn start(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Ready, TaskState::Running)
    }

    /// Return a running task to the scheduler's ready set.
    pub fn yield_now(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Running, TaskState::Ready)
    }

    /// Block a running task on a kernel-owned event.
    pub fn block(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Running, TaskState::Blocked)
    }

    /// Wake a task after the event on which it blocked becomes ready.
    pub fn unblock(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Blocked, TaskState::Ready)
    }

    /// Record a supervisor-visible fault from a running task.
    pub fn fault(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Running, TaskState::Faulted)
    }

    /// Resume a task after its supervisor repaired or handled the fault.
    pub fn resume_fault(&mut self) -> Result<(), TaskError> {
        self.transition(TaskState::Faulted, TaskState::Ready)
    }

    /// End this task from any non-terminal state.
    pub fn exit(&mut self) -> Result<(), TaskError> {
        if self.state == TaskState::Exited {
            return Err(TaskError::Exited);
        }
        self.state = TaskState::Exited;
        Ok(())
    }

    fn transition(&mut self, from: TaskState, to: TaskState) -> Result<(), TaskError> {
        if self.state == TaskState::Exited {
            return Err(TaskError::Exited);
        }
        if self.state != from {
            return Err(TaskError::InvalidTransition);
        }
        self.state = to;
        Ok(())
    }
}

/// Boot-time task-object smoke test.
///
/// Exercises construction guard rails, all non-terminal lifecycle transitions,
/// frame mutation while exclusively owned, and the terminal-state invariant.
pub fn selftest() -> Result<(), TaskError> {
    use crate::cap::Rights;

    let cspace = Capability::new(CapType::CNode, 0x40_0000, 5, Rights::ALL);
    let mut task = TaskControlBlock::new(7, cspace, 0x80_0000, UserFrame::new(0x4000, 0x8000))?;
    if task.id() != 7
        || task.cspace_root() != cspace
        || task.vspace_root() != 0x80_0000
        || task.state() != TaskState::Ready
    {
        return Err(TaskError::InvalidTransition);
    }

    if task.is_schedulable() {
        return Err(TaskError::MissingSchedContext);
    }
    task.bind_sched_context(SchedContext::new(2, 4))?;
    if !task.is_schedulable() {
        return Err(TaskError::MissingSchedContext);
    }
    task.start()?;
    task.user_frame_mut().set_ret(0xfeed);
    if task.user_frame().syscall_nr() != 0xfeed {
        return Err(TaskError::InvalidTransition);
    }
    task.block()?;
    task.unblock()?;
    task.start()?;
    task.fault()?;
    task.resume_fault()?;
    task.start()?;
    task.yield_now()?;
    if !task.is_schedulable() {
        return Err(TaskError::MissingSchedContext);
    }
    if task.unbind_sched_context()? != SchedContext::new(2, 4) || task.is_schedulable() {
        return Err(TaskError::MissingSchedContext);
    }
    if task.bind_sched_context(SchedContext::new(0, 4)) != Err(TaskError::InvalidSchedContext) {
        return Err(TaskError::InvalidSchedContext);
    }
    task.exit()?;
    if task.start() != Err(TaskError::Exited) || task.exit() != Err(TaskError::Exited) {
        return Err(TaskError::Exited);
    }

    let non_cnode = Capability::new(CapType::Page, 0x40_0000, 12, Rights::READ);
    if !matches!(
        TaskControlBlock::new(8, non_cnode, 0x80_0000, UserFrame::new(0x4000, 0x8000)),
        Err(TaskError::InvalidCspaceRoot)
    ) {
        return Err(TaskError::InvalidCspaceRoot);
    }
    if !matches!(
        TaskControlBlock::new(9, cspace, 0x80_0001, UserFrame::new(0x4000, 0x8000)),
        Err(TaskError::InvalidVspaceRoot)
    ) {
        return Err(TaskError::InvalidVspaceRoot);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::cap::Rights;

    fn root() -> Capability {
        Capability::new(CapType::CNode, 0x40_0000, 5, Rights::ALL)
    }

    fn task() -> TaskControlBlock {
        TaskControlBlock::new(1, root(), 0x80_0000, UserFrame::new(0x4000, 0x8000)).unwrap()
    }

    #[test]
    fn lifecycle_is_strict_and_terminal() {
        let mut t = task();
        assert_eq!(t.block(), Err(TaskError::InvalidTransition));
        t.start().unwrap();
        t.block().unwrap();
        assert_eq!(t.fault(), Err(TaskError::InvalidTransition));
        t.unblock().unwrap();
        t.exit().unwrap();
        assert_eq!(t.unblock(), Err(TaskError::Exited));
    }

    #[test]
    fn scheduling_context_binding_controls_eligibility() {
        let mut t = task();
        assert!(!t.is_schedulable());
        assert_eq!(
            t.bind_sched_context(SchedContext::new(5, 4)),
            Err(TaskError::InvalidSchedContext)
        );
        assert_eq!(
            t.unbind_sched_context(),
            Err(TaskError::MissingSchedContext)
        );
        t.bind_sched_context(SchedContext::new(2, 4)).unwrap();
        assert!(t.is_schedulable());
        t.start().unwrap();
        assert!(!t.is_schedulable());
        t.yield_now().unwrap();
        assert_eq!(t.unbind_sched_context().unwrap(), SchedContext::new(2, 4));
        assert!(!t.is_schedulable());
    }

    #[test]
    fn construction_rejects_invalid_authority_roots() {
        let page = Capability::new(CapType::Page, 0x40_0000, 12, Rights::READ);
        assert!(matches!(
            TaskControlBlock::new(1, page, 0x80_0000, UserFrame::new(0x4000, 0x8000)),
            Err(TaskError::InvalidCspaceRoot)
        ));
        assert!(matches!(
            TaskControlBlock::new(1, root(), 0, UserFrame::new(0x4000, 0x8000)),
            Err(TaskError::InvalidVspaceRoot)
        ));
    }

    #[test]
    fn selftest_passes() {
        selftest().unwrap();
    }
}

/// Why a task-table lookup or construction operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTableError {
    /// No task with the requested kernel-stable ID exists in the table.
    NotFound,
    /// The caller attempted to construct a table with duplicate task IDs.
    DuplicateId,
}

/// Caller-owned task storage indexed by kernel-stable task ID.
///
/// The table is deliberately a simple bounded linear lookup for the first
/// integration path. It gives IPC one exclusive, checked place to find and
/// transition blocked peers; ready-queue indexing becomes a separate Tide
/// optimization after task-aware correctness is established.
pub struct TaskTable<'tasks> {
    tasks: &'tasks mut [TaskControlBlock],
}

impl<'tasks> TaskTable<'tasks> {
    /// Wraps a unique-ID task slice for kernel use.
    pub fn new(tasks: &'tasks mut [TaskControlBlock]) -> Result<Self, TaskTableError> {
        for (i, task) in tasks.iter().enumerate() {
            if tasks[..i].iter().any(|prior| prior.id == task.id) {
                return Err(TaskTableError::DuplicateId);
            }
        }
        Ok(Self { tasks })
    }

    /// Number of stored task control blocks.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether the table contains no task control blocks.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Read a task by its kernel-stable ID.
    pub fn get(&self, id: u64) -> Result<&TaskControlBlock, TaskTableError> {
        self.tasks
            .iter()
            .find(|task| task.id == id)
            .ok_or(TaskTableError::NotFound)
    }

    /// Mutably borrow a task by its kernel-stable ID.
    pub fn get_mut(&mut self, id: u64) -> Result<&mut TaskControlBlock, TaskTableError> {
        self.tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or(TaskTableError::NotFound)
    }
}

#[cfg(test)]
mod task_table_tests {
    extern crate std;

    use super::*;
    use crate::cap::Rights;

    fn task(id: u64) -> TaskControlBlock {
        let root = Capability::new(CapType::CNode, 0x40_0000 + id * 0x1000, 5, Rights::ALL);
        TaskControlBlock::new(
            id,
            root,
            0x80_0000 + id * 0x1000,
            UserFrame::new(0x4000, 0x8000),
        )
        .unwrap()
    }

    #[test]
    fn table_requires_unique_ids_and_checked_lookups() {
        let mut duplicate = [task(1), task(1)];
        assert_eq!(
            TaskTable::new(&mut duplicate).err(),
            Some(TaskTableError::DuplicateId)
        );

        let mut tasks = [task(1), task(2)];
        let mut table = TaskTable::new(&mut tasks).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(2).unwrap().id(), 2);
        assert!(matches!(table.get(3), Err(TaskTableError::NotFound)));
        table.get_mut(1).unwrap().exit().unwrap();
        assert_eq!(table.get(1).unwrap().state(), TaskState::Exited);
    }
}
