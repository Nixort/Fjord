// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

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

use hull::boot::BootInfo;
use hull::mmu::{FrameAllocator, FRAME_SIZE};

pub mod cap;
pub mod vspace;
pub mod ipc;
pub mod tide;
pub mod untyped;

/// Target architecture, resolved at compile time for the boot banner.
#[cfg(target_arch = "x86_64")]
pub const ARCH: &str = "x86_64";
/// Target architecture, resolved at compile time for the boot banner.
#[cfg(target_arch = "aarch64")]
pub const ARCH: &str = "aarch64";
/// Target architecture, resolved at compile time for the boot banner.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub const ARCH: &str = "unknown";

/// Kernel entry point, invoked by the `boot` shim once a stack exists.
///
/// Brings up the early serial console, prints a boot banner, logs the physical
/// memory map handed over by the loader, and stands up the early physical
/// frame allocator so later bring-up has a source of backing memory. The real
/// boot sequence (root CSpace, untyped hand-off to `Helm`, first userspace
/// task) lands in ROADMAP Phase 2; until then we park the CPU in a low-power
/// idle loop. Must never return.
pub fn kmain(boot: &BootInfo) -> ! {
    let _serial = hull::serial::Serial::init();

    hull::kprintln!();
    hull::kprintln!("Fjord OS — keel v{}", env!("CARGO_PKG_VERSION"));
    hull::kprintln!("  arch    : {ARCH}");
    hull::kprintln!(
        "  profile : {}",
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );

    // Physical memory: log the map, then bring up the early frame allocator.
    boot.log();
    let mut frames = FrameAllocator::new(boot);
    let capacity = frames.capacity_frames();
    let usable_mib = capacity.saturating_mul(FRAME_SIZE) / (1024 * 1024);
    hull::kprintln!(
        "keel: frame allocator online — {} free frames (~{} MiB) above {:#x}",
        capacity,
        usable_mib,
        frames.floor()
    );

    // Smoke-test: pull a few frames to prove the allocator hands out distinct,
    // page-aligned physical addresses.
    if let (Some(a), Some(b), Some(c)) = (frames.alloc(), frames.alloc(), frames.alloc()) {
        hull::kprintln!("keel: frame self-test -> {a:#x} {b:#x} {c:#x}");
    } else if capacity > 0 {
        hull::kprintln!("keel: WARNING frame self-test failed despite reported capacity");
    }

    // Replace the throwaway bootstrap identity map with a Hull-built kernel
    // address space that enforces per-section W^X, then prove we survived the
    // CR3 switch by continuing to run and log from it.
    match hull::paging::init_kernel_address_space(&mut frames) {
        Some(root) => {
            hull::kprintln!(
                "keel: kernel address space active — per-section W^X, CR3={root:#x}"
            );
            start_timer(root, &mut frames);
        }
        None => hull::kprintln!(
            "keel: WARNING could not build kernel address space; on bootstrap tables"
        ),
    }

    hull::kprintln!("keel: early console up; entering idle (Phase 2 boot pending).");

    // TODO(keel): init subsystems in order cap -> vspace -> tide -> ipc, then
    // launch the first userspace task.
    idle()
}

/// Bring up the local APIC periodic timer on architectures that support it.
#[cfg(target_arch = "x86_64")]
fn start_timer(root: u64, frames: &mut FrameAllocator) {
    if hull::apic::init_timer(root, frames) {
        hull::kprintln!("keel: local APIC + periodic timer armed (vector 0x40)");
    } else {
        hull::kprintln!("keel: WARNING could not bring up local APIC timer");
    }
}

/// Portable fallback until other architectures grow an interrupt controller.
#[cfg(not(target_arch = "x86_64"))]
fn start_timer(_root: u64, _frames: &mut FrameAllocator) {}

/// Park the CPU, waking only to service interrupts (e.g. the periodic timer).
fn idle() -> ! {
    loop {
        // SAFETY: `hlt` halts until the next interrupt; portable targets spin.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}
