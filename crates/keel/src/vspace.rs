// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 24 june 2026

//! Virtual address spaces as cap-mediated page maps.
//!
//! A [`VSpace`] is the keel-level model of a task's address space: a set of
//! virtual pages, each backed by a Page [`Capability`]. `map` installs a page
//! (checking type, alignment and rights), `unmap` removes it and hands the
//! capability back, and `translate` resolves a virtual address to a physical
//! one plus the rights that govern the access.
//!
//! [`VSpace`] itself models the *bookkeeping* only; storage is caller-owned
//! (`&mut [Mapping]`), so there is no kernel heap and the mapping budget is
//! explicit. [`HwVSpace`] pairs that bookkeeping with `hull`'s architecture
//! `Mapper` so `map`/`unmap` additionally write the real hardware translation
//! tables (4 KiB leaves, W^X enforced) and can be cross-checked against them.
//! An `HwVSpace` is never installed into TTBR0/CR3 by this module, so it can be
//! built and torn down without disturbing the running kernel. Folding the page
//! capability into the CDT so `revoke` tears mappings down is still future work.

use crate::cap::{CapType, Capability, Rights};
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use hull::{
    mmu::FrameAllocator,
    paging::{self, Mapper},
};

/// Page size in address bits (4 KiB), matching `hull`'s frame size.
pub const PAGE_BITS: u32 = 12;
/// Page size in bytes.
pub const PAGE_SIZE: u64 = 1 << PAGE_BITS;

/// One virtual-page mapping: a page-aligned VA backed by a Page capability.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mapping {
    va: u64,
    page: Capability,
    used: bool,
}

impl Mapping {
    /// An empty, reusable mapping slot. Use to initialise backing storage:
    /// `let mut maps = [Mapping::EMPTY; 32];`
    pub const EMPTY: Self = Self {
        va: 0,
        page: Capability::NULL,
        used: false,
    };

    /// The virtual address this mapping covers.
    #[must_use]
    pub const fn va(self) -> u64 {
        self.va
    }

    /// The page capability backing this mapping.
    #[must_use]
    pub const fn page(self) -> Capability {
        self.page
    }
}

/// Why a virtual-memory operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VSpaceError {
    /// The capability is not a Page capability.
    NotAPage,
    /// The virtual address is not page-aligned.
    Misaligned,
    /// The page capability does not even grant read access.
    NoRights,
    /// A mapping already covers this virtual address.
    AlreadyMapped,
    /// No mapping covers this virtual address.
    NotMapped,
    /// No free mapping slot remains in the backing storage.
    Full,
    /// A backing hardware page-table operation failed (e.g. a W^X violation or
    /// no free frame for an intermediate table).
    HardwareFault,
}

/// A virtual address space: a flat set of page mappings over caller storage.
pub struct VSpace<'maps> {
    maps: &'maps mut [Mapping],
}

impl<'maps> VSpace<'maps> {
    /// Wrap a slice of mapping storage, clearing every slot to empty.
    #[must_use]
    pub fn new(maps: &'maps mut [Mapping]) -> Self {
        for m in maps.iter_mut() {
            *m = Mapping::EMPTY;
        }
        Self { maps }
    }

    /// Number of mappings this address space can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.maps.len()
    }

    /// Number of live mappings.
    #[must_use]
    pub fn count(&self) -> usize {
        self.maps.iter().filter(|m| m.used).count()
    }

    /// Index of the live mapping covering page-aligned `va`, if any.
    fn find(&self, va: u64) -> Option<usize> {
        self.maps.iter().position(|m| m.used && m.va == va)
    }

    /// Map a Page capability at page-aligned virtual address `va`.
    ///
    /// # Errors
    /// - [`VSpaceError::NotAPage`] if `page` is not a Page capability.
    /// - [`VSpaceError::Misaligned`] if `va` is not page-aligned.
    /// - [`VSpaceError::NoRights`] if `page` lacks read access.
    /// - [`VSpaceError::AlreadyMapped`] if `va` is already mapped.
    /// - [`VSpaceError::Full`] if no free mapping slot remains.
    pub fn map(&mut self, va: u64, page: Capability) -> Result<(), VSpaceError> {
        self.check_mappable(va, page)?;
        let idx = self
            .maps
            .iter()
            .position(|m| !m.used)
            .ok_or(VSpaceError::Full)?;
        self.maps[idx] = Mapping {
            va,
            page,
            used: true,
        };
        Ok(())
    }

    /// Validate that `page` may be mapped at `va` *without* recording it.
    ///
    /// Runs the same guard rails as [`map`](Self::map) — capability type,
    /// alignment, read access, no duplicate, and a free slot — so a caller that
    /// also drives hardware tables can check before committing either side.
    ///
    /// # Errors
    /// The same errors as [`map`](Self::map).
    pub fn check_mappable(&self, va: u64, page: Capability) -> Result<(), VSpaceError> {
        if page.cap_type() != CapType::Page {
            return Err(VSpaceError::NotAPage);
        }
        if va & (PAGE_SIZE - 1) != 0 {
            return Err(VSpaceError::Misaligned);
        }
        if !page.rights().contains(Rights::READ) {
            return Err(VSpaceError::NoRights);
        }
        if self.find(va).is_some() {
            return Err(VSpaceError::AlreadyMapped);
        }
        if !self.maps.iter().any(|m| !m.used) {
            return Err(VSpaceError::Full);
        }
        Ok(())
    }

    /// Remove the mapping at page-aligned `va` and return its page capability.
    ///
    /// # Errors
    /// Returns [`VSpaceError::NotMapped`] if nothing is mapped at `va`.
    pub fn unmap(&mut self, va: u64) -> Result<Capability, VSpaceError> {
        let idx = self.find(va).ok_or(VSpaceError::NotMapped)?;
        let cap = self.maps[idx].page;
        self.maps[idx] = Mapping::EMPTY;
        Ok(cap)
    }

    /// Resolve a virtual address (any offset within a mapped page) to its
    /// physical address and the governing rights.
    ///
    /// # Errors
    /// Returns [`VSpaceError::NotMapped`] if the containing page is not mapped.
    pub fn translate(&self, va: u64) -> Result<(u64, Rights), VSpaceError> {
        let base = va & !(PAGE_SIZE - 1);
        let offset = va & (PAGE_SIZE - 1);
        let idx = self.find(base).ok_or(VSpaceError::NotMapped)?;
        let page = self.maps[idx].page;
        Ok((page.object() + offset, page.rights()))
    }
}

/// Boot-time self-test exercising map/translate/unmap and the guard rails.
///
/// # Errors
/// Returns a [`VSpaceError`] if any invariant fails.
pub fn selftest() -> Result<(), VSpaceError> {
    let mut storage = [Mapping::EMPTY; 4];
    let mut vs = VSpace::new(&mut storage);
    let rw = Rights::READ.union(Rights::WRITE);
    let page = |pa| Capability::new(CapType::Page, pa, u64::from(PAGE_BITS), rw);

    // Guard rails: misaligned VA, non-page cap, and a rights-less page.
    if vs.map(0x1001, page(0x4020_0000)) != Err(VSpaceError::Misaligned) {
        return Err(VSpaceError::Misaligned);
    }
    let untyped = Capability::new(CapType::Untyped, 0x4020_0000, 21, Rights::ALL);
    if vs.map(0x1000, untyped) != Err(VSpaceError::NotAPage) {
        return Err(VSpaceError::NotAPage);
    }
    let no_rights = Capability::new(CapType::Page, 0x4020_0000, u64::from(PAGE_BITS), Rights::NONE);
    if vs.map(0x1000, no_rights) != Err(VSpaceError::NoRights) {
        return Err(VSpaceError::NoRights);
    }

    // Map two pages and confirm the count.
    vs.map(0x1000, page(0x4020_0000))?;
    vs.map(0x2000, page(0x4020_1000))?;
    if vs.count() != 2 {
        return Err(VSpaceError::Full);
    }

    // A second mapping at the same VA is refused.
    if vs.map(0x1000, page(0x4020_2000)) != Err(VSpaceError::AlreadyMapped) {
        return Err(VSpaceError::AlreadyMapped);
    }

    // Translation resolves the intra-page offset and reports the rights.
    let (pa, rights) = vs.translate(0x1000 + 0x40)?;
    if pa != 0x4020_0000 + 0x40 || rights != rw {
        return Err(VSpaceError::NotMapped);
    }

    // Unmap returns the original page cap; a second unmap fails.
    let cap = vs.unmap(0x1000)?;
    if cap != page(0x4020_0000) {
        return Err(VSpaceError::NotMapped);
    }
    if vs.unmap(0x1000) != Err(VSpaceError::NotMapped) {
        return Err(VSpaceError::NotMapped);
    }
    if vs.count() != 1 {
        return Err(VSpaceError::Full);
    }

    Ok(())
}

/// A virtual address space whose `map`/`unmap` drive a live `hull` page-table
/// [`Mapper`] in addition to the keel-level bookkeeping.
///
/// The bookkeeping half ([`VSpace`]) enforces the capability invariants and
/// records which page backs each VA; the `Mapper` half writes the actual
/// hardware translation tables. The mapper is *not* installed into TTBR0/CR3 by
/// this type, so a fresh `HwVSpace` describes an inactive address space that can
/// be built and torn down without disturbing the running kernel.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub struct HwVSpace<'maps> {
    book: VSpace<'maps>,
    mapper: Mapper,
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
impl<'maps> HwVSpace<'maps> {
    /// Wrap mapping storage and allocate a fresh, empty hardware root table.
    ///
    /// Returns `None` if a backing frame for the root table is unavailable.
    pub fn new(maps: &'maps mut [Mapping], alloc: &mut FrameAllocator) -> Option<Self> {
        let mapper = Mapper::new(alloc)?;
        Some(Self {
            book: VSpace::new(maps),
            mapper,
        })
    }

    /// Physical address of the root translation table (the TTBR0/CR3 value).
    #[must_use]
    pub fn root(&self) -> u64 {
        self.mapper.root()
    }

    /// Number of live mappings (delegates to the bookkeeping half).
    #[must_use]
    pub fn count(&self) -> usize {
        self.book.count()
    }

    /// Map `page` at page-aligned `va`, writing both the bookkeeping entry and
    /// the hardware translation tables.
    ///
    /// Rights drive the leaf permissions: WRITE makes the page writable and
    /// EXECUTE makes it executable. A page that is both writable and executable
    /// is refused by `hull` to uphold W^X.
    ///
    /// # Errors
    /// The [`VSpace::map`] guard-rail errors, plus
    /// [`VSpaceError::HardwareFault`] if the hardware mapping could not be
    /// installed. On a hardware fault no bookkeeping entry is recorded.
    pub fn map(
        &mut self,
        va: u64,
        page: Capability,
        alloc: &mut FrameAllocator,
    ) -> Result<(), VSpaceError> {
        // Validate against the bookkeeping rules first, but do not record yet.
        self.book.check_mappable(va, page)?;
        let rights = page.rights();
        let writable = rights.contains(Rights::WRITE);
        let executable = rights.contains(Rights::EXECUTE);
        if !paging::map_page(&mut self.mapper, va, page.object(), writable, executable, alloc) {
            return Err(VSpaceError::HardwareFault);
        }
        // The hardware leaf is in place; record the bookkeeping entry. This
        // cannot fail because `check_mappable` already proved a free slot.
        self.book.map(va, page)
    }

    /// Unmap `va`, clearing the hardware leaf and returning the page cap.
    ///
    /// # Errors
    /// [`VSpaceError::NotMapped`] if nothing is mapped at `va`, or
    /// [`VSpaceError::HardwareFault`] if the hardware leaf was unexpectedly
    /// absent.
    pub fn unmap(&mut self, va: u64) -> Result<Capability, VSpaceError> {
        // Confirm a bookkeeping entry exists (also validates the page address).
        if self.book.translate(va).is_err() {
            return Err(VSpaceError::NotMapped);
        }
        if !paging::unmap_page(&mut self.mapper, va) {
            return Err(VSpaceError::HardwareFault);
        }
        self.book.unmap(va)
    }

    /// Resolve `va` through the hardware tables to `(pa, writable, executable)`.
    #[must_use]
    pub fn query(&self, va: u64) -> Option<(u64, bool, bool)> {
        paging::query_page(&self.mapper, va)
    }
}

/// Boot-time self-test that drives the `hull` page-table `Mapper` through
/// [`HwVSpace`]: it builds an inactive address space, maps a page, verifies the
/// hardware leaf, proves W^X is enforced, then unmaps and confirms the leaf is
/// gone. The address space is never installed, so the running kernel is
/// undisturbed; the handful of frames it consumes are not reclaimed.
///
/// # Errors
/// Returns a [`VSpaceError`] if any invariant fails.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn hw_selftest(frames: &mut FrameAllocator) -> Result<(), VSpaceError> {
    let mut storage = [Mapping::EMPTY; 4];
    let mut vs = HwVSpace::new(&mut storage, frames).ok_or(VSpaceError::HardwareFault)?;

    // A high, canonical VA well clear of the identity-mapped kernel window.
    const VA: u64 = 0x10_0000_0000;
    let pa = frames.alloc().ok_or(VSpaceError::HardwareFault)?;
    let rw = Rights::READ.union(Rights::WRITE);
    let rwx = rw.union(Rights::EXECUTE);

    // Map RW and confirm the hardware leaf reports the frame, writable + NX.
    vs.map(
        VA,
        Capability::new(CapType::Page, pa, u64::from(PAGE_BITS), rw),
        frames,
    )?;
    match vs.query(VA) {
        Some((leaf_pa, true, false)) if leaf_pa == pa => {}
        _ => return Err(VSpaceError::HardwareFault),
    }
    if vs.count() != 1 {
        return Err(VSpaceError::Full);
    }

    // W^X: a writable+executable page is refused by the hardware backend and
    // leaves no bookkeeping entry behind.
    let wx = Capability::new(CapType::Page, pa, u64::from(PAGE_BITS), rwx);
    if vs.map(VA + PAGE_SIZE, wx, frames) != Err(VSpaceError::HardwareFault) {
        return Err(VSpaceError::HardwareFault);
    }
    if vs.count() != 1 {
        return Err(VSpaceError::Full);
    }

    // Unmap clears the hardware leaf and returns the original capability.
    let cap = vs.unmap(VA)?;
    if cap.object() != pa {
        return Err(VSpaceError::NotMapped);
    }
    if vs.query(VA).is_some() {
        return Err(VSpaceError::HardwareFault);
    }
    if vs.unmap(VA) != Err(VSpaceError::NotMapped) {
        return Err(VSpaceError::NotMapped);
    }

    Ok(())
}
