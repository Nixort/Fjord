// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! BLAKE3 Merkle tree over page-sized leaves (fs-verity style).
//!
//! A Cask's code/data is split into fixed-size pages. Each page is a leaf; the
//! tree is a balanced binary hash tree whose root is sealed into the header and
//! signed. Two operations matter, on opposite sides of the trust boundary:
//!
//! * **Build** ([`MerkleTree::build`]) runs on the *host* (`xtask/shipwright`)
//!   or in Helm when sealing. It needs the whole image and an allocator.
//! * **Verify a single page** ([`verify_page`]) runs in the *loader*, on a page
//!   fault, with only the faulting page, a small inclusion proof, and the
//!   trusted root. It is allocation-free and never hashes the whole image —
//!   the proof is `O(log n)` sibling hashes.
//!
//! ## Domain separation
//!
//! Leaf and parent hashes are tagged with a distinct prefix byte so that a
//! parent node can never be reinterpreted as a leaf (a second-preimage guard):
//!
//! * leaf  = `BLAKE3(0x00 || page_bytes)`
//! * node  = `BLAKE3(0x01 || left_hash || right_hash)`
//!
//! ## Odd levels
//!
//! When a tree level has an odd number of nodes the last node is *promoted*
//! unchanged to the next level (it is never duplicated — duplication enables
//! the classic CVE-2012-2459 style ambiguity). The promotion is implicit in
//! both the builder and the proof, so the two always agree.
//!
//! See `docs/ARCHITECTURE.md` §5.

use crate::blake3;
use alloc::vec::Vec;

/// A 32-byte BLAKE3 digest used as a tree node.
pub type Hash = [u8; blake3::OUT_LEN];

/// Domain tag prefixed to leaf input.
const LEAF_TAG: u8 = 0x00;
/// Domain tag prefixed to interior-node input.
const NODE_TAG: u8 = 0x01;

/// Hashes one page into its leaf digest (`BLAKE3(0x00 || page)`).
#[must_use]
pub fn leaf_hash(page: &[u8]) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[LEAF_TAG]);
    hasher.update(page);
    hasher.finalize()
}

/// Hashes two child digests into their parent (`BLAKE3(0x01 || left || right)`).
#[must_use]
pub fn parent_hash(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[NODE_TAG]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize()
}

/// Which side a proof step's sibling sits on, relative to the running hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The sibling is the left child; the running hash is the right child.
    Left,
    /// The sibling is the right child; the running hash is the left child.
    Right,
}

/// One step of a Merkle inclusion proof: a sibling digest and its side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProofStep {
    /// The sibling node's hash.
    pub sibling: Hash,
    /// Whether `sibling` is the left or right child at this level.
    pub side: Side,
}

/// Re-folds a leaf up to a root using its inclusion proof.
///
/// This is the loader's lazy path: given the faulting `page`, its `index`, the
/// `proof` produced at seal time, and the trusted `root`, it returns whether the
/// page is authentic. Allocation-free and constant in image size.
///
/// `index` is accepted for API symmetry and bounds intent; the fold itself is
/// driven by each step's [`Side`], so a proof minted for a different position
/// cannot validate against this root.
#[must_use]
pub fn verify_page(index: u64, page: &[u8], proof: &[ProofStep], root: &Hash) -> bool {
    let _ = index;
    let mut acc = leaf_hash(page);
    for step in proof {
        acc = match step.side {
            Side::Left => parent_hash(&step.sibling, &acc),
            Side::Right => parent_hash(&acc, &step.sibling),
        };
    }
    eq(&acc, root)
}

/// Compares two digests without a data-dependent early return.
///
/// Used for every root/leaf comparison on the trust path so timing cannot leak
/// how many leading bytes of a forged hash were correct.
#[must_use]
pub fn eq(a: &Hash, b: &Hash) -> bool {
    let mut diff = 0u8;
    for i in 0..blake3::OUT_LEN {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// A fully materialized Merkle tree, kept on the build side for proof minting.
///
/// `levels[0]` is the leaves; the last level is the single-element root. Built
/// with an allocator; never used on the heap-free loader path.
pub struct MerkleTree {
    levels: Vec<Vec<Hash>>,
}

impl MerkleTree {
    /// Builds the tree over `pages` (one leaf per page).
    ///
    /// Returns `None` for an empty page list — a Cask with no content has no
    /// meaningful root and is rejected upstream as malformed.
    #[must_use]
    pub fn build(pages: &[&[u8]]) -> Option<Self> {
        if pages.is_empty() {
            return None;
        }
        let mut levels: Vec<Vec<Hash>> = Vec::new();
        levels.push(pages.iter().map(|p| leaf_hash(p)).collect());

        while levels.last().map_or(0, Vec::len) > 1 {
            let current = levels.last().unwrap();
            let mut next = Vec::with_capacity(current.len().div_ceil(2));
            let mut i = 0;
            while i + 1 < current.len() {
                next.push(parent_hash(&current[i], &current[i + 1]));
                i += 2;
            }
            if i < current.len() {
                // Odd one out: promote unchanged.
                next.push(current[i]);
            }
            levels.push(next);
        }
        Some(Self { levels })
    }

    /// The Merkle root that gets sealed into the Cask header and signed.
    #[must_use]
    pub fn root(&self) -> Hash {
        self.levels.last().expect("non-empty by construction")[0]
    }

    /// The number of leaves (pages) in the tree.
    #[must_use]
    pub fn leaf_count(&self) -> usize {
        self.levels[0].len()
    }

    /// Produces the inclusion proof for leaf `index`, leaf → root.
    ///
    /// Returns `None` if `index` is out of range. Levels where the node is
    /// promoted (no sibling) contribute no step, matching [`verify_page`].
    #[must_use]
    pub fn proof(&self, index: usize) -> Option<Vec<ProofStep>> {
        if index >= self.leaf_count() {
            return None;
        }
        let mut steps = Vec::new();
        let mut idx = index;
        for level in &self.levels {
            if level.len() <= 1 {
                break;
            }
            if idx % 2 == 0 {
                // We are the left child; sibling (if any) is on the right.
                if idx + 1 < level.len() {
                    steps.push(ProofStep {
                        sibling: level[idx + 1],
                        side: Side::Right,
                    });
                }
                // else: promoted node, no step.
            } else {
                steps.push(ProofStep {
                    sibling: level[idx - 1],
                    side: Side::Left,
                });
            }
            idx /= 2;
        }
        Some(steps)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn pages(n: usize, sz: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| (0..sz).map(|j| (i * 7 + j) as u8).collect())
            .collect()
    }

    fn refs(p: &[Vec<u8>]) -> Vec<&[u8]> {
        p.iter().map(Vec::as_slice).collect()
    }

    #[test]
    fn single_page_root_is_leaf() {
        let p = pages(1, 64);
        let tree = MerkleTree::build(&refs(&p)).unwrap();
        assert_eq!(tree.root(), leaf_hash(&p[0]));
        let proof = tree.proof(0).unwrap();
        assert!(proof.is_empty());
        assert!(verify_page(0, &p[0], &proof, &tree.root()));
    }

    #[test]
    fn empty_is_rejected() {
        assert!(MerkleTree::build(&[]).is_none());
    }

    /// Every leaf in trees of many shapes (including odd counts that force
    /// promotion) must verify, and only against its own root and position.
    #[test]
    fn all_proofs_round_trip() {
        for n in [1usize, 2, 3, 4, 5, 7, 8, 9, 16, 17, 31, 100] {
            let p = pages(n, 128);
            let tree = MerkleTree::build(&refs(&p)).unwrap();
            let root = tree.root();
            for i in 0..n {
                let proof = tree.proof(i).unwrap();
                assert!(verify_page(i as u64, &p[i], &proof, &root), "n={n} i={i}");
            }
        }
    }

    #[test]
    fn tampered_page_is_rejected() {
        let p = pages(8, 128);
        let tree = MerkleTree::build(&refs(&p)).unwrap();
        let root = tree.root();
        let proof = tree.proof(3).unwrap();
        let mut bad = p[3].clone();
        bad[0] ^= 0x01;
        assert!(!verify_page(3, &bad, &proof, &root));
    }

    #[test]
    fn wrong_proof_position_is_rejected() {
        let p = pages(8, 128);
        let tree = MerkleTree::build(&refs(&p)).unwrap();
        let root = tree.root();
        // Page 2 verified with page 5's proof must fail.
        let proof_for_5 = tree.proof(5).unwrap();
        assert!(!verify_page(2, &p[2], &proof_for_5, &root));
    }

    #[test]
    fn out_of_range_proof_is_none() {
        let p = pages(4, 64);
        let tree = MerkleTree::build(&refs(&p)).unwrap();
        assert!(tree.proof(4).is_none());
    }
}
