// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! x86_64 4-level paging: page-table types, a frame-backed mapper, and the
//! routine that builds and activates the kernel's own address space with
//! per-section W^X permissions.
//!
//! The bootstrap shim (`boot.s`) brings the CPU up on a throwaway identity map
//! of the low 1 GiB using 2 MiB pages, which leaves the whole kernel image
//! writable *and* executable. [`init_kernel_address_space`] replaces it with a
//! freshly built hierarchy in which `.text` is R-X, `.rodata` is R--, and
//! `.data`/`.bss` (and all other low memory) are RW and never executable.
//!
//! While the kernel runs identity-mapped (virtual == physical for low memory),
//! every page-table frame handed out by [`FrameAllocator`] is itself reachable
//! at its physical address, so the mapper edits tables directly. Higher-half
//! relocation is deferred to a later ROADMAP step.

use crate::mmu::{FrameAllocator, FRAME_SIZE};
use core::ptr::addr_of;

/// Number of entries in every x86_64 page table.
pub const ENTRY_COUNT: usize = 512;
/// Bytes mapped by a level-2 (PD) huge page: 2 MiB.
pub const HUGE_PAGE_SIZE: u64 = 2 * 1024 * 1024;
/// Size of the identity-mapped physical window the kernel space provides (1 GiB).
pub const IDENTITY_LIMIT: u64 = 1024 * 1024 * 1024;

/// Page is present.
pub const PRESENT: u64 = 1 << 0;
/// Writes are permitted.
pub const WRITABLE: u64 = 1 << 1;
/// Accessible from ring 3.
pub const USER: u64 = 1 << 2;
/// Maps a large page at this level (2 MiB at the PD).
pub const HUGE: u64 = 1 << 7;
/// Global page: not flushed on a CR3 reload.
pub const GLOBAL: u64 = 1 << 8;
/// Execution disabled (requires `EFER.NXE`).
pub const NO_EXECUTE: u64 = 1 << 63;
/// Page-level write-through caching.
pub const WRITE_THROUGH: u64 = 1 << 3;
/// Page-level cache disable (used for memory-mapped device pages such as MMIO).
pub const CACHE_DISABLE: u64 = 1 << 4;

/// Mask selecting the physical-address field of a page-table entry (bits 12..=51).
const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Round `value` up to the next multiple of `align` (a power of two).
const fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

const fn pml4_index(va: u64) -> usize {
    ((va >> 39) & 0x1ff) as usize
}
const fn pdpt_index(va: u64) -> usize {
    ((va >> 30) & 0x1ff) as usize
}
const fn pd_index(va: u64) -> usize {
    ((va >> 21) & 0x1ff) as usize
}
const fn pt_index(va: u64) -> usize {
    ((va >> 12) & 0x1ff) as usize
}

/// A single 4 KiB page table: 512 64-bit entries.
#[repr(C, align(4096))]
struct PageTable {
    entries: [u64; ENTRY_COUNT],
}

impl PageTable {
    /// Reinterpret the identity-mapped frame at physical address `pa` as a table.
    ///
    /// # Safety
    /// `pa` must be a 4 KiB-aligned, identity-mapped, writable frame that is
    /// not aliased mutably elsewhere for the duration of `'a`.
    unsafe fn from_phys<'a>(pa: u64) -> &'a mut PageTable {
        // SAFETY: callers guarantee `pa` is a live, aligned, identity-mapped frame.
        unsafe { &mut *(pa as *mut PageTable) }
    }
}

/// A 4-level page-table mapper rooted at a physical PML4 frame.
///
/// All target and table frames must lie inside the identity-mapped window so
/// the mapper can reach them by physical address.
pub struct Mapper {
    pml4: u64,
}

impl Mapper {
    /// Allocate and zero a fresh PML4, yielding an empty address space.
    pub fn new(alloc: &mut FrameAllocator) -> Option<Mapper> {
        Some(Mapper { pml4: alloc_zeroed(alloc)? })
    }

    /// Physical address of the root PML4; load it into CR3 to activate.
    pub fn root(&self) -> u64 {
        self.pml4
    }

    /// Reconstruct a mapper over an already-active PML4 (e.g. the live CR3).
    ///
    /// # Safety
    /// `pml4` must be a valid PML4 whose tables are all reachable at their
    /// identity-mapped physical addresses.
    pub unsafe fn from_root(pml4: u64) -> Mapper {
        Mapper { pml4 }
    }

    /// Return the next-level table referenced by `table[index]`, creating and
    /// linking a zeroed one (present + writable, execute-permitting) if absent.
    fn ensure_table(
        table: &mut PageTable,
        index: usize,
        alloc: &mut FrameAllocator,
    ) -> Option<u64> {
        let entry = table.entries[index];
        if entry & PRESENT != 0 {
            return Some(entry & ADDR_MASK);
        }
        let frame = alloc_zeroed(alloc)?;
        // Intermediate entries stay permissive; the leaf entry decides the
        // effective permissions (the CPU ANDs every level).
        table.entries[index] = (frame & ADDR_MASK) | PRESENT | WRITABLE;
        Some(frame)
    }

    /// Map one 4 KiB page `va -> pa` with `leaf_flags` (PRESENT is implied).
    ///
    /// Refuses any leaf that is simultaneously writable and executable, so the
    /// W^X invariant cannot be broken through this API.
    pub fn map_4k(
        &mut self,
        va: u64,
        pa: u64,
        leaf_flags: u64,
        alloc: &mut FrameAllocator,
    ) -> bool {
        if leaf_flags & WRITABLE != 0 && leaf_flags & NO_EXECUTE == 0 {
            return false;
        }
        // SAFETY: every table frame is identity-mapped and uniquely owned by
        // this mapper while we hold `&mut self`.
        unsafe {
            let pml4 = PageTable::from_phys(self.pml4);
            let pdpt = match Self::ensure_table(pml4, pml4_index(va), alloc) {
                Some(p) => PageTable::from_phys(p),
                None => return false,
            };
            let pd = match Self::ensure_table(pdpt, pdpt_index(va), alloc) {
                Some(p) => PageTable::from_phys(p),
                None => return false,
            };
            let pt = match Self::ensure_table(pd, pd_index(va), alloc) {
                Some(p) => PageTable::from_phys(p),
                None => return false,
            };
            pt.entries[pt_index(va)] = (pa & ADDR_MASK) | leaf_flags | PRESENT;
        }
        true
    }

    /// Map one 2 MiB huge page `va -> pa` at the PD level.
    ///
    /// Both addresses must be 2 MiB-aligned. Like [`map_4k`](Self::map_4k) it
    /// refuses writable-and-executable leaves.
    pub fn map_2m(
        &mut self,
        va: u64,
        pa: u64,
        leaf_flags: u64,
        alloc: &mut FrameAllocator,
    ) -> bool {
        if leaf_flags & WRITABLE != 0 && leaf_flags & NO_EXECUTE == 0 {
            return false;
        }
        if va & (HUGE_PAGE_SIZE - 1) != 0 || pa & (HUGE_PAGE_SIZE - 1) != 0 {
            return false;
        }
        // SAFETY: see `map_4k`.
        unsafe {
            let pml4 = PageTable::from_phys(self.pml4);
            let pdpt = match Self::ensure_table(pml4, pml4_index(va), alloc) {
                Some(p) => PageTable::from_phys(p),
                None => return false,
            };
            let pd = match Self::ensure_table(pdpt, pdpt_index(va), alloc) {
                Some(p) => PageTable::from_phys(p),
                None => return false,
            };
            pd.entries[pd_index(va)] = (pa & ADDR_MASK) | leaf_flags | PRESENT | HUGE;
        }
        true
    }
}

/// Build leaf flags for a 4 KiB page with the given permissions.
///
/// `writable` adds `WRITABLE`; a non-executable page gets `NO_EXECUTE`. A
/// writable-and-executable request yields flags that [`Mapper::map_4k`]
/// rejects, preserving W^X.
const fn leaf_flags(writable: bool, executable: bool) -> u64 {
    let mut flags = PRESENT;
    if writable {
        flags |= WRITABLE;
    }
    if !executable {
        flags |= NO_EXECUTE;
    }
    flags
}

/// Invalidate the TLB entry for the 4 KiB page containing `va`.
///
/// # Safety
/// Issues `invlpg` against the active address space; it only ever drops a
/// cached translation, forcing a re-walk on the next access.
unsafe fn flush_tlb_4k(va: u64) {
    // SAFETY: TLB invalidation is sound; see the function contract.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
    }
}

/// Follow a present, non-huge table entry to the next-level frame, if any.
fn walk(table: &PageTable, index: usize) -> Option<u64> {
    let entry = table.entries[index];
    if entry & PRESENT != 0 && entry & HUGE == 0 {
        Some(entry & ADDR_MASK)
    } else {
        None
    }
}

/// Map one 4 KiB page `va -> pa` with the given permissions into `mapper`.
///
/// Permissions are translated to leaf flags; a writable-and-executable request
/// is refused to uphold W^X. Returns `false` if refused or a table frame could
/// not be allocated.
#[must_use]
pub fn map_page(
    mapper: &mut Mapper,
    va: u64,
    pa: u64,
    writable: bool,
    executable: bool,
    alloc: &mut FrameAllocator,
) -> bool {
    mapper.map_4k(va, pa, leaf_flags(writable, executable), alloc)
}

/// Remove the 4 KiB mapping at `va`, flushing its TLB entry.
///
/// Returns `true` if a present leaf was cleared, `false` otherwise.
#[must_use]
pub fn unmap_page(mapper: &mut Mapper, va: u64) -> bool {
    // SAFETY: every table frame is identity-mapped and uniquely owned by
    // `mapper` while we hold `&mut`.
    let cleared = unsafe {
        let pml4 = PageTable::from_phys(mapper.root());
        let Some(pdpt_f) = walk(pml4, pml4_index(va)) else {
            return false;
        };
        let Some(pd_f) = walk(PageTable::from_phys(pdpt_f), pdpt_index(va)) else {
            return false;
        };
        let Some(pt_f) = walk(PageTable::from_phys(pd_f), pd_index(va)) else {
            return false;
        };
        let pt = PageTable::from_phys(pt_f);
        if pt.entries[pt_index(va)] & PRESENT == 0 {
            false
        } else {
            pt.entries[pt_index(va)] = 0;
            true
        }
    };
    if cleared {
        // SAFETY: drop any stale translation for the now-unmapped page.
        unsafe { flush_tlb_4k(va) }
    }
    cleared
}

/// Resolve the 4 KiB mapping at `va` to `(pa, writable, executable)`.
///
/// Returns `None` if no present 4 KiB leaf maps `va` (including when a 2 MiB
/// huge page covers it).
#[must_use]
pub fn query_page(mapper: &Mapper, va: u64) -> Option<(u64, bool, bool)> {
    // SAFETY: table frames are identity-mapped and only read here.
    unsafe {
        let pml4 = PageTable::from_phys(mapper.root());
        let pdpt = PageTable::from_phys(walk(pml4, pml4_index(va))?);
        let pd = PageTable::from_phys(walk(pdpt, pdpt_index(va))?);
        let pt = PageTable::from_phys(walk(pd, pd_index(va))?);
        let e = pt.entries[pt_index(va)];
        if e & PRESENT == 0 {
            return None;
        }
        Some((e & ADDR_MASK, e & WRITABLE != 0, e & NO_EXECUTE == 0))
    }
}

/// Allocate a frame and zero it (valid because frames are identity-mapped).
fn alloc_zeroed(alloc: &mut FrameAllocator) -> Option<u64> {
    let frame = alloc.alloc()?;
    // SAFETY: `frame` is a fresh, page-aligned, identity-mapped physical frame
    // that nothing else references yet.
    unsafe {
        core::ptr::write_bytes(frame as *mut u8, 0, FRAME_SIZE as usize);
    }
    Some(frame)
}

/// Identity-map a single device MMIO page into the live kernel address space.
///
/// Maps `phys` (rounded down to a 4 KiB boundary) as uncacheable, writable and
/// non-executable at the same virtual address, then flushes its TLB entry.
/// Returns the virtual address (equal to the page base) on success. Used for
/// memory-mapped device registers, such as the local APIC, that live above the
/// identity-mapped RAM window.
pub fn map_mmio_page(kernel_pml4: u64, phys: u64, alloc: &mut FrameAllocator) -> Option<u64> {
    let page = phys & !(FRAME_SIZE - 1);
    // SAFETY: `kernel_pml4` is the live kernel PML4 from
    // `init_kernel_address_space`; all of its tables are identity-mapped.
    let mut mapper = unsafe { Mapper::from_root(kernel_pml4) };
    let flags = PRESENT | WRITABLE | NO_EXECUTE | CACHE_DISABLE | WRITE_THROUGH;
    if !mapper.map_4k(page, page, flags, alloc) {
        return None;
    }
    // SAFETY: invalidate any stale TLB entry for the freshly mapped device page.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) page, options(nostack, preserves_flags));
    }
    Some(page)
}

extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __kernel_end: u8;
}

/// Page-aligned kernel section boundaries, as emitted by `boot/linker.ld`.
struct KernelLayout {
    text_start: u64,
    text_end: u64,
    rodata_start: u64,
    rodata_end: u64,
    data_start: u64,
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
        data_start: addr_of!(__data_start) as u64,
        kernel_end: addr_of!(__kernel_end) as u64,
    }
}

/// Leaf flags for an identity-mapped low-memory page at `va`.
fn page_flags(layout: &KernelLayout, va: u64) -> u64 {
    if va >= layout.text_start && va < layout.text_end {
        // Code (and the leading PVH note): readable + executable, never writable.
        PRESENT
    } else if va >= layout.rodata_start && va < layout.rodata_end {
        // Constants: read-only, no execute.
        PRESENT | NO_EXECUTE
    } else {
        // .data/.bss plus legacy low-memory padding: writable, never executable.
        PRESENT | WRITABLE | NO_EXECUTE
    }
}

/// Build a fresh kernel address space with per-section W^X and activate it.
///
/// Identity-maps the low [`IDENTITY_LIMIT`] of physical memory. The 2 MiB
/// span(s) covering the kernel image are mapped with 4 KiB pages so each
/// section receives exact permissions; the remainder uses 2 MiB huge pages
/// (RW, NX). The null page (`0..4 KiB`) is deliberately left unmapped as a
/// guard. Returns the physical PML4 that was loaded into CR3, or `None` if a
/// backing frame could not be obtained.
///
/// # Safety considerations
/// On success the new tables map the running code, the active stack, the GDT/
/// IDT and the page tables themselves; `EFER.NXE` is enabled before any NX bit
/// becomes live and `CR0.WP` before the switch.
pub fn init_kernel_address_space(alloc: &mut FrameAllocator) -> Option<u64> {
    let layout = kernel_layout();
    let mut mapper = Mapper::new(alloc)?;

    // Fine-grained (4 KiB) span: from the first page up to the 2 MiB boundary
    // above the kernel image, so per-section permissions are expressible.
    let fine_top = align_up(layout.kernel_end, HUGE_PAGE_SIZE);
    let mut va = FRAME_SIZE; // null page left unmapped as a guard
    while va < fine_top {
        if !mapper.map_4k(va, va, page_flags(&layout, va), alloc) {
            return None;
        }
        va += FRAME_SIZE;
    }

    // Coarse (2 MiB) identity map for the rest of the window: RW, NX.
    let mut va = fine_top;
    while va < IDENTITY_LIMIT {
        if !mapper.map_2m(va, va, WRITABLE | NO_EXECUTE, alloc) {
            return None;
        }
        va += HUGE_PAGE_SIZE;
    }

    // SAFETY: the freshly built hierarchy covers everything the CPU touches
    // after the switch; NXE/WP are configured inside `activate`.
    unsafe {
        activate(mapper.root());
    }
    Some(mapper.root())
}

/// Enable `EFER.NXE` and `CR0.WP`, then load `pml4_phys` into CR3.
///
/// # Safety
/// `pml4_phys` must reference a valid PML4 that maps the currently executing
/// instructions, the active stack and the page tables themselves; otherwise
/// the CR3 load faults immediately.
unsafe fn activate(pml4_phys: u64) {
    use core::arch::asm;

    // SAFETY: the caller guarantees `pml4_phys` maps the running code, the
    // active stack and the page tables themselves; NXE is set before any NX
    // entry goes live and WP before the CR3 load.
    unsafe {
        // EFER.NXE (bit 11) must be set before any live entry carries the NX bit.
        const IA32_EFER: u32 = 0xC000_0080;
        let mut lo: u32;
        let hi: u32;
        asm!("rdmsr", in("ecx") IA32_EFER, out("eax") lo, out("edx") hi,
             options(nostack, preserves_flags));
        lo |= 1 << 11;
        asm!("wrmsr", in("ecx") IA32_EFER, in("eax") lo, in("edx") hi,
             options(nostack, preserves_flags));

        // CR0.WP (bit 16): ring-0 writes honour read-only pages.
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
        cr0 |= 1 << 16;
        asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));

        // Switch address space; this flushes the non-global TLB.
        asm!("mov cr3, {}", in(reg) pml4_phys, options(nostack, preserves_flags));
    }
}
