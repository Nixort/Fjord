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
pub mod cte;
pub mod vspace;
pub mod ipc;
pub mod irqhandler;
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

    // Fused capability-table entries: prove retype-from-untyped, rights-reduced
    // mint, sibling copy, derivation-preserving move, and recursive revoke all
    // drive real CSpace slots through a single structure (cap + MDB link).
    match cte::selftest() {
        Ok(()) => hull::kprintln!("keel: cte self-test -> retype/mint/revoke OK"),
        Err(e) => hull::kprintln!("keel: WARNING cte self-test failed: {e:?}"),
    }

    match vspace::selftest() {
        Ok(()) => hull::kprintln!("keel: vspace self-test -> map/translate/unmap OK"),
        Err(e) => hull::kprintln!("keel: WARNING vspace self-test failed: {e:?}"),
    }

    // VSpace <-> Hull integration: drive real hardware page tables through the
    // keel vspace bridge, in an inactive scratch address space so the running
    // kernel's own mapping is left untouched.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    match vspace::hw_selftest(&mut frames) {
        Ok(()) => hull::kprintln!("keel: vspace<->hull self-test -> hw map/unmap OK"),
        Err(e) => hull::kprintln!("keel: WARNING vspace<->hull self-test failed: {e:?}"),
    }

    match ipc::selftest() {
        Ok(()) => hull::kprintln!("keel: ipc self-test -> ntfn/endpoint/vmring OK"),
        Err(e) => hull::kprintln!("keel: WARNING ipc self-test failed: {e:?}"),
    }

    match tide::selftest() {
        Ok(()) => hull::kprintln!("keel: tide self-test -> priority/budget/block OK"),
        Err(e) => hull::kprintln!("keel: WARNING tide self-test failed: {e:?}"),
    }

    // Tide context switch: prove the real callee-saved register save/restore and
    // stack handoff by switching into a freshly built context and straight back
    // -- the mechanism the timer-tick preemptive scheduler will drive.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    match tide::ctx_selftest() {
        Ok(()) => {
            hull::kprintln!("keel: tide ctx-switch self-test -> save/restore/handoff OK")
        }
        Err(e) => {
            hull::kprintln!("keel: WARNING tide ctx-switch self-test failed: {e:?}")
        }
    }

    // Tide preemptive scheduling: prove the platform timer alone can interleave
    // two non-cooperative worker contexts. The boot context installs a tick
    // hook that round-robins the workers from the timer ISR, then verifies both
    // ran and that the timer drove the full schedule.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    match tide::preempt_selftest() {
        Ok(s) => hull::kprintln!(
            "keel: tide preempt self-test -> {} timer-driven switches (A={}, B={}) OK",
            s.switches,
            s.worker_a,
            s.worker_b
        ),
        Err(e) => hull::kprintln!("keel: WARNING tide preempt self-test failed: {e:?}"),
    }

    // IRQ-as-capability: prove that a platform timer interrupt is delivered
    // to a Keel Notification as a badge, via the Hull IRQ hook — the seL4
    // IRQHandler model, where the kernel *delivers* interrupts rather than
    // servicing them.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    match irqhandler::selftest() {
        Ok(badge) => hull::kprintln!(
            "keel: irq-handler self-test -> badge {badge:#x} delivered OK"
        ),
        Err(e) => hull::kprintln!("keel: WARNING irq-handler self-test failed: {e:?}"),
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

/// Build the aarch64 kernel address space and arm the platform timer.
///
/// Installs the Hull-built per-section W^X mapping (TTBR0), then brings up the
/// GIC v2 + ARM generic timer so the scheduler gains a tick source, mirroring
/// the x86_64 local-APIC timer path.
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
