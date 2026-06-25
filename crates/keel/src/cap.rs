// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Capabilities and capability spaces (CSpace).
//!
//! Every kernel object is named by an unforgeable capability. Authority is
//! delegated by minting derived capabilities with a reduced rights set;
//! revocation (a later slice) walks a derivation tree. There is no ambient
//! authority anywhere in the system.
//!
//! A [`CNode`] is one level of a thread's capability space: a fixed array of
//! capability slots. Its backing storage is owned by the caller (ultimately
//! retyped from untyped memory), so capability bookkeeping never touches a
//! kernel heap — there is none (`docs/ARCHITECTURE.md` §1.1).
//!
//! See `docs/ARCHITECTURE.md` §1 and `docs/GLOSSARY.md`.

/// Access rights carried by a [`Capability`].
///
/// Rights form a lattice ordered by subset inclusion. A derived capability
/// (see [`CNode::mint`]) may only ever *drop* rights, never gain them — this is
/// what makes authority delegation monotonic.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Rights(u32);

impl Rights {
    /// No authority at all.
    pub const NONE: Self = Self(0);
    /// Permission to read the object's backing memory or receive on it.
    pub const READ: Self = Self(1 << 0);
    /// Permission to write the object's backing memory or send to it.
    pub const WRITE: Self = Self(1 << 1);
    /// Permission to delegate (grant) capabilities through this object.
    pub const GRANT: Self = Self(1 << 2);
    /// Permission to map the object's memory executable.
    pub const EXECUTE: Self = Self(1 << 3);
    /// Permission to seal/unseal the object (rights amplification).
    pub const SEAL: Self = Self(1 << 4);
    /// Every right defined above.
    pub const ALL: Self = Self(0b1_1111);

    /// Returns the raw bit pattern.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Builds a rights set from a raw bit pattern, dropping unknown bits.
    #[must_use]
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// Returns `true` if every right in `other` is also present in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union (logical OR) of two rights sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The intersection (logical AND) of two rights sets.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Returns `true` if `self` carries no rights.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Debug for Rights {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let flag = |bit: Rights, ch: char| if self.contains(bit) { ch } else { '-' };
        write!(
            f,
            "{}{}{}{}{}",
            flag(Rights::READ, 'r'),
            flag(Rights::WRITE, 'w'),
            flag(Rights::GRANT, 'g'),
            flag(Rights::EXECUTE, 'x'),
            flag(Rights::SEAL, 's'),
        )
    }
}

/// The kind of kernel object a [`Capability`] refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapType {
    /// An empty slot — refers to nothing.
    Null,
    /// A region of untyped physical memory awaiting retype.
    Untyped,
    /// A capability-storage node (one CSpace level).
    CNode,
    /// A physical memory frame that can be mapped into a `VSpace`.
    Page,
    /// A synchronous IPC endpoint.
    Endpoint,
    /// An asynchronous notification object.
    Notification,
    /// A thread control block.
    Tcb,
    /// A handle to a hardware interrupt line.
    Irq,
}

/// An unforgeable, typed reference to a kernel object together with the
/// [`Rights`] the holder may exercise over it.
///
/// Capabilities are plain copyable values; their authority comes from the fact
/// that they can only be obtained through kernel-mediated operations
/// ([`CNode::insert`], [`CNode::copy`], [`CNode::mint`], retype, …), never
/// fabricated by userspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capability {
    cap_type: CapType,
    object: u64,
    arg: u64,
    rights: Rights,
}

impl Capability {
    /// The canonical empty capability stored in unused [`CNode`] slots.
    pub const NULL: Self = Self {
        cap_type: CapType::Null,
        object: 0,
        arg: 0,
        rights: Rights::NONE,
    };

    /// Constructs a capability of `cap_type` over object handle `object`.
    ///
    /// `arg` carries a type-specific parameter (untyped size-bits, CNode radix,
    /// page order, endpoint badge, …); `rights` is the authority granted.
    #[must_use]
    pub const fn new(cap_type: CapType, object: u64, arg: u64, rights: Rights) -> Self {
        Self {
            cap_type,
            object,
            arg,
            rights,
        }
    }

    /// The kind of object this capability names.
    #[must_use]
    pub const fn cap_type(self) -> CapType {
        self.cap_type
    }

    /// The kernel-internal object handle (typically a physical base address).
    #[must_use]
    pub const fn object(self) -> u64 {
        self.object
    }

    /// The type-specific parameter word.
    #[must_use]
    pub const fn arg(self) -> u64 {
        self.arg
    }

    /// The rights granted to the holder.
    #[must_use]
    pub const fn rights(self) -> Rights {
        self.rights
    }

    /// Returns `true` if this is the empty ([`CapType::Null`]) capability.
    #[must_use]
    pub const fn is_null(self) -> bool {
        matches!(self.cap_type, CapType::Null)
    }
}

impl Default for Capability {
    fn default() -> Self {
        Self::NULL
    }
}

/// Why a capability-space operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapError {
    /// The requested slot index lies outside this CNode.
    OutOfRange,
    /// The destination slot already holds a (non-null) capability.
    SlotOccupied,
    /// The source slot is empty.
    SlotEmpty,
    /// The requested rights are not a subset of the parent's rights.
    RightsEscalation,
}

/// A capability-storage node: one level of a thread's capability space
/// (CSpace), holding a fixed array of capability slots.
///
/// The slot storage is borrowed from the caller (ultimately retyped from
/// untyped memory), so a `CNode` never allocates — consistent with Fjord's
/// no-kernel-heap rule (`docs/ARCHITECTURE.md` §1.1).
pub struct CNode<'slots> {
    slots: &'slots mut [Capability],
}

impl<'slots> CNode<'slots> {
    /// Wraps caller-provided storage as a CNode, clearing every slot to null.
    #[must_use]
    pub fn new(slots: &'slots mut [Capability]) -> Self {
        for slot in slots.iter_mut() {
            *slot = Capability::NULL;
        }
        Self { slots }
    }

    /// The number of capability slots in this node.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns a copy of the capability in `index`.
    ///
    /// # Errors
    /// [`CapError::OutOfRange`] if `index` is past the end of the node.
    pub fn get(&self, index: usize) -> Result<Capability, CapError> {
        self.slots.get(index).copied().ok_or(CapError::OutOfRange)
    }

    /// Installs `cap` into an empty slot.
    ///
    /// # Errors
    /// - [`CapError::OutOfRange`] if `index` is invalid.
    /// - [`CapError::SlotOccupied`] if the slot is not null.
    pub fn insert(&mut self, index: usize, cap: Capability) -> Result<(), CapError> {
        let slot = self.slots.get_mut(index).ok_or(CapError::OutOfRange)?;
        if !slot.is_null() {
            return Err(CapError::SlotOccupied);
        }
        *slot = cap;
        Ok(())
    }

    /// Removes and returns the capability in `index`, leaving the slot null.
    ///
    /// # Errors
    /// - [`CapError::OutOfRange`] if `index` is invalid.
    /// - [`CapError::SlotEmpty`] if the slot is already null.
    pub fn delete(&mut self, index: usize) -> Result<Capability, CapError> {
        let slot = self.slots.get_mut(index).ok_or(CapError::OutOfRange)?;
        if slot.is_null() {
            return Err(CapError::SlotEmpty);
        }
        Ok(core::mem::replace(slot, Capability::NULL))
    }

    /// Copies the capability at `from` into the empty slot `to`, preserving
    /// rights.
    ///
    /// # Errors
    /// - [`CapError::OutOfRange`] if either index is invalid.
    /// - [`CapError::SlotEmpty`] if `from` is null.
    /// - [`CapError::SlotOccupied`] if `to` is not null.
    pub fn copy(&mut self, from: usize, to: usize) -> Result<(), CapError> {
        let cap = self.get(from)?;
        if cap.is_null() {
            return Err(CapError::SlotEmpty);
        }
        self.insert(to, cap)
    }

    /// Mints a *derived* capability from `from` into the empty slot `to`, with
    /// `new_rights`, which must be a subset of the parent's rights.
    ///
    /// # Errors
    /// - [`CapError::OutOfRange`] if either index is invalid.
    /// - [`CapError::SlotEmpty`] if `from` is null.
    /// - [`CapError::SlotOccupied`] if `to` is not null.
    /// - [`CapError::RightsEscalation`] if `new_rights` is not a subset of the
    ///   parent's rights.
    pub fn mint(&mut self, from: usize, to: usize, new_rights: Rights) -> Result<(), CapError> {
        let parent = self.get(from)?;
        if parent.is_null() {
            return Err(CapError::SlotEmpty);
        }
        if !parent.rights().contains(new_rights) {
            return Err(CapError::RightsEscalation);
        }
        let derived =
            Capability::new(parent.cap_type(), parent.object(), parent.arg(), new_rights);
        self.insert(to, derived)
    }

    /// Moves the capability at `from` into the empty slot `to`, nulling `from`.
    ///
    /// A move to the same slot is a no-op (still validated).
    ///
    /// # Errors
    /// - [`CapError::OutOfRange`] if either index is invalid.
    /// - [`CapError::SlotEmpty`] if `from` is null.
    /// - [`CapError::SlotOccupied`] if `to` is not null.
    pub fn move_cap(&mut self, from: usize, to: usize) -> Result<(), CapError> {
        if from == to {
            let cap = self.get(from)?;
            return if cap.is_null() {
                Err(CapError::SlotEmpty)
            } else {
                Ok(())
            };
        }
        let cap = self.get(from)?;
        if cap.is_null() {
            return Err(CapError::SlotEmpty);
        }
        match self.slots.get(to) {
            None => return Err(CapError::OutOfRange),
            Some(dst) if !dst.is_null() => return Err(CapError::SlotOccupied),
            Some(_) => {}
        }
        self.slots[to] = cap;
        self.slots[from] = Capability::NULL;
        Ok(())
    }
}

/// Exercise the capability-space core on a throwaway stack CNode.
///
/// Proves the invariants the rest of Phase 2 relies on: insert/copy/move/
/// delete bookkeeping, monotonic rights reduction on [`CNode::mint`], and the
/// refusal of rights escalation. Runs early in boot as a smoke test, mirroring
/// the frame-allocator self-test.
///
/// # Errors
/// Returns the first [`CapError`] that violates an expected invariant, or a
/// representative error if an operation that should have failed unexpectedly
/// succeeded.
pub fn selftest() -> Result<(), CapError> {
    let mut storage = [Capability::NULL; 8];
    let mut cnode = CNode::new(&mut storage);

    // An untyped region with full authority in slot 0.
    let untyped = Capability::new(CapType::Untyped, 0x4000_0000, 24, Rights::ALL);
    cnode.insert(0, untyped)?;

    // Mint a read-only derived cap into slot 1: rights must shrink, identity kept.
    cnode.mint(0, 1, Rights::READ)?;
    let derived = cnode.get(1)?;
    if derived.rights().bits() != Rights::READ.bits()
        || derived.object() != untyped.object()
        || derived.cap_type() != CapType::Untyped
    {
        return Err(CapError::RightsEscalation);
    }

    // Escalation must be refused: deriving WRITE from a READ-only cap.
    if cnode.mint(1, 2, Rights::WRITE) != Err(CapError::RightsEscalation) {
        return Err(CapError::RightsEscalation);
    }

    // Copy preserves rights; move vacates the source slot.
    cnode.copy(0, 3)?;
    cnode.move_cap(3, 4)?;
    if !cnode.get(3)?.is_null() || cnode.get(4)?.cap_type() != CapType::Untyped {
        return Err(CapError::SlotEmpty);
    }

    // Delete clears the slot and a double-delete is refused.
    cnode.delete(0)?;
    if !matches!(cnode.delete(0), Err(CapError::SlotEmpty)) {
        return Err(CapError::SlotEmpty);
    }

    Ok(())
}
