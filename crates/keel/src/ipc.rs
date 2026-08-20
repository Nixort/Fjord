// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// The code was written for Fjord.

//! Inter-process communication.
//!
//! Three mechanisms, all heap-free and over caller-owned storage:
//!
//! * [`Endpoint`] — synchronous rendezvous. A thread that sends with no peer
//!   waiting blocks in the endpoint queue; the arrival of a receiver (or vice
//!   versa) completes the transfer. An endpoint queue is homogeneous: either
//!   senders or receivers wait, never both at once.
//! * [`Notification`] — asynchronous signalling. `signal` ORs a badge into a
//!   pending word; `poll` reads and clears it. This is the seL4 notification
//!   word model.
//! * [`VmRing`] — a single-producer/single-consumer ring index over a shared
//!   frame, for bulk/streaming transfer outside the synchronous path.
//!
//! The endpoint's task-aware operations couple this queue to caller-owned
//! [`crate::task::TaskTable`] storage: a queued caller moves from Running to
//! Blocked, and a rendezvous wakes the previously blocked peer to Ready. Tide
//! remains responsible for choosing the next ready task.
//!
//! See `docs/ARCHITECTURE.md` §1.

use crate::task::{TaskError, TaskState, TaskTable, TaskTableError};

/// A fixed-size IPC message: a badge, a label, and a few data words.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Message {
    badge: u64,
    label: u64,
    words: [u64; Message::MAX_WORDS],
    len: usize,
}

impl Message {
    /// Maximum number of data words carried inline in a message.
    pub const MAX_WORDS: usize = 8;

    /// An empty message (no badge, no label, no words).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            badge: 0,
            label: 0,
            words: [0; Self::MAX_WORDS],
            len: 0,
        }
    }

    /// Build a message from a badge, a label, and up to [`Message::MAX_WORDS`]
    /// data words. Extra words are dropped.
    #[must_use]
    pub fn new(badge: u64, label: u64, words: &[u64]) -> Self {
        let mut buf = [0u64; Self::MAX_WORDS];
        for (slot, &w) in buf.iter_mut().zip(words) {
            *slot = w;
        }
        Self {
            badge,
            label,
            words: buf,
            len: words.len().min(Self::MAX_WORDS),
        }
    }

    /// The sender-stamped badge.
    #[must_use]
    pub const fn badge(self) -> u64 {
        self.badge
    }

    /// The message label (operation selector).
    #[must_use]
    pub const fn label(self) -> u64 {
        self.label
    }

    /// Number of valid data words.
    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    /// Whether the message carries no data words.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// The `i`th data word, if present.
    #[must_use]
    pub fn word(self, i: usize) -> Option<u64> {
        if i < self.len {
            Some(self.words[i])
        } else {
            None
        }
    }
}

/// An asynchronous notification: a word of badge bits.
#[derive(Clone, Copy, Debug, Default)]
pub struct Notification {
    word: u64,
}

impl Notification {
    /// A fresh notification with nothing pending.
    #[must_use]
    pub const fn new() -> Self {
        Self { word: 0 }
    }

    /// OR `badge` into the pending set. Non-blocking, never fails.
    pub fn signal(&mut self, badge: u64) {
        self.word |= badge;
    }

    /// Whether any badge bits are pending.
    #[must_use]
    pub const fn pending(self) -> bool {
        self.word != 0
    }

    /// Read and clear the pending badge set.
    pub fn poll(&mut self) -> u64 {
        let pending = self.word;
        self.word = 0;
        pending
    }
}

/// One blocked party in an endpoint queue. Callers allocate the endpoint's
/// wait-queue storage as `[Waiter::default(); N]`; the fields are private.
#[derive(Clone, Copy, Debug, Default)]
pub struct Waiter {
    thread: u64,
    msg: Message,
}

/// What an endpoint queue currently holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum EpState {
    /// Nothing waiting.
    #[default]
    Idle,
    /// One or more senders are blocked waiting for a receiver.
    Senders,
    /// One or more receivers are blocked waiting for a sender.
    Receivers,
}

/// Outcome of an endpoint operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IpcResult {
    /// The operation rendezvoused with a waiting peer; the message transferred.
    Delivered {
        /// The peer thread that was waiting.
        peer: u64,
        /// The transferred message.
        msg: Message,
    },
    /// No peer was waiting; the caller was enqueued and would block.
    Queued,
}

/// Why an endpoint operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IpcError {
    /// The endpoint's wait queue is full.
    QueueFull,
    /// The endpoint internal state did not match its queue storage.
    InconsistentQueue,
    /// A named task was absent from the caller-provided task table.
    TaskLookup(TaskTableError),
    /// A task lifecycle transition required by IPC was not legal.
    TaskState(TaskError),
}

/// A synchronous rendezvous endpoint over a caller-owned wait queue (a FIFO
/// ring). The queue is homogeneous: it holds either senders or receivers.
pub struct Endpoint<'q> {
    queue: &'q mut [Waiter],
    state: EpState,
    head: usize,
    len: usize,
}

impl<'q> Endpoint<'q> {
    /// Wrap a slice of queue storage, clearing it to an idle endpoint.
    #[must_use]
    pub fn new(queue: &'q mut [Waiter]) -> Self {
        for w in queue.iter_mut() {
            *w = Waiter::default();
        }
        Self {
            queue,
            state: EpState::Idle,
            head: 0,
            len: 0,
        }
    }

    /// Number of parties currently blocked on this endpoint.
    #[must_use]
    pub const fn waiting(&self) -> usize {
        self.len
    }

    /// Maximum number of parties that can block at once.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.queue.len()
    }

    fn ensure_enqueue_capacity(&self) -> Result<(), IpcError> {
        if self.queue.is_empty() || self.len >= self.queue.len() {
            return Err(IpcError::QueueFull);
        }
        Ok(())
    }

    fn enqueue(&mut self, waiter: Waiter) -> Result<(), IpcError> {
        self.ensure_enqueue_capacity()?;
        let slot = (self.head + self.len) % self.queue.len();
        self.queue[slot] = waiter;
        self.len += 1;
        Ok(())
    }

    fn front(&self) -> Option<Waiter> {
        (self.len != 0).then(|| self.queue[self.head])
    }

    fn dequeue(&mut self) -> Option<Waiter> {
        if self.len == 0 {
            return None;
        }
        let cap = self.queue.len();
        let waiter = self.queue[self.head];
        self.queue[self.head] = Waiter::default();
        self.head = (self.head + 1) % cap;
        self.len -= 1;
        if self.len == 0 {
            self.state = EpState::Idle;
        }
        Some(waiter)
    }

    /// Send `msg` from `thread`. Rendezvous with a waiting receiver if one is
    /// blocked, otherwise enqueue this sender.
    ///
    /// # Errors
    /// Returns [`IpcError::QueueFull`] if no receiver waits and the sender
    /// queue is full.
    pub fn send(&mut self, thread: u64, msg: Message) -> Result<IpcResult, IpcError> {
        if self.state == EpState::Receivers {
            let peer = self.dequeue().ok_or(IpcError::QueueFull)?;
            Ok(IpcResult::Delivered {
                peer: peer.thread,
                msg,
            })
        } else {
            self.enqueue(Waiter { thread, msg })?;
            self.state = EpState::Senders;
            Ok(IpcResult::Queued)
        }
    }

    /// Receive on behalf of `thread`. Rendezvous with a waiting sender if one
    /// is blocked, otherwise enqueue this receiver.
    ///
    /// # Errors
    /// Returns [`IpcError::QueueFull`] if no sender waits and the receiver
    /// queue is full.
    pub fn recv(&mut self, thread: u64) -> Result<IpcResult, IpcError> {
        if self.state == EpState::Senders {
            let peer = self.dequeue().ok_or(IpcError::QueueFull)?;
            Ok(IpcResult::Delivered {
                peer: peer.thread,
                msg: peer.msg,
            })
        } else {
            self.enqueue(Waiter {
                thread,
                msg: Message::empty(),
            })?;
            self.state = EpState::Receivers;
            Ok(IpcResult::Queued)
        }
    }

    /// Send from the currently running task `sender`.
    ///
    /// If no receiver waits, this records the message then blocks `sender` in
    /// the supplied task table. If a receiver is already blocked, its state is
    /// checked before the queue is consumed and it is woken to `Ready` after the
    /// transfer. This preserves endpoint/task atomicity in the single-core
    /// kernel path: refused calls leave both task state and queue untouched.
    pub fn send_task(
        &mut self,
        tasks: &mut TaskTable<'_>,
        sender: u64,
        msg: Message,
    ) -> Result<IpcResult, IpcError> {
        require_running(tasks, sender)?;
        if self.state == EpState::Receivers {
            let waiting = self.front().ok_or(IpcError::InconsistentQueue)?;
            require_state(tasks, waiting.thread, TaskState::Blocked)?;
            let peer = self.dequeue().ok_or(IpcError::InconsistentQueue)?;
            tasks
                .get_mut(peer.thread)
                .map_err(IpcError::TaskLookup)?
                .unblock()
                .map_err(IpcError::TaskState)?;
            Ok(IpcResult::Delivered {
                peer: peer.thread,
                msg,
            })
        } else {
            self.ensure_enqueue_capacity()?;
            tasks
                .get_mut(sender)
                .map_err(IpcError::TaskLookup)?
                .block()
                .map_err(IpcError::TaskState)?;
            self.enqueue(Waiter {
                thread: sender,
                msg,
            })?;
            self.state = EpState::Senders;
            Ok(IpcResult::Queued)
        }
    }

    /// Receive on behalf of the currently running task `receiver`.
    ///
    /// The symmetric task-aware operation: a receiver with no waiting sender is
    /// blocked, while a queued sender is made ready after its message is
    /// consumed. Explicit reply objects and capability transfer intentionally
    /// remain a later IPC slice.
    pub fn recv_task(
        &mut self,
        tasks: &mut TaskTable<'_>,
        receiver: u64,
    ) -> Result<IpcResult, IpcError> {
        require_running(tasks, receiver)?;
        if self.state == EpState::Senders {
            let waiting = self.front().ok_or(IpcError::InconsistentQueue)?;
            require_state(tasks, waiting.thread, TaskState::Blocked)?;
            let peer = self.dequeue().ok_or(IpcError::InconsistentQueue)?;
            tasks
                .get_mut(peer.thread)
                .map_err(IpcError::TaskLookup)?
                .unblock()
                .map_err(IpcError::TaskState)?;
            Ok(IpcResult::Delivered {
                peer: peer.thread,
                msg: peer.msg,
            })
        } else {
            self.ensure_enqueue_capacity()?;
            tasks
                .get_mut(receiver)
                .map_err(IpcError::TaskLookup)?
                .block()
                .map_err(IpcError::TaskState)?;
            self.enqueue(Waiter {
                thread: receiver,
                msg: Message::empty(),
            })?;
            self.state = EpState::Receivers;
            Ok(IpcResult::Queued)
        }
    }
}

fn require_running(tasks: &TaskTable<'_>, id: u64) -> Result<(), IpcError> {
    require_state(tasks, id, TaskState::Running)
}

fn require_state(tasks: &TaskTable<'_>, id: u64, expected: TaskState) -> Result<(), IpcError> {
    let task = tasks.get(id).map_err(IpcError::TaskLookup)?;
    if task.state() == expected {
        Ok(())
    } else {
        Err(IpcError::TaskState(TaskError::InvalidTransition))
    }
}

/// A single-producer/single-consumer ring index for bulk/streaming IPC over a
/// shared frame. This tracks only the head/tail bookkeeping; the backing
/// storage (a shared frame mapped into both parties) is supplied by the caller.
#[derive(Clone, Copy, Debug)]
pub struct VmRing {
    capacity: usize,
    head: usize,
    tail: usize,
    full: bool,
}

impl VmRing {
    /// Create a ring index over `capacity` slots.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            head: 0,
            tail: 0,
            full: false,
        }
    }

    /// Number of slots managed by this ring.
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// Whether the ring holds no entries.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.capacity == 0 || (!self.full && self.head == self.tail)
    }

    /// Whether the ring is full.
    #[must_use]
    pub const fn is_full(self) -> bool {
        self.capacity != 0 && self.full
    }

    /// Number of occupied slots.
    #[must_use]
    pub const fn len(self) -> usize {
        if self.full {
            self.capacity
        } else if self.tail >= self.head {
            self.tail - self.head
        } else {
            self.capacity - self.head + self.tail
        }
    }

    /// Reserve the next producer slot index, advancing the tail.
    /// Returns `None` if the ring is full.
    pub fn push(&mut self) -> Option<usize> {
        if self.capacity == 0 || self.full {
            return None;
        }
        let slot = self.tail;
        self.tail = (self.tail + 1) % self.capacity;
        if self.tail == self.head {
            self.full = true;
        }
        Some(slot)
    }

    /// Release the next consumer slot index, advancing the head.
    /// Returns `None` if the ring is empty.
    pub fn pop(&mut self) -> Option<usize> {
        if self.is_empty() {
            return None;
        }
        let slot = self.head;
        self.head = (self.head + 1) % self.capacity;
        self.full = false;
        Some(slot)
    }
}

/// Boot-time self-test for notifications, endpoint rendezvous, and the vmring.
///
/// # Errors
/// Returns an [`IpcError`] (used as a failure sentinel) if any invariant fails.
pub fn selftest() -> Result<(), IpcError> {
    // --- Notification: signal accumulates badges, poll reads and clears. ---
    let mut ntfn = Notification::new();
    if ntfn.pending() {
        return Err(IpcError::QueueFull);
    }
    ntfn.signal(0b0001);
    ntfn.signal(0b0100);
    if !ntfn.pending() || ntfn.poll() != 0b0101 {
        return Err(IpcError::QueueFull);
    }
    if ntfn.pending() {
        return Err(IpcError::QueueFull);
    }

    // --- Endpoint: receiver-first then sender-first rendezvous. ---
    let mut storage = [Waiter::default(); 4];
    let mut ep = Endpoint::new(&mut storage);
    let msg = Message::new(0xBADC_0DE, 0x10, &[1, 2, 3]);

    // Receiver blocks (no sender yet).
    if ep.recv(7)? != IpcResult::Queued || ep.waiting() != 1 {
        return Err(IpcError::QueueFull);
    }
    // Sender arrives -> rendezvous with thread 7, carrying the sender's msg.
    match ep.send(9, msg)? {
        IpcResult::Delivered { peer, msg: got } if peer == 7 && got == msg => {}
        _ => return Err(IpcError::QueueFull),
    }
    if ep.waiting() != 0 {
        return Err(IpcError::QueueFull);
    }

    let mut zero_ep_storage: [Waiter; 0] = [];
    let mut zero_ep = Endpoint::new(&mut zero_ep_storage);
    if !matches!(zero_ep.send(1, Message::empty()), Err(IpcError::QueueFull)) {
        return Err(IpcError::QueueFull);
    }

    // Now sender-first: sender blocks, receiver completes it.
    if ep.send(11, msg)? != IpcResult::Queued {
        return Err(IpcError::QueueFull);
    }
    match ep.recv(13)? {
        IpcResult::Delivered { peer, msg: got }
            if peer == 11 && got.word(2) == Some(3) && got.badge() == 0xBADC_0DE => {}
        _ => return Err(IpcError::QueueFull),
    }
    if ep.waiting() != 0 {
        return Err(IpcError::QueueFull);
    }

    // --- VmRing: fill, detect full, drain, detect empty (FIFO order). ---

    let mut zero = VmRing::new(0);
    if zero.capacity() != 0
        || !zero.is_empty()
        || zero.is_full()
        || zero.push().is_some()
        || zero.pop().is_some()
    {
        return Err(IpcError::QueueFull);
    }

    let mut ring = VmRing::new(3);
    if !ring.is_empty() {
        return Err(IpcError::QueueFull);
    }
    let a = ring.push().ok_or(IpcError::QueueFull)?;
    let b = ring.push().ok_or(IpcError::QueueFull)?;
    let c = ring.push().ok_or(IpcError::QueueFull)?;
    if !ring.is_full() || ring.push().is_some() || ring.len() != 3 {
        return Err(IpcError::QueueFull);
    }
    if ring.pop() != Some(a) || ring.pop() != Some(b) || ring.pop() != Some(c) {
        return Err(IpcError::QueueFull);
    }
    if !ring.is_empty() || ring.pop().is_some() {
        return Err(IpcError::QueueFull);
    }

    Ok(())
}

#[cfg(test)]
mod task_endpoint_tests {
    extern crate std;

    use super::*;
    use crate::cap::{CapType, Capability, Rights};
    use crate::task::TaskControlBlock;
    use hull::user::UserFrame;

    fn task(id: u64) -> TaskControlBlock {
        let cspace = Capability::new(CapType::CNode, 0x40_0000 + id * 0x1000, 5, Rights::ALL);
        TaskControlBlock::new(
            id,
            cspace,
            0x80_0000 + id * 0x1000,
            UserFrame::new(0x4000, 0x8000),
        )
        .unwrap()
    }

    #[test]
    fn receiver_first_blocks_then_is_woken_by_sender() {
        let mut task_storage = [task(1), task(2)];
        let mut tasks = TaskTable::new(&mut task_storage).unwrap();
        tasks.get_mut(2).unwrap().start().unwrap();
        let mut waiters = [Waiter::default(); 2];
        let mut endpoint = Endpoint::new(&mut waiters);
        let msg = Message::new(0x44, 7, &[1, 2]);

        assert_eq!(endpoint.recv_task(&mut tasks, 2), Ok(IpcResult::Queued));
        assert_eq!(tasks.get(2).unwrap().state(), TaskState::Blocked);
        assert_eq!(endpoint.waiting(), 1);

        tasks.get_mut(1).unwrap().start().unwrap();
        assert_eq!(
            endpoint.send_task(&mut tasks, 1, msg),
            Ok(IpcResult::Delivered { peer: 2, msg })
        );
        assert_eq!(tasks.get(1).unwrap().state(), TaskState::Running);
        assert_eq!(tasks.get(2).unwrap().state(), TaskState::Ready);
        assert_eq!(endpoint.waiting(), 0);
    }

    #[test]
    fn sender_first_blocks_then_is_woken_by_receiver() {
        let mut task_storage = [task(1), task(2)];
        let mut tasks = TaskTable::new(&mut task_storage).unwrap();
        tasks.get_mut(1).unwrap().start().unwrap();
        let mut waiters = [Waiter::default(); 2];
        let mut endpoint = Endpoint::new(&mut waiters);
        let msg = Message::new(0x55, 8, &[3, 4]);

        assert_eq!(
            endpoint.send_task(&mut tasks, 1, msg),
            Ok(IpcResult::Queued)
        );
        assert_eq!(tasks.get(1).unwrap().state(), TaskState::Blocked);
        tasks.get_mut(2).unwrap().start().unwrap();
        assert_eq!(
            endpoint.recv_task(&mut tasks, 2),
            Ok(IpcResult::Delivered { peer: 1, msg })
        );
        assert_eq!(tasks.get(1).unwrap().state(), TaskState::Ready);
        assert_eq!(tasks.get(2).unwrap().state(), TaskState::Running);
        assert_eq!(endpoint.waiting(), 0);
    }

    #[test]
    fn non_running_or_full_queue_rejection_is_non_destructive() {
        let mut task_storage = [task(1), task(2)];
        let mut tasks = TaskTable::new(&mut task_storage).unwrap();
        let mut waiters = [Waiter::default(); 1];
        let mut endpoint = Endpoint::new(&mut waiters);
        let msg = Message::new(0, 1, &[]);

        assert_eq!(
            endpoint.send_task(&mut tasks, 1, msg),
            Err(IpcError::TaskState(TaskError::InvalidTransition))
        );
        assert_eq!(endpoint.waiting(), 0);
        assert_eq!(tasks.get(1).unwrap().state(), TaskState::Ready);

        tasks.get_mut(1).unwrap().start().unwrap();
        assert_eq!(
            endpoint.send_task(&mut tasks, 1, msg),
            Ok(IpcResult::Queued)
        );
        assert_eq!(tasks.get(1).unwrap().state(), TaskState::Blocked);
        tasks.get_mut(2).unwrap().start().unwrap();
        assert_eq!(
            endpoint.send_task(&mut tasks, 2, msg),
            Err(IpcError::QueueFull)
        );
        assert_eq!(tasks.get(2).unwrap().state(), TaskState::Running);
        assert_eq!(endpoint.waiting(), 1);
    }
}
