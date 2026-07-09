// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Untyped memory and the retype model.
//!
//! All kernel objects are carved out of untyped memory by an explicit retype
//! operation, so kernel memory is accounted to whoever holds the untyped cap.
//! This removes implicit kernel heaps from the TCB (seL4 model).
//!
//! An [`Untyped`] tracks a power-of-two physical region and a monotonic
//! watermark. [`Untyped::retype_one`] carves a single naturally-aligned object
//! from the region and hands back a fresh [`Capability`] naming it;
//! [`Untyped::retype_into`] carves a run of objects straight into a [`CNode`].
//! Memory is never reclaimed implicitly — a later `revoke`/CDT slice will undo a
//! retype by walking the derivation tree.

use crate::cap::{CNode, CapError, CapType, Capability, Rights};

/// Page object size in address bits (4 KiB), matching `hull`'s frame size.
pub const PAGE_BITS: u32 = 12;

/// Round `value` up to the next multiple of `align` (a power of two).
const fn align_up(value: u64, align: u64) -> u64 {
    (value + (align - 1)) & !(align - 1)
}

/// Why a retype request was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RetypeError {
    /// Object size in bits is zero or does not fit in a 64-bit region.
    BadSize,
    /// Not enough room left in the region for the requested object(s).
    OutOfSpace,
    /// A carved object was not naturally aligned (internal invariant break).
    Misaligned,
    /// The destination CNode rejected a carved capability.
    Cspace(CapError),
}

/// A region of untyped (as-yet-unstructured) physical memory.
///
/// The region spans `[base, base + 2^size_bits)`. `watermark` counts the bytes
/// already handed out, measured from `base`; it only ever moves forward.
#[derive(Clone, Copy, Debug)]
pub struct Untyped {
    base: u64,
    size_bits: u32,
    watermark: u64,
}

impl Untyped {
    /// Create an untyped region covering `2^size_bits` bytes at `base`.
    #[must_use]
    pub const fn new(base: u64, size_bits: u32) -> Self {
        Self {
            base,
            size_bits,
            watermark: 0,
        }
    }

    /// Reconstruct a partially-consumed region with `used_bytes` already
    /// carved, so a retype watermark can be persisted outside the region (e.g.
    /// in a capability-table entry) without exposing the internal field.
    #[must_use]
    pub const fn resume(base: u64, size_bits: u32, used_bytes: u64) -> Self {
        Self {
            base,
            size_bits,
            watermark: used_bytes,
        }
    }

    /// Reconstruct the region described by an `Untyped` capability.
    ///
    /// Returns `None` for a non-untyped capability or an out-of-range size.
    #[must_use]
    pub fn from_cap(cap: Capability) -> Option<Self> {
        if cap.cap_type() != CapType::Untyped {
            return None;
        }
        let size_bits = u32::try_from(cap.arg()).ok()?;
        if size_bits == 0 || size_bits >= 64 {
            return None;
        }
        Some(Self::new(cap.object(), size_bits))
    }

    /// Base physical address of the region.
    #[must_use]
    pub const fn base(self) -> u64 {
        self.base
    }

    /// Total size of the region in bytes.
    #[must_use]
    pub const fn region_bytes(self) -> u64 {
        1u64 << self.size_bits
    }

    /// Bytes already carved out of the region.
    #[must_use]
    pub const fn used_bytes(self) -> u64 {
        self.watermark
    }

    /// Bytes still available (before alignment of the next object).
    #[must_use]
    pub const fn remaining(self) -> u64 {
        self.region_bytes() - self.watermark
    }

    /// Carve a single naturally-aligned object of `2^obj_size_bits` bytes.
    ///
    /// On success the watermark advances past the object and a fresh capability
    /// of `obj_type` naming it is returned. The capability's `arg` carries the
    /// object's size in bits so the region can be reconstructed on revoke.
    ///
    /// # Errors
    /// Returns [`RetypeError::BadSize`] if `obj_size_bits` is zero or ≥ 64, and
    /// [`RetypeError::OutOfSpace`] if the aligned object would not fit.
    pub fn retype_one(
        &mut self,
        obj_type: CapType,
        obj_size_bits: u32,
        rights: Rights,
    ) -> Result<Capability, RetypeError> {
        if obj_size_bits == 0 || obj_size_bits >= 64 {
            return Err(RetypeError::BadSize);
        }
        let obj_size = 1u64 << obj_size_bits;
        let region_end = self.base + self.region_bytes();
        let start = align_up(self.base + self.watermark, obj_size);
        // Guard against address wraparound as well as plain exhaustion.
        if start < self.base || start > region_end - obj_size {
            return Err(RetypeError::OutOfSpace);
        }
        let end = start + obj_size;
        self.watermark = end - self.base;
        Ok(Capability::new(
            obj_type,
            start,
            u64::from(obj_size_bits),
            rights,
        ))
    }

    /// Carve `count` objects of `2^obj_size_bits` bytes into `dest`, starting at
    /// slot `start_index`. Carved capabilities are inserted in order.
    ///
    /// # Errors
    /// Propagates [`RetypeError::BadSize`]/[`RetypeError::OutOfSpace`] from
    /// carving, or [`RetypeError::Cspace`] if the destination CNode rejects an
    /// insert (e.g. an occupied or out-of-range slot).
    pub fn retype_into(
        &mut self,
        obj_type: CapType,
        obj_size_bits: u32,
        count: usize,
        rights: Rights,
        dest: &mut CNode<'_>,
        start_index: usize,
    ) -> Result<(), RetypeError> {
        let end_index = start_index
            .checked_add(count)
            .ok_or(RetypeError::Cspace(CapError::OutOfRange))?;
        if end_index > dest.capacity() {
            return Err(RetypeError::Cspace(CapError::OutOfRange));
        }

        // Validate destination slots before moving the watermark. Retype must
        // be atomic from the caller's point of view: a bad CSpace request must
        // not consume physical memory.
        for i in start_index..end_index {
            if !dest.get(i).map_err(RetypeError::Cspace)?.is_null() {
                return Err(RetypeError::Cspace(CapError::SlotOccupied));
            }
        }

        // Probe the exact carving sequence against a copy. Only after every
        // object fits do we commit capabilities into the destination CNode and
        // publish the advanced watermark.
        let mut probe = *self;
        for _ in 0..count {
            probe.retype_one(obj_type, obj_size_bits, rights)?;
        }

        let mut commit = *self;
        for i in 0..count {
            let cap = commit.retype_one(obj_type, obj_size_bits, rights)?;
            dest.insert(start_index + i, cap)
                .map_err(RetypeError::Cspace)?;
        }
        *self = commit;
        Ok(())
    }
}

/// Boot-time self-test exercising retype accounting and exhaustion.
///
/// # Errors
/// Returns a [`RetypeError`] if any invariant (object type, alignment,
/// ascending addresses, watermark accounting, or exhaustion) is violated.
pub fn selftest() -> Result<(), RetypeError> {
    // A 2 MiB untyped region at a plausible (unused) physical base.
    let mut ut = Untyped::new(0x4020_0000, 21);
    let page_size = 1u64 << PAGE_BITS;

    // Carve three read/write pages into a small CNode.
    let mut storage = [Capability::NULL; 4];
    let mut cnode = CNode::new(&mut storage);
    let rw = Rights::READ.union(Rights::WRITE);
    ut.retype_into(CapType::Page, PAGE_BITS, 3, rw, &mut cnode, 0)?;

    // Each carved page must be a Page, page-aligned, and strictly ascending.
    let mut prev = 0u64;
    for i in 0..3usize {
        let cap = cnode.get(i).map_err(RetypeError::Cspace)?;
        if cap.cap_type() != CapType::Page {
            return Err(RetypeError::BadSize);
        }
        let addr = cap.object();
        if addr & (page_size - 1) != 0 {
            return Err(RetypeError::Misaligned);
        }
        if i > 0 && addr <= prev {
            return Err(RetypeError::OutOfSpace);
        }
        prev = addr;
    }

    // Accounting: exactly three pages have been consumed.
    if ut.used_bytes() != 3 * page_size {
        return Err(RetypeError::OutOfSpace);
    }


    // Atomicity: a bad destination range must not advance the watermark.
    let mut tiny_storage = [Capability::NULL; 1];
    let mut tiny = CNode::new(&mut tiny_storage);
    let before = ut.used_bytes();
    if !matches!(
        ut.retype_into(CapType::Page, PAGE_BITS, 2, rw, &mut tiny, 0),
        Err(RetypeError::Cspace(CapError::OutOfRange))
    ) || ut.used_bytes() != before
    {
        return Err(RetypeError::OutOfSpace);
    }

    // Exhaustion: a 4 MiB object cannot come out of a 2 MiB region.
    if !matches!(
        ut.retype_one(CapType::Untyped, 22, Rights::ALL),
        Err(RetypeError::OutOfSpace)
    ) {
        return Err(RetypeError::OutOfSpace);
    }

    Ok(())
}
