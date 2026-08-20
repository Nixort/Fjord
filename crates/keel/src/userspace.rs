// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// The code was written for Fjord.

//! Two-task unprivileged IPC round-trip.
//!
//! This is the Phase 2 integration proof. It creates two independent task
//! address spaces by cloning the active kernel root, maps a distinct unprivileged
//! code/stack pair into each VSpace, then lets Tide choose each task in turn.
//! The receiver first enters EL0/ring 3 and blocks on an [`crate::ipc::Endpoint`].
//! The sender enters separately, traps with a send request, and wakes the receiver.
//! The kernel installs the delivered word as the receiver's syscall result; its
//! saved [`UserFrame`] resumes at the instruction after its receive syscall and
//! exits with that value. Consequently the final exit trap proves all of:
//!
//! * two distinct CR3/TTBR0 roots were activated and restored;
//! * two user programs crossed the privilege boundary;
//! * Tide dispatched the receiver, sender, then woken receiver according to
//!   priority and bound scheduling contexts;
//! * task-aware endpoint IPC moved the receiver `Running -> Blocked -> Ready`;
//! * the receiver observed the sender's payload after a real frame resume.

use crate::{
    cap::{CapType, Capability, Rights},
    ipc::{Endpoint, IpcError, IpcResult, Message, Waiter},
    task::{TaskControlBlock, TaskError, TaskState, TaskTable, TaskTableError},
    tide::{SchedContext, SchedError, Scheduler, Thread},
    vspace::{self, HwVSpace, Mapping, VSpaceError},
};
use hull::mmu::FrameAllocator;
use hull::user::{self, UserFrame};

const PAGE_SIZE: u64 = 4096;
const TASK_CSPACE_RADIX: u64 = 5;

/// Kernel-stable IDs used by the bounded two-task integration run.
const RECEIVER_ID: u64 = 0x5048_32_52;
const SENDER_ID: u64 = 0x5048_32_53;

/// Static Tide priorities: the receiver runs first and blocks, then is selected
/// again as soon as endpoint IPC wakes it.
const RECEIVER_PRIORITY: u8 = 10;
const SENDER_PRIORITY: u8 = 5;
const RECEIVER_CONTEXT: SchedContext = SchedContext::new(2, 8);
const SENDER_CONTEXT: SchedContext = SchedContext::new(2, 8);

/// Per-task user mappings. Every address is in a user-only subtree that the
/// identity-mapped kernel sections do not use.
const RECEIVER_CODE_VA: u64 = 0x80_0000_0000;
const RECEIVER_STACK_VA: u64 = 0x80_0000_2000;
const SENDER_CODE_VA: u64 = 0x80_0001_0000;
const SENDER_STACK_VA: u64 = 0x80_0001_2000;
const USER_STACK_TOP_OFFSET: u64 = PAGE_SIZE;

/// Syscall ABI shared by the two tiny user programs.
const SYS_EXIT: u64 = 0;
const SYS_SEND: u64 = 1;
const SYS_RECV: u64 = 2;
const IPC_LABEL_ROUNDTRIP: u64 = 0x5048_32;
const IPC_BADGE_SENDER: u64 = 0x53;
/// Payload that must travel sender -> endpoint -> receiver -> final user exit.
const IPC_MAGIC: u64 = 0xF70D_CA11;

/// Why the Phase 2 task/VSpace/IPC proof could not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundtripError {
    /// The frame allocator could not provide task code, stack, root or table frames.
    OutOfFrames,
    /// A task user mapping was refused.
    MapFailed,
    /// A task table lookup failed unexpectedly.
    TaskLookup,
    /// A task lifecycle or scheduling-context transition was invalid.
    TaskState,
    /// The VSpace root could not be activated or restored.
    VSpace,
    /// Tide did not select the expected task or scheduler admission failed.
    Scheduler,
    /// Endpoint IPC did not block, wake or transfer as required.
    Ipc,
    /// A user program trapped with an unexpected syscall number or argument.
    BadTrap,
    /// The task lifecycle did not reach its expected terminal state.
    BadTaskState,
}

impl From<VSpaceError> for RoundtripError {
    fn from(_: VSpaceError) -> Self {
        Self::VSpace
    }
}

impl From<TaskError> for RoundtripError {
    fn from(_: TaskError) -> Self {
        Self::TaskState
    }
}

impl From<TaskTableError> for RoundtripError {
    fn from(_: TaskTableError) -> Self {
        Self::TaskLookup
    }
}

impl From<SchedError> for RoundtripError {
    fn from(_: SchedError) -> Self {
        Self::Scheduler
    }
}

impl From<IpcError> for RoundtripError {
    fn from(_: IpcError) -> Self {
        Self::Ipc
    }
}

/// Run Fjord's Phase 2 two-task userspace IPC round-trip.
///
/// The bump allocator deliberately retains every frame allocated by this
/// boot-time proof. The test is safe to invoke once after kernel mappings,
/// traps, frame allocation, and the local timer are online.
///
/// # Errors
/// Returns a precise [`RoundtripError`] when allocation, VSpace construction,
/// task lifecycle, Tide dispatch, user entry, or endpoint IPC loses an invariant.
pub fn ipc_roundtrip(frames: &mut FrameAllocator) -> Result<u64, RoundtripError> {
    let receiver_code_pa = frames.alloc().ok_or(RoundtripError::OutOfFrames)?;
    let receiver_stack_pa = frames.alloc().ok_or(RoundtripError::OutOfFrames)?;
    let sender_code_pa = frames.alloc().ok_or(RoundtripError::OutOfFrames)?;
    let sender_stack_pa = frames.alloc().ok_or(RoundtripError::OutOfFrames)?;
    write_receiver_program(receiver_code_pa);
    write_sender_program(sender_code_pa);

    // Each cloned L0/PML4 retains all kernel mappings but gets a separate root.
    // The mapping ledgers live for the complete run even though the programs use
    // Hull directly for their first small user-page population.
    let mut receiver_maps = [Mapping::EMPTY; 2];
    let mut sender_maps = [Mapping::EMPTY; 2];
    let receiver_vspace = HwVSpace::clone_kernel_root(&mut receiver_maps, frames)
        .ok_or(RoundtripError::OutOfFrames)?;
    let sender_vspace =
        HwVSpace::clone_kernel_root(&mut sender_maps, frames).ok_or(RoundtripError::OutOfFrames)?;
    if receiver_vspace.root() == sender_vspace.root() {
        return Err(RoundtripError::VSpace);
    }
    map_task_pages(
        receiver_vspace.root(),
        RECEIVER_CODE_VA,
        receiver_code_pa,
        RECEIVER_STACK_VA,
        receiver_stack_pa,
        frames,
    )?;
    map_task_pages(
        sender_vspace.root(),
        SENDER_CODE_VA,
        sender_code_pa,
        SENDER_STACK_VA,
        sender_stack_pa,
        frames,
    )?;

    // The CSpace root is not dereferenced by this narrow user-entry proof, but
    // it remains a real CNode capability rooted at the current kernel address
    // space. Later capability lookup will replace this bootstrap authority with
    // per-task CNode storage without changing the TCB execution contract.
    let cspace_root = Capability::new(
        CapType::CNode,
        hull::paging::active_root(),
        TASK_CSPACE_RADIX,
        Rights::ALL,
    );
    let mut receiver = TaskControlBlock::new(
        RECEIVER_ID,
        cspace_root,
        receiver_vspace.root(),
        UserFrame::new(RECEIVER_CODE_VA, RECEIVER_STACK_VA + USER_STACK_TOP_OFFSET),
    )?;
    let mut sender = TaskControlBlock::new(
        SENDER_ID,
        cspace_root,
        sender_vspace.root(),
        UserFrame::new(SENDER_CODE_VA, SENDER_STACK_VA + USER_STACK_TOP_OFFSET),
    )?;
    receiver.bind_sched_context(RECEIVER_CONTEXT)?;
    sender.bind_sched_context(SENDER_CONTEXT)?;

    let mut tasks = [receiver, sender];
    let receiver_sc = tasks[0].sched_context().ok_or(RoundtripError::TaskState)?;
    let sender_sc = tasks[1].sched_context().ok_or(RoundtripError::TaskState)?;
    let mut table = TaskTable::new(&mut tasks)?;

    let mut thread_slots = [Thread::default(); 2];
    let mut tide = Scheduler::new(&mut thread_slots);
    tide.admit(RECEIVER_ID, RECEIVER_PRIORITY, receiver_sc)?;
    tide.admit(SENDER_ID, SENDER_PRIORITY, sender_sc)?;

    let mut waiter_slots = [Waiter::default(); 2];
    let mut endpoint = Endpoint::new(&mut waiter_slots);
    user::init();

    // Receiver is Tide's first choice. Its initial user entry asks for a
    // synchronous receive; endpoint IPC blocks the TCB and Tide marks its slot
    // unavailable before selecting the sender.
    if tide.schedule() != Some(RECEIVER_ID)
        || run_user_task(&mut table, RECEIVER_ID)? != (SYS_RECV, 0)
    {
        return Err(RoundtripError::Scheduler);
    }
    if endpoint.recv_task(&mut table, RECEIVER_ID)? != IpcResult::Queued {
        return Err(RoundtripError::Ipc);
    }
    tide.block(RECEIVER_ID)?;
    if table.get(RECEIVER_ID)?.state() != TaskState::Blocked || endpoint.waiting() != 1 {
        return Err(RoundtripError::BadTaskState);
    }

    // Sender now becomes the highest eligible Tide task. Its user request is
    // transferred atomically, which wakes the receiver to Ready in TaskTable.
    if tide.schedule() != Some(SENDER_ID)
        || run_user_task(&mut table, SENDER_ID)? != (SYS_SEND, IPC_MAGIC)
    {
        return Err(RoundtripError::Scheduler);
    }
    let message = Message::new(IPC_BADGE_SENDER, IPC_LABEL_ROUNDTRIP, &[IPC_MAGIC]);
    match endpoint.send_task(&mut table, SENDER_ID, message)? {
        IpcResult::Delivered { peer, msg }
            if peer == RECEIVER_ID
                && msg.badge() == IPC_BADGE_SENDER
                && msg.label() == IPC_LABEL_ROUNDTRIP
                && msg.word(0) == Some(IPC_MAGIC) => {}
        _ => return Err(RoundtripError::Ipc),
    }
    tide.unblock(RECEIVER_ID)?;
    table.get_mut(SENDER_ID)?.exit()?;
    tide.block(SENDER_ID)?;
    if table.get(RECEIVER_ID)?.state() != TaskState::Ready || endpoint.waiting() != 0 {
        return Err(RoundtripError::BadTaskState);
    }

    // Resume the exact receiver frame captured at its first syscall. The code
    // copies its kernel-provided return register into EXIT(arg0), so this trap
    // proves the transferred payload reached unprivileged execution.
    table
        .get_mut(RECEIVER_ID)?
        .user_frame_mut()
        .set_ret(IPC_MAGIC);
    if tide.schedule() != Some(RECEIVER_ID)
        || run_user_task(&mut table, RECEIVER_ID)? != (SYS_EXIT, IPC_MAGIC)
    {
        return Err(RoundtripError::Scheduler);
    }
    table.get_mut(RECEIVER_ID)?.exit()?;
    tide.block(RECEIVER_ID)?;

    if tide.schedule().is_some()
        || table.get(RECEIVER_ID)?.state() != TaskState::Exited
        || table.get(SENDER_ID)?.state() != TaskState::Exited
    {
        return Err(RoundtripError::BadTaskState);
    }
    Ok(IPC_MAGIC)
}

/// Map one user R-X code page and one user RW/NX stack page into an inactive
/// cloned task root. The first activation flushes the full local TLB, so no
/// page-local flush can accidentally target the still-active kernel root.
fn map_task_pages(
    root: u64,
    code_va: u64,
    code_pa: u64,
    stack_va: u64,
    stack_pa: u64,
    frames: &mut FrameAllocator,
) -> Result<(), RoundtripError> {
    // SAFETY: `root` comes directly from `clone_kernel_root`; its page-table
    // frames are fresh and reachable through the active kernel identity mapping.
    let mut mapper = unsafe { hull::paging::Mapper::from_root(root) };
    if !hull::paging::map_user_page(&mut mapper, code_va, code_pa, false, true, frames)
        || !hull::paging::map_user_page(&mut mapper, stack_va, stack_pa, true, false, frames)
    {
        return Err(RoundtripError::MapFailed);
    }
    Ok(())
}

/// Activate `id`'s task VSpace, enter or resume its saved user frame through
/// one syscall trap, restore the kernel root, and leave its TCB Running so the
/// syscall dispatcher can choose the next lifecycle transition.
fn run_user_task(tasks: &mut TaskTable<'_>, id: u64) -> Result<(u64, u64), RoundtripError> {
    let (frame, guard) = {
        let task = tasks.get_mut(id)?;
        task.start()?;
        // SAFETY: this integration path mapped the task's R-X code and RW/NX
        // stack into a cloned root while preserving the live kernel mappings.
        let guard = unsafe { vspace::handoff_to_task(task) }?;
        (task.user_frame_mut() as *mut UserFrame, guard)
    };
    // SAFETY: `user::init` ran before dispatch, and `frame` resides in a TCB
    // retained by `tasks` for this call. Its VSpace is active until the trap.
    unsafe { user::run(frame) };
    // SAFETY: the guard captured the live kernel root immediately before entry.
    unsafe { guard.restore() }?;

    let task = tasks.get(id)?;
    let frame = task.user_frame();
    Ok((frame.syscall_nr(), frame.arg0()))
}

// ---------------------------------------------------------------------------
// Tiny userspace programs. Their machine-code bytes are architecture-specific
// executable data, not inline Rust asm templates. The x86_64 path uses int 0x80
// while the aarch64 path uses svc #0, matching hull::user's portable ABI.
// ---------------------------------------------------------------------------

/// Sender: `SYS_SEND(IPC_MAGIC)`, then park until the kernel retires its TCB.
#[cfg(target_arch = "x86_64")]
fn write_sender_program(code_pa: u64) {
    let blob: [u8; 14] = [
        0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, SYS_SEND
        0xBF, 0x11, 0xCA, 0x0D, 0xF7, // mov edi, IPC_MAGIC
        0xCD, 0x80, // int 0x80
        0xEB, 0xFE, // jmp $
    ];
    // SAFETY: fresh identity-mapped RAM frame, not referenced by another task.
    unsafe { core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u8, blob.len()) };
}

/// Receiver: `SYS_RECV()`, then `SYS_EXIT(return_value)` after frame resume.
#[cfg(target_arch = "x86_64")]
fn write_receiver_program(code_pa: u64) {
    let blob: [u8; 18] = [
        0xB8, 0x02, 0x00, 0x00, 0x00, // mov eax, SYS_RECV
        0x31, 0xFF, // xor edi, edi
        0xCD, 0x80, // int 0x80
        0x48, 0x89, 0xC7, // mov rdi, rax
        0x31, 0xC0, // xor eax, eax (SYS_EXIT)
        0xCD, 0x80, // int 0x80
        0xEB, 0xFE, // jmp $
    ];
    // SAFETY: see `write_sender_program`.
    unsafe { core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u8, blob.len()) };
}

/// Sender for EL0: `SYS_SEND(IPC_MAGIC)`, then park.
#[cfg(target_arch = "aarch64")]
fn write_sender_program(code_pa: u64) {
    let blob: [u32; 5] = [
        0xD280_0020, // mov x0, #SYS_SEND
        0x5299_4221, // movz w1, #0xCA11
        0x72BE_E1A1, // movk w1, #0xF70D, lsl #16
        0xD400_0001, // svc #0
        0x1400_0000, // b .
    ];
    // SAFETY: fresh identity-mapped RAM frame; publish data writes to EL0 I-side.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u32, blob.len());
        hull::arch::aarch64::sync_instruction_cache(code_pa, core::mem::size_of_val(&blob));
    }
}

/// Receiver for EL0: `SYS_RECV()`, then `SYS_EXIT(return_value)` after resume.
#[cfg(target_arch = "aarch64")]
fn write_receiver_program(code_pa: u64) {
    let blob: [u32; 7] = [
        0xD280_0040, // mov x0, #SYS_RECV
        0xD280_0001, // mov x1, #0
        0xD400_0001, // svc #0
        0xAA00_03E1, // mov x1, x0
        0xD280_0000, // mov x0, #SYS_EXIT
        0xD400_0001, // svc #0
        0x1400_0000, // b .
    ];
    // SAFETY: see `write_sender_program`.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u32, blob.len());
        hull::arch::aarch64::sync_instruction_cache(code_pa, core::mem::size_of_val(&blob));
    }
}
