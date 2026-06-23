// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Inter-process communication.
//!
//! Three mechanisms: synchronous endpoints (with a fast-path migrating-thread
//! call), asynchronous notifications, and shared-memory rings (`vmrings`).
//!
//! See `docs/ARCHITECTURE.md` §1.

/// A synchronous rendezvous endpoint capability.
pub struct Endpoint; // TODO(keel): sender/receiver queues, badges.

/// Perform a synchronous call (send + block for reply).
///
/// TODO(keel): implement fast-path migrating-thread call (donate scheduling
/// context to the callee to avoid a full reschedule).
pub fn call(_ep: &Endpoint) { todo!("sync IPC call") }

/// A lock-free shared-memory ring for bulk/streaming IPC.
pub struct VmRing; // TODO(keel): SPSC ring over shared frames, head/tail.
