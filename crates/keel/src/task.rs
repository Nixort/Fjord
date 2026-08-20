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
