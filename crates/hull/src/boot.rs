// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! PVH boot-protocol hand-off.
//!
//! The `boot` shim enters long mode with the physical address of the PVH
//! `hvm_start_info` structure (passed by the loader in `ebx`). This module
//! turns that raw pointer into a typed, heap-free physical [`BootInfo`] memory
//! map that the early frame allocator in [`crate::mmu`] consumes.
//!
//! Only the low 1 GiB is identity-mapped at this point, and QEMU/Xen place the
//! `hvm_start_info` and its memory map well within low RAM, so the structures
//! are readable directly through their physical addresses.

use crate::kprintln;

/// Magic value found at the top of a valid PVH `hvm_start_info`.
const HVM_START_MAGIC: u32 = 0x336e_c578;

// PVH / E820-style memory region types.
const MEMMAP_TYPE_RAM: u32 = 1;
const MEMMAP_TYPE_RESERVED: u32 = 2;
const MEMMAP_TYPE_ACPI: u32 = 3;
const MEMMAP_TYPE_NVS: u32 = 4;
const MEMMAP_TYPE_UNUSABLE: u32 = 5;

/// Maximum number of memory regions recorded without a heap.
pub const MAX_REGIONS: usize = 32;

/// PVH `hvm_start_info` (see Xen `arch-x86/hvm/start_info.h`).
#[repr(C)]
struct HvmStartInfo {
    magic: u32,
    version: u32,
    flags: u32,
    nr_modules: u32,
    modlist_paddr: u64,
    cmdline_paddr: u64,
    rsdp_paddr: u64,
    // Present only in version 1 and newer.
    memmap_paddr: u64,
    memmap_entries: u32,
    reserved: u32,
}

/// PVH `hvm_memmap_table_entry`.
#[repr(C)]
#[derive(Clone, Copy)]
struct HvmMemmapEntry {
    addr: u64,
    size: u64,
    kind: u32,
    reserved: u32,
}

/// Classification of a physical memory region (E820 lineage).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    /// Free RAM the kernel may allocate from.
    Usable,
    /// Firmware- or device-reserved; never allocate.
    Reserved,
    /// ACPI tables; reclaimable once parsed.
    AcpiReclaimable,
    /// ACPI non-volatile storage; must be preserved.
    AcpiNvs,
    /// Defective or explicitly unusable RAM.
    Unusable,
    /// Any other type reported by the loader, with its raw code.
    Other(u32),
}

impl MemoryKind {
    fn from_raw(raw: u32) -> Self {
        match raw {
            MEMMAP_TYPE_RAM => MemoryKind::Usable,
            MEMMAP_TYPE_RESERVED => MemoryKind::Reserved,
            MEMMAP_TYPE_ACPI => MemoryKind::AcpiReclaimable,
            MEMMAP_TYPE_NVS => MemoryKind::AcpiNvs,
            MEMMAP_TYPE_UNUSABLE => MemoryKind::Unusable,
            other => MemoryKind::Other(other),
        }
    }

    /// Short human-readable label for boot logging.
    pub fn label(self) -> &'static str {
        match self {
            MemoryKind::Usable => "usable",
            MemoryKind::Reserved => "reserved",
            MemoryKind::AcpiReclaimable => "ACPI",
            MemoryKind::AcpiNvs => "ACPI NVS",
            MemoryKind::Unusable => "unusable",
            MemoryKind::Other(_) => "other",
        }
    }

    /// Whether frames in this region may be handed to the allocator.
    pub fn is_usable(self) -> bool {
        matches!(self, MemoryKind::Usable)
    }
}

/// A single contiguous physical memory region.
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    /// Inclusive base physical address.
    pub start: u64,
    /// Region length in bytes.
    pub size: u64,
    /// How the firmware/loader classified the region.
    pub kind: MemoryKind,
}

impl MemoryRegion {
    /// Exclusive end physical address (`start + size`), saturating.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.size)
    }
}

/// Heap-free snapshot of the physical memory map plus loader validity.
#[derive(Clone, Copy)]
pub struct BootInfo {
    regions: [MemoryRegion; MAX_REGIONS],
    region_count: usize,
    truncated: bool,
    valid: bool,
}

impl BootInfo {
    const fn empty() -> Self {
        Self {
            regions: [MemoryRegion {
                start: 0,
                size: 0,
                kind: MemoryKind::Reserved,
            }; MAX_REGIONS],
            region_count: 0,
            truncated: false,
            valid: false,
        }
    }

    /// Whether a valid PVH `hvm_start_info` was found.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Number of recorded memory regions.
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// The recorded memory regions.
    pub fn regions(&self) -> &[MemoryRegion] {
        &self.regions[..self.region_count]
    }

    /// Copy of the region at `index` (panics out of range only in debug).
    pub fn region(&self, index: usize) -> MemoryRegion {
        self.regions[index]
    }

    /// Total bytes of usable RAM across all regions.
    pub fn usable_bytes(&self) -> u64 {
        let mut total = 0;
        for r in self.regions() {
            if r.kind.is_usable() {
                total += r.size;
            }
        }
        total
    }

    /// Print the memory map over the early serial console.
    pub fn log(&self) {
        if !self.valid {
            kprintln!("hull: no PVH start_info (non-PVH boot?); memory map unknown");
            return;
        }
        if self.region_count == 0 {
            kprintln!("hull: PVH start_info present but no memory map provided");
            return;
        }
        kprintln!("hull: physical memory map ({} regions):", self.region_count);
        for r in self.regions() {
            kprintln!(
                "  [{:#012x}..{:#012x}] {:>9} KiB  {}",
                r.start,
                r.end(),
                r.size / 1024,
                r.kind.label()
            );
        }
        kprintln!(
            "hull: usable RAM total ~{} MiB",
            self.usable_bytes() / (1024 * 1024)
        );
        if self.truncated {
            kprintln!(
                "hull: WARNING memory map truncated at {} regions",
                MAX_REGIONS
            );
        }
    }
}

/// Parse the PVH `hvm_start_info` at `start_info_paddr` into a [`BootInfo`].
///
/// Returns an empty, invalid map if the pointer is null or the magic does not
/// match, so a non-PVH boot degrades gracefully instead of faulting.
///
/// # Safety
/// `start_info_paddr` must be either `0` or the physical address of a PVH
/// `hvm_start_info` reachable through the current identity map. The function
/// validates the magic before trusting any further fields.
pub unsafe fn parse_pvh(start_info_paddr: u64) -> BootInfo {
    let mut info = BootInfo::empty();
    if start_info_paddr == 0 {
        return info;
    }

    // SAFETY: caller guarantees the address is mapped; we copy the header out
    // with an unaligned read and validate the magic before using any field.
    let si = unsafe { (start_info_paddr as *const HvmStartInfo).read_unaligned() };
    if si.magic != HVM_START_MAGIC {
        return info;
    }
    info.valid = true;

    // The memory map only exists from version 1 onward.
    if si.version < 1 || si.memmap_entries == 0 || si.memmap_paddr == 0 {
        return info;
    }

    let base = si.memmap_paddr as *const HvmMemmapEntry;
    for i in 0..si.memmap_entries as usize {
        if info.region_count == MAX_REGIONS {
            info.truncated = true;
            break;
        }
        // SAFETY: entries [0, memmap_entries) live in low identity-mapped RAM.
        let entry = unsafe { base.add(i).read_unaligned() };
        if entry.size == 0 {
            continue;
        }
        info.regions[info.region_count] = MemoryRegion {
            start: entry.addr,
            size: entry.size,
            kind: MemoryKind::from_raw(entry.kind),
        };
        info.region_count += 1;
    }

    info
}
