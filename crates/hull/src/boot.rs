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

    /// An empty, invalid memory map for boots without a PVH hand-off.
    ///
    /// Used by the aarch64 entry path until the flattened device tree is
    /// parsed: Keel still brings up the console and parks, it just sees no
    /// usable RAM yet.
    pub const fn none() -> Self {
        Self::empty()
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
            kprintln!("hull: no boot memory map (no PVH start_info / DTB); memory map unknown");
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

// ----------------------------------------------------------------------------
// Flattened Device Tree (DTB) memory-map parsing — aarch64 (QEMU `virt`).
// ----------------------------------------------------------------------------
//
// QEMU `virt` hands the kernel the physical address of a flattened device tree
// in `x0`. There is no heap and no libfdt, so we walk the FDT structure block
// directly with a hand-rolled, bounds-checked reader and pull the `/memory`
// node(s) into the same heap-free `BootInfo` the frame allocator consumes.
//
// Reference: Devicetree Specification v0.4, §5 (Flattened Devicetree format).

/// FDT header magic (`0xd00dfeed`, stored big-endian on disk).
const FDT_MAGIC: u32 = 0xd00d_feed;

// FDT structure-block tokens.
const FDT_BEGIN_NODE: u32 = 0x0000_0001;
const FDT_END_NODE: u32 = 0x0000_0002;
const FDT_PROP: u32 = 0x0000_0003;
const FDT_NOP: u32 = 0x0000_0004;
const FDT_END: u32 = 0x0000_0009;

/// Read a big-endian `u32` from `p` byte-by-byte.
///
/// Alignment-safe by construction: the MMU is off during early aarch64 boot, so
/// a wide unaligned load could fault, but single-byte reads never do.
///
/// # Safety
/// `p` must point at four readable bytes in the identity-mapped DTB.
unsafe fn fdt_be_u32(p: *const u8) -> u32 {
    // SAFETY: caller guarantees four readable bytes at `p`.
    unsafe {
        ((*p as u32) << 24)
            | ((*p.add(1) as u32) << 16)
            | ((*p.add(2) as u32) << 8)
            | (*p.add(3) as u32)
    }
}

/// Round `n` up to the next multiple of four (FDT tokens are 4-byte aligned).
const fn fdt_align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Length of the NUL-terminated C string at `p`, scanning at most `max` bytes.
///
/// # Safety
/// `p` must point at up to `max` readable bytes.
unsafe fn fdt_cstr_len(p: *const u8, max: usize) -> usize {
    let mut n = 0;
    // SAFETY: bounded by `max`; caller guarantees readability.
    while n < max && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

/// Whether the property name at `strings + nameoff` equals `target` exactly.
///
/// # Safety
/// `strings` must point at the DTB strings block and `nameoff` lie within it.
unsafe fn fdt_name_eq(strings: *const u8, nameoff: usize, target: &[u8]) -> bool {
    // SAFETY: caller guarantees `strings + nameoff` is a readable C string.
    let p = unsafe { strings.add(nameoff) };
    let mut i = 0;
    while i < target.len() {
        // SAFETY: bounded read within the string; terminated by the NUL check.
        if unsafe { *p.add(i) } != target[i] {
            return false;
        }
        i += 1;
    }
    // Exact match: the next byte must be the terminating NUL.
    // SAFETY: one byte past the compared prefix is still within the block.
    unsafe { *p.add(target.len()) == 0 }
}

/// Whether a node name is `memory` or `memory@<unit-address>`.
///
/// # Safety
/// `p` must point at `len` readable bytes.
unsafe fn fdt_name_is_memory(p: *const u8, len: usize) -> bool {
    const PREFIX: &[u8] = b"memory";
    if len < PREFIX.len() {
        return false;
    }
    let mut i = 0;
    while i < PREFIX.len() {
        // SAFETY: `i < PREFIX.len() <= len`, caller guarantees readability.
        if unsafe { *p.add(i) } != PREFIX[i] {
            return false;
        }
        i += 1;
    }
    // SAFETY: byte at PREFIX.len() is in-bounds when len > PREFIX.len().
    len == PREFIX.len() || unsafe { *p.add(PREFIX.len()) } == b'@'
}

/// Whether the property value `[val, val+len)` is the C string `target`.
///
/// # Safety
/// `val` must point at `len` readable bytes.
unsafe fn fdt_val_is(val: *const u8, len: usize, target: &[u8]) -> bool {
    if len < target.len() {
        return false;
    }
    let mut i = 0;
    while i < target.len() {
        // SAFETY: `i < target.len() <= len`, caller guarantees readability.
        if unsafe { *val.add(i) } != target[i] {
            return false;
        }
        i += 1;
    }
    // Allow an exact-length match or a single trailing NUL.
    len == target.len() || unsafe { *val.add(target.len()) == 0 }
}

/// Read `cells` big-endian 32-bit cells at `p` into a `u64`.
///
/// FDT addresses/sizes are 1–2 cells on the platforms we target; if a producer
/// ever uses more, we keep the low 64 bits (the most significant cells of a
/// >64-bit value are zero for all real RAM on QEMU `virt`).
///
/// # Safety
/// `p` must point at `cells * 4` readable bytes.
unsafe fn fdt_read_cells(p: *const u8, cells: u32) -> u64 {
    let mut value: u64 = 0;
    let mut i = 0;
    while i < cells {
        // SAFETY: caller guarantees `cells * 4` readable bytes.
        let word = unsafe { fdt_be_u32(p.add(i as usize * 4)) } as u64;
        value = (value << 32) | word;
        i += 1;
    }
    value
}

/// Parse the flattened device tree at `dtb_paddr` into a [`BootInfo`].
///
/// Walks the FDT structure block, reads the root `#address-cells` /
/// `#size-cells`, and records every `/memory` node's `reg` ranges as usable
/// RAM. Returns an empty, invalid map if the pointer is null or the FDT magic
/// does not match, so a missing device tree degrades gracefully instead of
/// faulting.
///
/// # Safety
/// `dtb_paddr` must be `0` or the physical address of a flattened device tree
/// reachable through the current identity map (MMU off / flat map). The magic
/// is validated before any other field is trusted.
pub unsafe fn parse_dtb(dtb_paddr: u64) -> BootInfo {
    let mut info = BootInfo::empty();
    if dtb_paddr == 0 {
        return info;
    }

    let base = dtb_paddr as *const u8;
    // SAFETY: header bytes are read only after the null check; the magic is
    // validated before any offset is trusted.
    let magic = unsafe { fdt_be_u32(base) };
    if magic != FDT_MAGIC {
        return info;
    }
    // SAFETY: a valid magic implies a well-formed header per the FDT spec.
    let totalsize = unsafe { fdt_be_u32(base.add(4)) } as usize;
    let off_struct = unsafe { fdt_be_u32(base.add(8)) } as usize;
    let off_strings = unsafe { fdt_be_u32(base.add(12)) } as usize;
    if totalsize < 36 || off_struct >= totalsize || off_strings >= totalsize {
        return info;
    }
    info.valid = true;

    // SAFETY: both offsets were bounded against `totalsize` above.
    let struct_ptr = unsafe { base.add(off_struct) };
    let strings_ptr = unsafe { base.add(off_strings) };
    let struct_len = totalsize - off_struct;

    // Root address/size cell counts (FDT defaults: 2 and 1) until overridden.
    let mut addr_cells: u32 = 2;
    let mut size_cells: u32 = 1;

    let mut off = 0usize;
    let mut depth: i32 = 0;

    // Per-candidate (depth-2) memory-node state.
    let mut in_mem = false;
    let mut dev_ok = false;
    let mut have_reg = false;
    let mut reg_off = 0usize;
    let mut reg_len = 0usize;

    // Bounded walk: also guarded by `struct_len`, but cap iterations to stay
    // safe on a malformed tree.
    let mut guard = 0u32;
    while off + 4 <= struct_len {
        guard += 1;
        if guard > 200_000 {
            break;
        }
        // SAFETY: `off + 4 <= struct_len`.
        let token = unsafe { fdt_be_u32(struct_ptr.add(off)) };
        off += 4;

        match token {
            FDT_BEGIN_NODE => {
                // SAFETY: the name is a NUL-terminated string within the block.
                let name_ptr = unsafe { struct_ptr.add(off) };
                let name_len = unsafe { fdt_cstr_len(name_ptr, struct_len - off) };
                off = (off + fdt_align4(name_len + 1)).min(struct_len);
                depth += 1;
                if depth == 2 {
                    // SAFETY: comparing the node name bytes in-bounds.
                    in_mem = unsafe { fdt_name_is_memory(name_ptr, name_len) };
                    dev_ok = in_mem; // refined by `device_type` if present.
                    have_reg = false;
                }
            }
            FDT_END_NODE => {
                if depth == 2 && in_mem && dev_ok && have_reg {
                    let addr_bytes = addr_cells as usize * 4;
                    let entry_bytes = addr_bytes + size_cells as usize * 4;
                    if entry_bytes > 0 {
                        let mut consumed = 0usize;
                        while consumed + entry_bytes <= reg_len {
                            if info.region_count == MAX_REGIONS {
                                info.truncated = true;
                                break;
                            }
                            // SAFETY: `reg_off + consumed + entry_bytes <= reg_len`
                            // lies inside the struct block.
                            let entry = unsafe { struct_ptr.add(reg_off + consumed) };
                            let start = unsafe { fdt_read_cells(entry, addr_cells) };
                            let size =
                                unsafe { fdt_read_cells(entry.add(addr_bytes), size_cells) };
                            if size != 0 {
                                info.regions[info.region_count] = MemoryRegion {
                                    start,
                                    size,
                                    kind: MemoryKind::Usable,
                                };
                                info.region_count += 1;
                            }
                            consumed += entry_bytes;
                        }
                    }
                }
                depth -= 1;
                if depth < 2 {
                    in_mem = false;
                }
            }
            FDT_PROP => {
                if off + 8 > struct_len {
                    break;
                }
                // SAFETY: `off + 8 <= struct_len`.
                let len = unsafe { fdt_be_u32(struct_ptr.add(off)) } as usize;
                let nameoff = unsafe { fdt_be_u32(struct_ptr.add(off + 4)) } as usize;
                off += 8;
                let val_off = off;
                off = (off + fdt_align4(len)).min(struct_len);

                if depth == 1 {
                    // SAFETY: name compared within the strings block.
                    if unsafe { fdt_name_eq(strings_ptr, nameoff, b"#address-cells") }
                        && len >= 4
                    {
                        // SAFETY: `val_off + 4 <= struct_len` since len >= 4.
                        addr_cells = unsafe { fdt_be_u32(struct_ptr.add(val_off)) };
                    } else if unsafe { fdt_name_eq(strings_ptr, nameoff, b"#size-cells") }
                        && len >= 4
                    {
                        // SAFETY: as above.
                        size_cells = unsafe { fdt_be_u32(struct_ptr.add(val_off)) };
                    }
                } else if depth == 2 && in_mem {
                    if unsafe { fdt_name_eq(strings_ptr, nameoff, b"reg") } {
                        reg_off = val_off;
                        reg_len = len.min(struct_len - val_off);
                        have_reg = true;
                    } else if unsafe { fdt_name_eq(strings_ptr, nameoff, b"device_type") } {
                        // SAFETY: value lies within the struct block.
                        dev_ok =
                            unsafe { fdt_val_is(struct_ptr.add(val_off), len, b"memory") };
                    }
                }
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => break,
        }
    }

    info
}
