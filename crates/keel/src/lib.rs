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
pub mod cdt;
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
    // address space and arm the platform timer. On x86_64 this installs the
    // per-section W^X mapping plus the local APIC periodic timer; other
    // architectures defer MMU + interrupt-controller bring-up to a later
    // Phase 1 slice and keep running on the bootstrap mapping.
    activate_address_space(&mut frames);

    // Capability-space core smoke test (Phase 2 groundwork): prove mint/copy/
    // move/delete bookkeeping and monotonic rights reduction before any real
    // CSpace is retyped from untyped memory.
    match cap::selftest() {
        Ok(()) => hull::kprintln!("keel: cspace self-test -> mint/copy/move/delete OK"),
        Err(e) => hull::kprintln!("keel: WARNING cspace self-test failed: {e:?}"),
    }

    match untyped::selftest() {
        Ok(()) => hull::kprintln!("keel: untyped self-test -> retype 3 pages OK"),
        Err(e) => hull::kprintln!("keel: WARNING untyped self-test failed: {e:?}"),
    }

    match cdt::selftest() {
        Ok(()) => hull::kprintln!("keel: cdt self-test -> derive/revoke/delete OK"),
        Err(e) => hull::kprintln!("keel: WARNING cdt self-test failed: {e:?}"),
    }

    match vspace::selftest() {
        Ok(()) => hull::kprintln!("keel: vspace self-test -> map/translate/unmap OK"),
        Err(e) => hull::kprintln!("keel: WARNING vspace self-test failed: {e:?}"),
    }

    hull::kprintln!("keel: early console up; entering idle (Phase 2 boot pending).");

    // TODO(keel): init subsystems in order cap -> vspace -> tide -> ipc, then
    // launch the first userspace task.
    idle()
}

/// Build the kernel address space and arm the platform timer (x86_64).
///
/// Installs the Hull-built per-section W^X mapping, then brings up the local
/// APIC periodic timer on top of it.
#[cfg(target_arch = "x86_64")]
fn activate_address_space(frames: &mut FrameAllocator) {
    match hull::paging::init_kernel_address_space(frames) {
        Some(root) => {
            hull::kprintln!(
                "keel: kernel address space active — per-section W^X, CR3={root:#x}"
            );
            if hull::apic::init_timer(root, frames) {
                hull::kprintln!("keel: local APIC + periodic timer armed (vector 0x40)");
            } else {
                hull::kprintln!("keel: WARNING could not bring up local APIC timer");
            }
        }
        None => hull::kprintln!(
            "keel: WARNING could not build kernel address space; on bootstrap tables"
        ),
    }
}

/// Arm the aarch64 platform timer (GIC v2 + the ARM generic timer).
///
/// The aarch64 MMU (per-section W^X paging) is still a later Phase 1 slice, so
/// we keep running on the bootstrap flat map; but the interrupt controller and
/// generic timer come up now so the scheduler gains a tick source, mirroring
/// the x86_64 local-APIC timer.
#[cfg(target_arch = "aarch64")]
fn activate_address_space(frames: &mut FrameAllocator) {
    let ram_top = frames.usable_top();
    match hull::paging::init_kernel_address_space(frames, ram_top) {
        Some(ttbr0) => hull::kprintln!(
            "keel: kernel address space active — per-section W^X, TTBR0={ttbr0:#x}"
        ),
        None => hull::kprintln!(
            "keel: WARNING could not build kernel address space; on bootstrap flat map"
        ),
    }
    if hull::gic::init_timer() {
        hull::kprintln!("keel: GIC v2 + generic timer armed (PPI 30)");
    } else {
        hull::kprintln!("keel: WARNING generic timer unavailable (CNTFRQ_EL0 = 0)");
    }
}

/// Portable fallback for architectures without an MMU/timer backend yet.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn activate_address_space(_frames: &mut FrameAllocator) {
    hull::kprintln!("keel: MMU + timer bring-up deferred (no backend for this arch)");
}

/// Park the CPU, waking only to service interrupts (e.g. the periodic timer).
fn idle() -> ! {
    loop {
        // SAFETY: `hlt`/`wfi` halt until the next interrupt; other targets spin.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        core::hint::spin_loop();
    }
}
