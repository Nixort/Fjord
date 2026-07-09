// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
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

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

fn raw_waker() -> RawWaker {
    RawWaker::new(core::ptr::null(), &VTABLE)
}

unsafe fn clone(_: *const ()) -> RawWaker {
    raw_waker()
}

unsafe fn wake(_: *const ()) {}
unsafe fn wake_by_ref(_: *const ()) {}
unsafe fn drop(_: *const ()) {}

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);

fn noop_waker() -> Waker {
    // SAFETY: the vtable never dereferences the null data pointer and every
    // operation is a no-op, which is valid for the cooperative bootstrap
    // executor used before notification-backed wakeups are wired in.
    unsafe { Waker::from_raw(raw_waker()) }
}

/// Run a future to completion on the current thread's scheduling context.
///
/// This bootstrap executor is deliberately tiny: it polls the future until it
/// completes and spins between pending polls. Later slices can replace the
/// no-op waker with a notification-backed reactor without changing the service
/// entry-point contract.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut fut = core::pin::pin!(fut);
    loop {
        match Pin::as_mut(&mut fut).poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => core::hint::spin_loop(),
        }
    }
}
