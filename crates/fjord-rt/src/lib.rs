// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # fjord-rt — userspace async runtime
//!
//! A small async executor that maps `Future`s onto Keel IPC and notifications,
//! plus timers from `timed`. It is the substrate every service runs on.
//! See `docs/ARCHITECTURE.md` §9.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// Run a future to completion on the current thread's scheduling context.
/// TODO(fjord-rt): reactor driven by notification wakeups; timer wheel.
pub fn block_on<F>(_fut: F) {
    todo!("single-threaded executor over IPC notifications")
}
