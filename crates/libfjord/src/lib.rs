// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # libfjord — userspace syscall + capability bindings
//!
//! Typed, capability-checked wrappers around Keel's IPC ABI. Application and
//! service code links this instead of issuing raw syscalls.
//! See `docs/ARCHITECTURE.md` §9.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// A userspace handle to a capability held in the task's CSpace.
pub struct Cap(pub u32); // index into the CSpace

/// Invoke an endpoint capability with a typed message.
/// TODO(libfjord): serialize message, perform `keel::ipc::call`, map errors.
pub fn invoke(_cap: Cap) { todo!("typed capability invocation") }
