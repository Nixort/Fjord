//! # Keel — the Fjord microkernel
//!
//! Keel is a capability-based microkernel (seL4 lineage). It is the only
//! component, besides [`anchor`] and the crypto core, that runs fully
//! privileged. It implements *mechanism, not policy*: capabilities, address
//! spaces, IPC and scheduling. Drivers, filesystems and network stacks all
//! live in userspace.
//!
//! See `docs/ARCHITECTURE.md` §1.
#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod cap;
pub mod vspace;
pub mod ipc;
pub mod tide;
pub mod untyped;

/// Kernel entry point, invoked by [`anchor`]/[`hull`] after the MMU is live.
///
/// TODO(keel): set up the root CSpace, hand initial untyped memory to `Helm`,
/// and start the first userspace task. Must never return.
pub fn kmain() -> ! {
    // TODO(keel): init subsystems in order: cap -> vspace -> tide -> ipc.
    todo!("Keel boot sequence — see ROADMAP Phase 2")
}
