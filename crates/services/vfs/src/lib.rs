//! # vfs — virtual filesystem + namespacing
//!
//! Deprivileged userspace service running over `fjord-rt`, reachable only via
//! capabilities granted by Helm. See `docs/ARCHITECTURE.md` §8.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// Service entry point; driven by the async runtime.
/// TODO(vfs): handle requests over its IPC endpoint.
pub fn run() -> ! {
    todo!("vfs service loop")
}
