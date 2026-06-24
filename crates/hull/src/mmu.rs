// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! MMU / physical-memory primitives consumed by `keel::vspace`.
//!
//! For now this provides the earliest building block: a one-way *bump* frame
//! allocator over the usable regions of the PVH [`BootInfo`](crate::boot). It
//! exists so the kernel can obtain backing physical frames (page tables, the
//! heap arena) before a real allocator or paging code is in place.
//!
//! TODO(hull): page-table walk/modify, higher-half mapping, W^X enforcement,
//! TLB shootdown and frame recycling once Keel owns the vspace.

use crate::boot::BootInfo;

/// Size of a physical frame on x86_64 (4 KiB).
pub const FRAME_SIZE: u64 = 4096;

extern "C" {
    /// End of the loaded kernel image; defined by `boot/linker.ld`.
    static __kernel_end: u8;
}

/// Physical end of the kernel image.
///
/// The kernel is identity-mapped, so its link-time address equals its physical
/// address.
fn kernel_end() -> u64 {
    // SAFETY: we only take the address of the linker symbol, never read it.
    unsafe { core::ptr::addr_of!(__kernel_end) as u64 }
}

/// Round `value` up to the next multiple of `align` (a power of two).
const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

/// One-way bump allocator handing out 4 KiB physical frames from usable RAM.
///
/// Frames below `floor` are never returned. `floor` is the page-aligned end of
/// the kernel image (clamped to at least 1 MiB), which conservatively covers
/// the low legacy region, the boot stack, the bootstrap page tables and the
/// loader's `hvm_start_info` — all of which live below it. Frames are not
/// recycled; this allocator backs permanent early structures only.
pub struct FrameAllocator {
    boot: BootInfo,
    floor: u64,
    idx: usize,
    next: u64,
    handed_out: u64,
}

impl FrameAllocator {
    /// Build an allocator over the usable regions of `boot`.
    pub fn new(boot: &BootInfo) -> Self {
        let floor = align_up(kernel_end().max(0x10_0000), FRAME_SIZE);
        let mut alloc = Self {
            boot: *boot,
            floor,
            idx: 0,
            next: 0,
            handed_out: 0,
        };
        alloc.enter_region();
        alloc
    }

    /// Position `idx`/`next` at the first allocatable frame at or after the
    /// current region, skipping reserved or fully-consumed regions.
    fn enter_region(&mut self) {
        while self.idx < self.boot.region_count() {
            let r = self.boot.region(self.idx);
            if r.kind.is_usable() {
                let start = align_up(r.start.max(self.floor), FRAME_SIZE);
                if start + FRAME_SIZE <= r.end() {
                    self.next = start;
                    return;
                }
            }
            self.idx += 1;
        }
    }

    /// Allocate one physical frame, returning its base physical address.
    pub fn alloc(&mut self) -> Option<u64> {
        while self.idx < self.boot.region_count() {
            let end = self.boot.region(self.idx).end();
            if self.next + FRAME_SIZE <= end {
                let frame = self.next;
                self.next += FRAME_SIZE;
                self.handed_out += 1;
                return Some(frame);
            }
            self.idx += 1;
            self.enter_region();
        }
        None
    }

    /// Page-aligned floor below which no frame is ever handed out.
    pub fn floor(&self) -> u64 {
        self.floor
    }

    /// Number of frames handed out so far.
    pub fn handed_out(&self) -> u64 {
        self.handed_out
    }

    /// Total frames available above `floor` across all usable regions.
    ///
    /// Computed independently of allocation state, so it reflects capacity.
    pub fn capacity_frames(&self) -> u64 {
        let mut frames = 0;
        for i in 0..self.boot.region_count() {
            let r = self.boot.region(i);
            if !r.kind.is_usable() {
                continue;
            }
            let start = align_up(r.start.max(self.floor), FRAME_SIZE);
            if start + FRAME_SIZE <= r.end() {
                frames += (r.end() - start) / FRAME_SIZE;
            }
        }
        frames
    }
}
