// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Logbook — transparency log client
//!
//! An append-only, Merkle-backed log of Cask signatures and revocations
//! (Certificate-Transparency / Sigstore-Rekor lineage). A Cask is only
//! trusted if its signature appears in Logbook with a valid inclusion proof,
//! which makes silent or targeted signing detectable and enables revocation.
//! See `docs/ARCHITECTURE.md` §5.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

use alloc::vec::Vec;
use cask::merkle::{self, Hash, ProofStep};

/// A signed checkpoint (root hash + tree size) of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Number of leaves in the transparency tree.
    pub tree_size: u64,
    /// Root hash committed by the log operator.
    pub root_hash: Hash,
    /// Opaque signature bytes over `(tree_size, root_hash)`.
    pub signature: Vec<u8>,
}

/// Proof that a leaf is included in the log at a given checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    /// Leaf index within the tree.
    pub leaf_index: u64,
    /// Audit path from leaf to root.
    pub path: Vec<ProofStep>,
}

/// A revocation entry already authenticated by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revocation {
    /// Hash of a revoked signature/log leaf.
    pub leaf_hash: Hash,
}

/// Why Logbook validation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogbookError {
    /// The checkpoint is empty or malformed.
    BadCheckpoint,
    /// The inclusion proof does not fold to the checkpoint root.
    BadInclusionProof,
    /// The signature/leaf was present in the revocation feed.
    Revoked,
}

/// Hashes the signed Cask signature block into a transparency-log leaf.
#[must_use]
pub fn signature_leaf(signature: &[u8]) -> Hash {
    merkle::leaf_hash(signature)
}

/// Verify that a signature is logged and not revoked under `checkpoint`.
pub fn verify_inclusion(
    leaf_hash: &Hash,
    checkpoint: &Checkpoint,
    proof: &InclusionProof,
    revocations: &[Revocation],
) -> Result<(), LogbookError> {
    if checkpoint.tree_size == 0 || proof.leaf_index >= checkpoint.tree_size {
        return Err(LogbookError::BadCheckpoint);
    }
    if revocations.iter().any(|r| merkle::eq(&r.leaf_hash, leaf_hash)) {
        return Err(LogbookError::Revoked);
    }

    // The Cask Merkle verifier expects page bytes. Logbook proof material is
    // already a leaf hash, so fold directly using the same parent function and
    // strict side checks.
    let mut acc = *leaf_hash;
    let mut idx = proof.leaf_index;
    let mut width = checkpoint.tree_size;
    let mut pos = 0usize;
    while width > 1 {
        if idx % 2 == 0 && idx + 1 >= width {
            idx /= 2;
            width = width.div_ceil(2);
            continue;
        }
        let step = proof.path.get(pos).ok_or(LogbookError::BadInclusionProof)?;
        let expected = if idx % 2 == 0 {
            merkle::Side::Right
        } else {
            merkle::Side::Left
        };
        if step.side != expected {
            return Err(LogbookError::BadInclusionProof);
        }
        acc = match step.side {
            merkle::Side::Left => merkle::parent_hash(&step.sibling, &acc),
            merkle::Side::Right => merkle::parent_hash(&acc, &step.sibling),
        };
        pos += 1;
        idx /= 2;
        width = width.div_ceil(2);
    }

    if pos == proof.path.len() && merkle::eq(&acc, &checkpoint.root_hash) {
        Ok(())
    } else {
        Err(LogbookError::BadInclusionProof)
    }
}
