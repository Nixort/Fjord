// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 24 june 2026

//! Capability derivation tree (CDT) and recursive revoke.
//!
//! `retype` and `mint` create *derived* capabilities: a page minted out of an
//! untyped, a read-only child minted from a read/write parent, and so on. To
//! make those derivations reversible the kernel records parent -> child edges
//! in a derivation tree. [`Cdt::revoke`] then deletes every capability derived
//! (transitively) from a target, which is how a holder reclaims an untyped
//! region or tears down a task's CSpace.
//!
//! This models the structure only — rights monotonicity is enforced separately
//! by [`crate::cap::CNode::mint`]. Storage is caller-owned: the tree lives in a
//! `&mut [CdtNode]` slice, so there is no kernel heap and the node budget is
//! explicit. A follow-up slice will fuse a `CdtNode` into each CSpace entry
//! (an seL4 "CTE": capability + mdb links) so revoke can drive real slots.

use crate::cap::Capability;

/// One node of the derivation tree: a capability plus a link to its parent.
///
/// Nodes never move, so a parent is referenced by its stable slot index.
/// `parent == None` marks a root (an originally-untyped capability).
#[derive(Clone, Copy, Debug)]
pub struct CdtNode {
    cap: Capability,
    parent: Option<usize>,
    used: bool,
}

impl CdtNode {
    /// An empty, reusable node. Use to initialise backing storage:
    /// `let mut nodes = [CdtNode::EMPTY; 16];`
    pub const EMPTY: Self = Self {
        cap: Capability::NULL,
        parent: None,
        used: false,
    };
}

impl Default for CdtNode {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Why a derivation-tree operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CdtError {
    /// No free node remains in the backing storage.
    Full,
    /// The index is outside the backing storage.
    OutOfRange,
    /// The referenced node is not in use.
    Empty,
}

/// A capability derivation tree over caller-owned node storage.
pub struct Cdt<'nodes> {
    nodes: &'nodes mut [CdtNode],
}

impl<'nodes> Cdt<'nodes> {
    /// Wrap a slice of node storage, clearing every node to empty.
    #[must_use]
    pub fn new(nodes: &'nodes mut [CdtNode]) -> Self {
        for node in nodes.iter_mut() {
            *node = CdtNode::EMPTY;
        }
        Self { nodes }
    }

    /// Number of nodes the tree can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.nodes.len()
    }

    /// Number of live (in-use) nodes.
    #[must_use]
    pub fn count(&self) -> usize {
        self.nodes.iter().filter(|n| n.used).count()
    }

    /// Find the first free node slot.
    fn alloc(&mut self) -> Result<usize, CdtError> {
        self.nodes
            .iter()
            .position(|n| !n.used)
            .ok_or(CdtError::Full)
    }

    /// Insert a root capability (one with no parent, e.g. boot untyped).
    ///
    /// # Errors
    /// Returns [`CdtError::Full`] if no free node remains.
    pub fn insert_root(&mut self, cap: Capability) -> Result<usize, CdtError> {
        let idx = self.alloc()?;
        self.nodes[idx] = CdtNode {
            cap,
            parent: None,
            used: true,
        };
        Ok(idx)
    }

    /// Record `cap` as derived from the live node `parent`.
    ///
    /// # Errors
    /// Returns [`CdtError::OutOfRange`] if `parent` is out of bounds,
    /// [`CdtError::Empty`] if it is not in use, or [`CdtError::Full`] if there
    /// is no free node.
    pub fn derive(&mut self, parent: usize, cap: Capability) -> Result<usize, CdtError> {
        self.check_live(parent)?;
        let idx = self.alloc()?;
        self.nodes[idx] = CdtNode {
            cap,
            parent: Some(parent),
            used: true,
        };
        Ok(idx)
    }

    /// Read back the capability stored at `idx`.
    ///
    /// # Errors
    /// Returns [`CdtError::OutOfRange`] or [`CdtError::Empty`].
    pub fn get(&self, idx: usize) -> Result<Capability, CdtError> {
        self.check_live(idx)?;
        Ok(self.nodes[idx].cap)
    }

    /// True if `node` is a transitive descendant of `ancestor`.
    #[must_use]
    pub fn is_descendant(&self, node: usize, ancestor: usize) -> bool {
        if node >= self.nodes.len() || ancestor >= self.nodes.len() {
            return false;
        }
        // Walk parent links upward, bounded by node count to defeat any cycle.
        let mut cur = self.nodes[node].parent;
        let mut steps = 0;
        while let Some(p) = cur {
            if p == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.nodes.len() {
                return false;
            }
            cur = self.nodes[p].parent;
        }
        false
    }

    /// Delete every capability transitively derived from `target`, leaving
    /// `target` itself in place. Returns how many descendants were freed.
    ///
    /// # Errors
    /// Returns [`CdtError::OutOfRange`] or [`CdtError::Empty`] for `target`.
    pub fn revoke(&mut self, target: usize) -> Result<usize, CdtError> {
        self.check_live(target)?;
        let mut freed = 0;
        // Parent links are left intact during the pass so that descendant
        // checks for later nodes still resolve through already-freed slots;
        // only `used`/`cap` are cleared here.
        for i in 0..self.nodes.len() {
            if i == target || !self.nodes[i].used {
                continue;
            }
            if self.is_descendant(i, target) {
                self.nodes[i].used = false;
                self.nodes[i].cap = Capability::NULL;
                freed += 1;
            }
        }
        // Now that the pass is done, detach the freed slots fully so a later
        // alloc starts them clean.
        for node in self.nodes.iter_mut() {
            if !node.used {
                node.parent = None;
            }
        }
        Ok(freed)
    }

    /// Revoke all descendants of `target` and then delete `target` itself.
    ///
    /// # Errors
    /// Returns [`CdtError::OutOfRange`] or [`CdtError::Empty`] for `target`.
    pub fn delete(&mut self, target: usize) -> Result<usize, CdtError> {
        let freed = self.revoke(target)?;
        self.nodes[target].used = false;
        self.nodes[target].cap = Capability::NULL;
        self.nodes[target].parent = None;
        Ok(freed + 1)
    }

    /// Bounds- and liveness-check an index.
    fn check_live(&self, idx: usize) -> Result<(), CdtError> {
        if idx >= self.nodes.len() {
            return Err(CdtError::OutOfRange);
        }
        if !self.nodes[idx].used {
            return Err(CdtError::Empty);
        }
        Ok(())
    }
}

/// Boot-time self-test exercising derive/revoke/delete accounting.
///
/// Builds the tree
/// ```text
/// root
///  +-- a
///  |    +-- c
///  |         +-- d
///  +-- b
/// ```
/// then revokes `a` (must drop `c` and `d` only) and `root` (must drop `a`
/// and `b`), and finally deletes `root`.
///
/// # Errors
/// Returns a [`CdtError`] if any structural or accounting invariant fails.
pub fn selftest() -> Result<(), CdtError> {
    use crate::cap::{CapType, Rights};

    let mut storage = [CdtNode::EMPTY; 8];
    let mut cdt = Cdt::new(&mut storage);

    let cap = |obj| Capability::new(CapType::Page, obj, 12, Rights::READ);
    let root = cdt.insert_root(Capability::new(
        CapType::Untyped,
        0x4020_0000,
        21,
        Rights::ALL,
    ))?;
    let a = cdt.derive(root, cap(0x1000))?;
    let b = cdt.derive(root, cap(0x2000))?;
    let c = cdt.derive(a, cap(0x3000))?;
    let d = cdt.derive(c, cap(0x4000))?;

    if cdt.count() != 5 {
        return Err(CdtError::Full);
    }
    if !cdt.is_descendant(d, root) || !cdt.is_descendant(d, a) || cdt.is_descendant(d, b) {
        return Err(CdtError::Empty);
    }

    // Revoke `a`: its subtree (c, d) goes; a, b, root stay.
    if cdt.revoke(a)? != 2 {
        return Err(CdtError::Full);
    }
    if cdt.count() != 3 {
        return Err(CdtError::Full);
    }
    cdt.get(a)?; // still live
    cdt.get(b)?; // still live
    if !matches!(cdt.get(c), Err(CdtError::Empty)) || !matches!(cdt.get(d), Err(CdtError::Empty)) {
        return Err(CdtError::Empty);
    }

    // Revoke `root`: a and b go; root remains.
    if cdt.revoke(root)? != 2 {
        return Err(CdtError::Full);
    }
    if cdt.count() != 1 {
        return Err(CdtError::Full);
    }

    // Delete `root` itself: tree is now empty.
    if cdt.delete(root)? != 1 {
        return Err(CdtError::Full);
    }
    if cdt.count() != 0 {
        return Err(CdtError::Full);
    }

    Ok(())
}
