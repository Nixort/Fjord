// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 24 june 2026

//! aarch64 stage-1 paging: a 4-level (4 KiB granule, 48-bit VA) translation
//! hierarchy built and activated with per-section W^X, mirroring the x86_64
//! [`crate::paging`] module.
//!
//! The boot shim brings the CPU up with the MMU *off*, so the whole image is
//! implicitly accessible and every data access is Device-nGnRnE. This module
//! builds an identity map in which `.text` is R-X, `.rodata` is R--, `.data`/
//! `.bss` (and spare RAM) are RW + never-execute, and the QEMU `virt` MMIO
//! window below RAM is Device memory, then enables the MMU and caches.
//!
//! Addresses are specific to the QEMU `virt` machine (RAM at 0x4000_0000),
//! consistent with the hard-coded GIC/UART bases elsewhere in Hull. Higher-half
//! relocation and per-task address spaces are deferred to later ROADMAP steps.

use crate::mmu::{FrameAllocator, FRAME_SIZE};
use core::ptr::addr_of;

/// Number of 64-bit descriptors in every translation table.
pub const ENTRY_COUNT: usize = 512;
/// Bytes mapped by a level-2 block descriptor: 2 MiB.
pub const BLOCK_2M: u64 = 2 * 1024 * 1024;
/// Base of usable RAM on the QEMU `virt` machine; everything below is MMIO.
pub const RAM_BASE: u64 = 0x4000_0000;

// Descriptor type bits [1:0].
const PTE_VALID: u64 = 1 << 0;
/// Distinguishes table/page (0b11) from block (0b01) descriptors.
const PTE_TABLE: u64 = 1 << 1;

// Lower attributes.
const ATTR_NORMAL: u64 = 0 << 2; // MAIR index 0: Normal write-back
const ATTR_DEVICE: u64 = 1 << 2; // MAIR index 1: Device-nGnRnE
const AP_RO: u64 = 1 << 7; // AP[2] = 1: read-only at EL1
const SH_INNER: u64 = 0b11 << 8; // inner shareable
const AF: u64 = 1 << 10; // access flag (fault if clear)

// Upper attributes.
const PXN: u64 = 1 << 53; // privileged execute-never (EL1)
const UXN: u64 = 1 << 54; // unprivileged execute-never (EL0)

/// Leaf attributes for kernel code: Normal, read-only, EL1-executable.
const NORMAL_TEXT: u64 = ATTR_NORMAL | AP_RO | SH_INNER | AF | UXN;
/// Leaf attributes for constants: Normal, read-only, never-execute.
const NORMAL_RODATA: u64 = ATTR_NORMAL | AP_RO | SH_INNER | AF | PXN | UXN;
/// Leaf attributes for data / spare RAM: Normal, read-write, never-execute.
const NORMAL_DATA: u64 = ATTR_NORMAL | SH_INNER | AF | PXN | UXN;
/// Leaf attributes for MMIO: Device, read-write, never-execute.
const DEVICE_RW: u64 = ATTR_DEVICE | AF | PXN | UXN;

/// Mask selecting the output-address field of a descriptor (bits 12..=47).
const ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

/// Round `value` up to the next multiple of `align` (a power of two).
const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

const fn l0_index(va: u64) -> usize {
    ((va >> 39) & 0x1ff) as usize
}
const fn l1_index(va: u64) -> usize {
    ((va >> 30) & 0x1ff) as usize
}
const fn l2_index(va: u64) -> usize {
    ((va >> 21) & 0x1ff) as usize
}
const fn l3_index(va: u64) -> usize {
    ((va >> 12) & 0x1ff) as usize
}

/// `true` if `attrs` would map memory both writable and EL1-executable.
const fn breaks_wxn(attrs: u64) -> bool {
    let writable = attrs & AP_RO == 0;
    let el1_exec = attrs & PXN == 0;
    writable && el1_exec
}

/// A single 4 KiB translation table: 512 64-bit descriptors.
#[repr(C, align(4096))]
struct Table {
    entries: [u64; ENTRY_COUNT],
}

impl Table {
    /// Reinterpret the identity-mapped frame at physical address `pa` as a table.
    ///
    /// # Safety
    /// `pa` must be a 4 KiB-aligned frame reachable at its physical address and
    /// not aliased mutably elsewhere for the duration of `'a`.
    unsafe fn from_phys<'a>(pa: u64) -> &'a mut Table {
        // SAFETY: the caller guarantees `pa` is a live, aligned, reachable frame.
        unsafe { &mut *(pa as *mut Table) }
    }
}

/// A 4-level translation-table mapper rooted at a physical L0 frame.
pub struct Mapper {
    root: u64,
}

impl Mapper {
    /// Allocate and zero a fresh L0 table, yielding an empty address space.
    pub fn new(alloc: &mut FrameAllocator) -> Option<Mapper> {
        Some(Mapper { root: alloc_zeroed(alloc)? })
    }

    /// Physical address of the root table; load it into TTBR0_EL1 to activate.
    pub fn root(&self) -> u64 {
        self.root
    }

    /// Return the next-level table referenced by `table[index]`, creating and
    /// linking a zeroed one if absent.
    fn ensure_table(
        table: &mut Table,
        index: usize,
        alloc: &mut FrameAllocator,
    ) -> Option<u64> {
        let entry = table.entries[index];
        if entry & PTE_VALID != 0 {
            return Some(entry & ADDR_MASK);
        }
        let frame = alloc_zeroed(alloc)?;
        table.entries[index] = (frame & ADDR_MASK) | PTE_TABLE | PTE_VALID;
        Some(frame)
    }

    /// Map one 4 KiB page `va -> pa` with leaf `attrs`. Refuses W^X violations.
    pub fn map_4k(&mut self, va: u64, pa: u64, attrs: u64, alloc: &mut FrameAllocator) -> bool {
        if breaks_wxn(attrs) {
            return false;
        }
        // SAFETY: every table frame is reachable at its physical address and
        // uniquely owned by this mapper while we hold `&mut self`.
        unsafe {
            let l0 = Table::from_phys(self.root);
            let l1 = match Self::ensure_table(l0, l0_index(va), alloc) {
                Some(p) => Table::from_phys(p),
                None => return false,
            };
            let l2 = match Self::ensure_table(l1, l1_index(va), alloc) {
                Some(p) => Table::from_phys(p),
                None => return false,
            };
            let l3 = match Self::ensure_table(l2, l2_index(va), alloc) {
                Some(p) => Table::from_phys(p),
                None => return false,
            };
            l3.entries[l3_index(va)] = (pa & ADDR_MASK) | attrs | PTE_TABLE | PTE_VALID;
        }
        true
    }

    /// Map one 2 MiB block `va -> pa` at L2. Both must be 2 MiB-aligned.
    pub fn map_2m(&mut self, va: u64, pa: u64, attrs: u64, alloc: &mut FrameAllocator) -> bool {
        if breaks_wxn(attrs) {
            return false;
        }
        if va & (BLOCK_2M - 1) != 0 || pa & (BLOCK_2M - 1) != 0 {
            return false;
        }
        // SAFETY: see `map_4k`.
        unsafe {
            let l0 = Table::from_phys(self.root);
            let l1 = match Self::ensure_table(l0, l0_index(va), alloc) {
                Some(p) => Table::from_phys(p),
                None => return false,
            };
            let l2 = match Self::ensure_table(l1, l1_index(va), alloc) {
                Some(p) => Table::from_phys(p),
                None => return false,
            };
            // Block descriptor: type bits [1:0] = 0b01 (PTE_VALID without PTE_TABLE).
            l2.entries[l2_index(va)] = (pa & ADDR_MASK) | attrs | PTE_VALID;
        }
        true
    }
}

/// Allocate a frame and zero it (reachable because frames are identity-mapped).
fn alloc_zeroed(alloc: &mut FrameAllocator) -> Option<u64> {
    let frame = alloc.alloc()?;
    // SAFETY: `frame` is a fresh, page-aligned physical frame nothing else
    // references yet; while the MMU is off it is directly addressable.
    unsafe {
        core::ptr::write_bytes(frame as *mut u8, 0, FRAME_SIZE as usize);
    }
    Some(frame)
}

extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __kernel_end: u8;
}

/// Page-aligned kernel section boundaries, emitted by `boot/linker-aarch64.ld`.
struct KernelLayout {
    text_start: u64,
    text_end: u64,
    rodata_start: u64,
    rodata_end: u64,
    kernel_end: u64,
}

fn kernel_layout() -> KernelLayout {
    // `addr_of!` only forms pointers to linker-defined symbols; nothing is
    // dereferenced, so no `unsafe` is required.
    KernelLayout {
        text_start: addr_of!(__text_start) as u64,
        text_end: addr_of!(__text_end) as u64,
        rodata_start: addr_of!(__rodata_start) as u64,
        rodata_end: addr_of!(__rodata_end) as u64,
        kernel_end: addr_of!(__kernel_end) as u64,
    }
}

/// Leaf attributes for an identity-mapped RAM page at `va`.
fn page_attrs(layout: &KernelLayout, va: u64) -> u64 {
    if va >= layout.text_start && va < layout.text_end {
        NORMAL_TEXT
    } else if va >= layout.rodata_start && va < layout.rodata_end {
        NORMAL_RODATA
    } else {
        // .data/.bss, the boot stack, page tables and spare RAM below/above.
        let _ = layout.rodata_start; // (boundaries already consulted above)
        NORMAL_DATA
    }
}

/// Build a fresh kernel address space with per-section W^X and activate it.
///
/// Identity-maps the QEMU `virt` MMIO window `[0, RAM_BASE)` as Device memory,
/// the 2 MiB block holding the kernel image as 4 KiB pages with exact W^X
/// permissions, and the remaining usable RAM up to `ram_top` as 2 MiB Normal
/// RW-NX blocks. Returns the physical root loaded into TTBR0_EL1, or `None` if
/// a backing frame could not be obtained or `ram_top` is unusable.
pub fn init_kernel_address_space(alloc: &mut FrameAllocator, ram_top: u64) -> Option<u64> {
    let layout = kernel_layout();
    let ram_end = ram_top & !(BLOCK_2M - 1);
    if ram_end <= RAM_BASE {
        return None;
    }
    let mut mapper = Mapper::new(alloc)?;

    // 1. MMIO window below RAM: Device, RW, never-execute (covers UART + GIC).
    let mut va = 0;
    while va < RAM_BASE {
        if !mapper.map_2m(va, va, DEVICE_RW, alloc) {
            return None;
        }
        va += BLOCK_2M;
    }

    // 2. The 2 MiB block containing the kernel image: 4 KiB pages, per-section
    //    W^X so .text stays R-X while everything else is RW/RO + NX.
    let fine_start = layout.text_start & !(BLOCK_2M - 1);
    let fine_top = align_up(layout.kernel_end, BLOCK_2M);
    let mut va = fine_start;
    while va < fine_top {
        if !mapper.map_4k(va, va, page_attrs(&layout, va), alloc) {
            return None;
        }
        va += FRAME_SIZE;
    }

    // 3. Remaining usable RAM: 2 MiB Normal RW-NX blocks.
    let mut va = fine_top;
    while va + BLOCK_2M <= ram_end {
        if !mapper.map_2m(va, va, NORMAL_DATA, alloc) {
            return None;
        }
        va += BLOCK_2M;
    }

    crate::kprintln!("hull: enabling MMU (TTBR0={:#x})", mapper.root());
    // SAFETY: the hierarchy maps the running code, the active stack, the page
    // tables themselves and the console/GIC MMIO, so execution survives the
    // switch and the timer bring-up that follows can reach the GIC.
    unsafe {
        activate(mapper.root());
    }
    Some(mapper.root())
}

/// Program MAIR/TCR/TTBR0, invalidate caches + TLB, then enable the MMU and
/// caches in SCTLR_EL1.
///
/// # Safety
/// `root` must reference a valid L0 table that identity-maps the currently
/// executing instructions, the active stack, the page tables and the console
/// MMIO; otherwise the first post-enable access faults.
unsafe fn activate(root: u64) {
    use core::arch::asm;

    // MAIR: attr0 = Normal Inner/Outer write-back RW-allocate (0xFF);
    //       attr1 = Device-nGnRnE (0x00).
    let mair: u64 = 0x00FF;
    // TCR_EL1 for a single 48-bit TTBR0 region, 4 KiB granule:
    //   T0SZ=16, IRGN0=ORGN0=WB-WA, SH0=inner, TG0=4KiB, EPD1=1, IPS=40-bit.
    let tcr: u64 = 16
        | (1 << 8)
        | (1 << 10)
        | (0b11 << 12)
        | (1 << 23)
        | (0b010 << 32);

    // SAFETY: programming translation control and barriers at EL1; `root`
    // satisfies the mapping requirements documented above.
    unsafe {
        asm!(
            "msr mair_el1, {mair}",
            "msr tcr_el1, {tcr}",
            "msr ttbr0_el1, {root}",
            "dsb ish",
            "tlbi vmalle1",
            "dsb ish",
            "ic iallu",
            "isb",
            mair = in(reg) mair,
            tcr = in(reg) tcr,
            root = in(reg) root,
            options(nostack, preserves_flags),
        );

        let mut sctlr: u64;
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags));
        sctlr |= 1 << 0; // M: enable stage-1 MMU
        sctlr |= 1 << 2; // C: data + unified cache
        sctlr |= 1 << 12; // I: instruction cache
        asm!(
            "msr sctlr_el1, {sctlr}",
            "isb",
            sctlr = in(reg) sctlr,
            options(nostack, preserves_flags),
        );
    }
}
