// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! Capability table entries (CTEs) and the unified capability space.
//!
//! Earlier slices kept two parallel structures: [`crate::cap::CNode`] stored
//! the capabilities, while [`crate::cdt`] tracked parent -> child derivation in
//! a *separate* node array. Keeping them in lock-step by hand is exactly the
//! kind of bookkeeping that goes wrong in a TCB.
//!
//! This module fuses them. A [`Cte`] is one CSpace slot carrying **both** the
//! [`Capability`] and its mapping-database (MDB) link to the parent it was
//! derived from — the seL4 "capability table entry". A [`CSpace`] is a flat
//! array of `Cte`s over caller-owned storage (ultimately retyped from untyped
//! memory, so there is still no kernel heap; see `docs/ARCHITECTURE.md` §1.1).
//!
//! On top of the fused slot the usual operations become *one* consistent step:
//!
//! - [`CSpace::insert_root`] installs an original (parentless) capability.
//! - [`CSpace::retype`] carves objects out of an `Untyped` slot straight into
//!   sibling slots, each recorded as derived from that untyped — the bridge
//!   from [`crate::untyped`] into real CSpace slots.
//! - [`CSpace::mint`] derives a rights-reduced child (monotonic; escalation is
//!   refused), [`CSpace::copy`] makes a same-authority sibling, and
//!   [`CSpace::move_cap`] relocates a slot while keeping the derivation tree
//!   intact (children are reparented to the new index).
//! - [`CSpace::revoke`] / [`CSpace::delete`] walk the MDB links and clear every
//!   transitive descendant — reclaiming an untyped region or tearing down a
//!   task's CSpace now drives the real slots, not a shadow tree.
//!
//! See `docs/ARCHITECTURE.md` §1.1–1.2 and `docs/GLOSSARY.md`.

use crate::cap::{CapType, Capability, Rights};
use crate::untyped::{RetypeError, Untyped};

/// One capability-table entry: a CSpace slot fusing a [`Capability`] with its
/// mapping-database link to the parent it was derived from.
///
/// `parent == None` marks an *original* capability (e.g. boot untyped or any
/// root handed in by the loader). For untyped entries, `watermark` persists how
/// many bytes have already been carved by [`CSpace::retype`], so repeated
/// retypes keep advancing instead of re-carving the same physical range.
#[derive(Clone, Copy, Debug)]
pub struct Cte {
    cap: Capability,
    parent: Option<usize>,
    watermark: u64,
    used: bool,
}

impl Cte {
    /// An empty, reusable slot. Use to initialise backing storage:
    /// `let mut slots = [Cte::EMPTY; 32];`
    pub const EMPTY: Self = Self {
        cap: Capability::NULL,
        parent: None,
        watermark: 0,
        used: false,
    };

    /// The capability stored in this entry (null for an empty slot).
    #[must_use]
    pub const fn capability(self) -> Capability {
        self.cap
    }

    /// The slot index this entry was derived from, or `None` if it is a root.
    #[must_use]
    pub const fn parent(self) -> Option<usize> {
        self.parent
    }

    /// Whether this slot currently holds a live capability.
    #[must_use]
    pub const fn is_used(self) -> bool {
        self.used
    }
}

impl Default for Cte {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Why a capability-space operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CteError {
    /// The requested slot index lies outside this CSpace.
    OutOfRange,
    /// The destination slot already holds a live capability.
    SlotOccupied,
    /// The source (or target) slot is empty.
    SlotEmpty,
    /// The requested rights are not a subset of the parent's rights.
    RightsEscalation,
    /// The slot named for a retype does not hold an untyped capability.
    NotUntyped,
    /// Carving objects out of the untyped region failed.
    Retype(RetypeError),
}

impl From<RetypeError> for CteError {
    fn from(err: RetypeError) -> Self {
        Self::Retype(err)
    }
}

/// A thread's capability space: a flat array of [`Cte`] slots over
/// caller-owned storage. Never allocates.
pub struct CSpace<'slots> {
    slots: &'slots mut [Cte],
}

impl<'slots> CSpace<'slots> {
    /// Wrap caller-provided storage as a capability space, clearing every slot.
    #[must_use]
    pub fn new(slots: &'slots mut [Cte]) -> Self {
        for slot in slots.iter_mut() {
            *slot = Cte::EMPTY;
        }
        Self { slots }
    }

    /// The number of slots in this capability space.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The number of live (in-use) slots.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.iter().filter(|c| c.used).count()
    }

    /// Read back the full entry at `index`.
    ///
    /// # Errors
    /// [`CteError::OutOfRange`] if `index` is past the end, or
    /// [`CteError::SlotEmpty`] if the slot is unused.
    pub fn entry(&self, index: usize) -> Result<Cte, CteError> {
        let slot = self.slots.get(index).ok_or(CteError::OutOfRange)?;
        if !slot.used {
            return Err(CteError::SlotEmpty);
        }
        Ok(*slot)
    }

    /// Read back the capability at `index`.
    ///
    /// # Errors
    /// [`CteError::OutOfRange`] or [`CteError::SlotEmpty`].
    pub fn get(&self, index: usize) -> Result<Capability, CteError> {
        Ok(self.entry(index)?.cap)
    }

    /// Install an *original* (parentless) capability into an empty slot.
    ///
    /// # Errors
    /// [`CteError::OutOfRange`] if `index` is invalid, or
    /// [`CteError::SlotOccupied`] if the slot is already live.
    pub fn insert_root(&mut self, index: usize, cap: Capability) -> Result<(), CteError> {
        self.install(index, cap, None, 0)
    }

    /// Carve `count` objects of `2^obj_size_bits` bytes out of the untyped
    /// capability in slot `ut`, minting each into consecutive destination slots
    /// starting at `start_index`, recorded as derived from `ut`.
    ///
    /// The untyped slot's watermark is advanced and persisted, so a later
    /// retype continues where this one stopped.
    ///
    /// # Errors
    /// - [`CteError::OutOfRange`]/[`CteError::SlotEmpty`] for a bad `ut` slot.
    /// - [`CteError::NotUntyped`] if `ut` does not hold an untyped capability.
    /// - [`CteError::SlotOccupied`]/[`CteError::OutOfRange`] for a destination.
    /// - [`CteError::Retype`] if the region cannot satisfy the request.
    pub fn retype(
        &mut self,
        ut: usize,
        obj_type: CapType,
        obj_size_bits: u32,
        count: usize,
        rights: Rights,
        start_index: usize,
    ) -> Result<(), CteError> {
        let entry = self.entry(ut)?;
        if entry.cap.cap_type() != CapType::Untyped {
            return Err(CteError::NotUntyped);
        }
        let size_bits = u32::try_from(entry.cap.arg()).map_err(|_| CteError::NotUntyped)?;
        // Resume from however much of this region was carved before.
        let mut region = Untyped::resume(entry.cap.object(), size_bits, entry.watermark);

        // Validate every destination slot up front so a bad index is rejected
        // before any physical memory is committed.
        for i in 0..count {
            let dest = start_index + i;
            let slot = self.slots.get(dest).ok_or(CteError::OutOfRange)?;
            if slot.used {
                return Err(CteError::SlotOccupied);
            }
        }

        for i in 0..count {
            let child = region.retype_one(obj_type, obj_size_bits, rights)?;
            self.install(start_index + i, child, Some(ut), 0)?;
        }

        // Persist the advanced watermark back into the untyped entry.
        self.slots[ut].watermark = region.used_bytes();
        Ok(())
    }

    /// Mint a *derived* capability from `from` into the empty slot `to`, with
    /// `new_rights` (which must be a subset of the parent's rights). The new
    /// entry is recorded as a child of `from`.
    ///
    /// # Errors
    /// - [`CteError::OutOfRange`]/[`CteError::SlotEmpty`] for a bad `from`.
    /// - [`CteError::RightsEscalation`] if `new_rights` is not a subset.
    /// - [`CteError::OutOfRange`]/[`CteError::SlotOccupied`] for `to`.
    pub fn mint(&mut self, from: usize, to: usize, new_rights: Rights) -> Result<(), CteError> {
        let parent = self.entry(from)?;
        if !parent.cap.rights().contains(new_rights) {
            return Err(CteError::RightsEscalation);
        }
        let derived = Capability::new(
            parent.cap.cap_type(),
            parent.cap.object(),
            parent.cap.arg(),
            new_rights,
        );
        self.install(to, derived, Some(from), 0)
    }

    /// Copy the capability at `from` into the empty slot `to`, preserving rights
    /// and parentage (a copy is a *sibling*, not a child).
    ///
    /// # Errors
    /// - [`CteError::OutOfRange`]/[`CteError::SlotEmpty`] for a bad `from`.
    /// - [`CteError::OutOfRange`]/[`CteError::SlotOccupied`] for `to`.
    pub fn copy(&mut self, from: usize, to: usize) -> Result<(), CteError> {
        let src = self.entry(from)?;
        self.install(to, src.cap, src.parent, 0)
    }

    /// Move the capability at `from` into the empty slot `to`, nulling `from`
    /// and reparenting any children that referenced `from` to `to` so the
    /// derivation tree stays intact.
    ///
    /// A move to the same (live) slot is a validated no-op.
    ///
    /// # Errors
    /// - [`CteError::OutOfRange`]/[`CteError::SlotEmpty`] for a bad `from`.
    /// - [`CteError::OutOfRange`]/[`CteError::SlotOccupied`] for `to`.
    pub fn move_cap(&mut self, from: usize, to: usize) -> Result<(), CteError> {
        let moving = self.entry(from)?;
        if from == to {
            return Ok(());
        }
        match self.slots.get(to) {
            None => return Err(CteError::OutOfRange),
            Some(dst) if dst.used => return Err(CteError::SlotOccupied),
            Some(_) => {}
        }
        self.slots[to] = Cte {
            cap: moving.cap,
            parent: moving.parent,
            watermark: moving.watermark,
            used: true,
        };
        self.slots[from] = Cte::EMPTY;
        for slot in self.slots.iter_mut() {
            if slot.used && slot.parent == Some(from) {
                slot.parent = Some(to);
            }
        }
        Ok(())
    }

    /// True if `node` is a transitive descendant of `ancestor`.
    #[must_use]
    pub fn is_descendant(&self, node: usize, ancestor: usize) -> bool {
        if node >= self.slots.len() || ancestor >= self.slots.len() {
            return false;
        }
        let mut cur = self.slots[node].parent;
        let mut steps = 0;
        // Bound the walk by the slot count to defeat any malformed cycle.
        while let Some(p) = cur {
            if p == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.slots.len() {
                return false;
            }
            cur = self.slots[p].parent;
        }
        false
    }

    /// Clear every capability transitively derived from `target`, leaving
    /// `target` itself in place. Returns how many descendants were freed.
    ///
    /// # Errors
    /// [`CteError::OutOfRange`] or [`CteError::SlotEmpty`] for `target`.
    pub fn revoke(&mut self, target: usize) -> Result<usize, CteError> {
        self.entry(target)?;
        let mut freed = 0;
        // Parent links stay intact during the scan so descendant checks for
        // later slots still resolve through already-freed entries; only
        // `used`/`cap` are cleared in this pass.
        for i in 0..self.slots.len() {
            if i == target || !self.slots[i].used {
                continue;
            }
            if self.is_descendant(i, target) {
                self.slots[i].used = false;
                self.slots[i].cap = Capability::NULL;
                self.slots[i].watermark = 0;
                freed += 1;
            }
        }
        // Detach freed slots fully so a later install starts them clean.
        for slot in self.slots.iter_mut() {
            if !slot.used {
                slot.parent = None;
            }
        }
        Ok(freed)
    }

    /// Revoke all descendants of `target` and then clear `target` itself.
    /// Returns the total number of slots freed (descendants + `target`).
    ///
    /// # Errors
    /// [`CteError::OutOfRange`] or [`CteError::SlotEmpty`] for `target`.
    pub fn delete(&mut self, target: usize) -> Result<usize, CteError> {
        let freed = self.revoke(target)?;
        self.slots[target] = Cte::EMPTY;
        Ok(freed + 1)
    }

    /// Install `cap` into an empty slot with the given parent link.
    fn install(
        &mut self,
        index: usize,
        cap: Capability,
        parent: Option<usize>,
        watermark: u64,
    ) -> Result<(), CteError> {
        let slot = self.slots.get_mut(index).ok_or(CteError::OutOfRange)?;
        if slot.used {
            return Err(CteError::SlotOccupied);
        }
        *slot = Cte {
            cap,
            parent,
            watermark,
            used: true,
        };
        Ok(())
    }
}

/// Boot-time self-test exercising the fused CSpace end to end.
///
/// Builds a small space, retypes two pages straight out of an untyped slot,
/// mints a rights-reduced child, refuses an escalation, copies a sibling, moves
/// a populated slot (checking children follow), then revokes and deletes,
/// asserting the descendant accounting at each step.
///
/// # Errors
/// Returns the first [`CteError`] that violates an expected invariant (or a
/// representative error if an operation that should have failed succeeded).
pub fn selftest() -> Result<(), CteError> {
    const PAGE_BITS: u32 = 12;
    const PAGE_SIZE: u64 = 1 << PAGE_BITS;

    let mut storage = [Cte::EMPTY; 12];
    let mut cs = CSpace::new(&mut storage);
    let rw = Rights::READ.union(Rights::WRITE);

    // Slot 0: a 2 MiB untyped region with full authority.
    cs.insert_root(0, Capability::new(CapType::Untyped, 0x4020_0000, 21, Rights::ALL))?;

    // Retype two read/write pages out of it into slots 1 and 2.
    cs.retype(0, CapType::Page, PAGE_BITS, 2, rw, 1)?;
    if cs.count() != 3 {
        return Err(CteError::Retype(RetypeError::OutOfSpace));
    }
    let mut prev = 0u64;
    for i in 1..=2usize {
        let cap = cs.get(i)?;
        if cap.cap_type() != CapType::Page || cap.object() & (PAGE_SIZE - 1) != 0 {
            return Err(CteError::Retype(RetypeError::Misaligned));
        }
        if i > 1 && cap.object() <= prev {
            return Err(CteError::Retype(RetypeError::OutOfSpace));
        }
        prev = cap.object();
        if cs.entry(i)?.parent() != Some(0) || !cs.is_descendant(i, 0) {
            return Err(CteError::SlotEmpty);
        }
    }

    // Mint a read-only child of the page in slot 1 into slot 3.
    cs.mint(1, 3, Rights::READ)?;
    let child = cs.get(3)?;
    if child.rights().bits() != Rights::READ.bits()
        || child.object() != cs.get(1)?.object()
        || cs.entry(3)?.parent() != Some(1)
        || !cs.is_descendant(3, 0)
    {
        return Err(CteError::RightsEscalation);
    }

    // Escalation must be refused: deriving WRITE from a READ-only child.
    if cs.mint(3, 4, Rights::WRITE) != Err(CteError::RightsEscalation) {
        return Err(CteError::RightsEscalation);
    }

    // Copy is a sibling (same parent as the source); move vacates the source.
    cs.copy(1, 4)?;
    if cs.entry(4)?.parent() != Some(0) {
        return Err(CteError::SlotEmpty);
    }
    cs.move_cap(4, 5)?;
    if cs.entry(4).is_ok() || cs.get(5)?.cap_type() != CapType::Page {
        return Err(CteError::SlotOccupied);
    }

    // Move a populated slot: relocating slot 1 must reparent its child (slot 3).
    cs.move_cap(1, 6)?;
    if cs.entry(1).is_ok() || cs.entry(3)?.parent() != Some(6) || !cs.is_descendant(3, 6) {
        return Err(CteError::SlotEmpty);
    }

    // Revoke slot 6: only its descendant (slot 3) is freed; slot 6 survives.
    if cs.revoke(6)? != 1 || cs.entry(3).is_ok() || cs.entry(6).is_err() {
        return Err(CteError::SlotEmpty);
    }

    // Delete the untyped root: every cap derived from it (slots 2, 5, 6) plus
    // the root itself are cleared — four slots, leaving an empty space.
    if cs.delete(0)? != 4 || cs.count() != 0 {
        return Err(CteError::SlotOccupied);
    }

    Ok(())
}
