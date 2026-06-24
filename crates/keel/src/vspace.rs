// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
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
//! This slice models the *bookkeeping* only — it does not yet touch real
//! hardware page tables. Storage is caller-owned (`&mut [Mapping]`), so there
//! is no kernel heap and the mapping budget is explicit. A follow-up slice will
//! drive `hull`'s architecture `Mapper` (`map_4k`/`map_2m`) from these
//! operations so a `map` mutates the live translation regime, and will fold the
//! page capability into the CDT so `revoke` tears mappings down.

use crate::cap::{CapType, Capability, Rights};

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
