// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// The code was written for Fjord.

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

/// A signed checkpoint (log identity + root hash + tree size) of the log.
///
/// The detached `signature` is never trusted merely because bytes are present:
/// [`verify_inclusion`] authenticates the domain-separated checkpoint payload
/// through a caller-supplied [`CheckpointAuthenticator`] before accepting its
/// root as a transparency trust anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Stable public identity of the Logbook instance that issued this head.
    pub log_id: Hash,
    /// Number of leaves in the transparency tree.
    pub tree_size: u64,
    /// Root hash committed by the log operator.
    pub root_hash: Hash,
    /// Detached trust-anchor signature over [`Self::signed_message`].
    pub signature: Vec<u8>,
}

impl Checkpoint {
    /// Canonical, domain-separated message authenticated by the log trust anchor.
    ///
    /// It binds the log identity, exact tree size and root. Big-endian encoding
    /// is fixed at the protocol boundary, so a verifier can be implemented in an
    /// HSM, Anchor, or a userspace signature service without host-endian drift.
    #[must_use]
    pub fn signed_message(&self) -> [u8; CHECKPOINT_MESSAGE_LEN] {
        let mut message = [0u8; CHECKPOINT_MESSAGE_LEN];
        let mut offset = 0usize;
        message[offset..offset + CHECKPOINT_CONTEXT.len()].copy_from_slice(&CHECKPOINT_CONTEXT);
        offset += CHECKPOINT_CONTEXT.len();
        message[offset..offset + self.log_id.len()].copy_from_slice(&self.log_id);
        offset += self.log_id.len();
        message[offset..offset + core::mem::size_of::<u64>()]
            .copy_from_slice(&self.tree_size.to_be_bytes());
        offset += core::mem::size_of::<u64>();
        message[offset..].copy_from_slice(&self.root_hash);
        message
    }
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

/// Domain separator for a Logbook signed tree head.
///
/// Its fixed spelling/version prevents an otherwise valid signature for a Cask,
/// revocation record, or another protocol from being replayed as a checkpoint.
const CHECKPOINT_CONTEXT: [u8; 27] = *b"FJORD-LOGBOOK-CHECKPOINT-v1";
/// Bytes authenticated for every checkpoint: context, log identity, size, root.
pub const CHECKPOINT_MESSAGE_LEN: usize = CHECKPOINT_CONTEXT.len() + 32 + 8 + 32;

/// Cryptographic trust anchor for a Logbook checkpoint.
///
/// The trait is deliberately narrow: Anchor/HSM-backed code supplies a real
/// Ed25519 or hybrid verifier later, while Logbook owns canonical serialization,
/// mandatory non-empty signatures, and fail-closed policy now. There is no
/// permissive default implementation.
pub trait CheckpointAuthenticator {
    /// Return `true` only when `signature` authenticates the exact checkpoint
    /// `message` under this anchor's configured Logbook public key.
    fn verify_checkpoint(&self, message: &[u8], signature: &[u8]) -> bool;
}

/// Why Logbook validation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogbookError {
    /// The checkpoint is empty or malformed.
    BadCheckpoint,
    /// The checkpoint signature was absent or rejected by its trust anchor.
    UnauthenticatedCheckpoint,
    /// The inclusion proof does not fold to the checkpoint root.
    BadInclusionProof,
    /// The signature/leaf was present in the revocation feed.
    Revoked,
}

/// Authenticate a checkpoint before its root is used for proof validation.
///
/// The non-empty tree and signature checks are performed before calling the
/// verifier, avoiding accidental acceptance by a lenient adapter. Any verifier
/// failure is indistinguishable from a forged signature to the caller.
pub fn authenticate_checkpoint<A: CheckpointAuthenticator + ?Sized>(
    checkpoint: &Checkpoint,
    authenticator: &A,
) -> Result<(), LogbookError> {
    if checkpoint.tree_size == 0 || checkpoint.signature.is_empty() {
        return Err(LogbookError::BadCheckpoint);
    }
    let message = checkpoint.signed_message();
    if authenticator.verify_checkpoint(&message, &checkpoint.signature) {
        Ok(())
    } else {
        Err(LogbookError::UnauthenticatedCheckpoint)
    }
}

/// Hashes the signed Cask signature block into a transparency-log leaf.
#[must_use]
pub fn signature_leaf(signature: &[u8]) -> Hash {
    merkle::leaf_hash(signature)
}

/// Verify that a signature is logged and not revoked under `checkpoint`.
pub fn verify_inclusion<A: CheckpointAuthenticator + ?Sized>(
    leaf_hash: &Hash,
    checkpoint: &Checkpoint,
    proof: &InclusionProof,
    revocations: &[Revocation],
    authenticator: &A,
) -> Result<(), LogbookError> {
    authenticate_checkpoint(checkpoint, authenticator)?;
    if proof.leaf_index >= checkpoint.tree_size {
        return Err(LogbookError::BadCheckpoint);
    }
    if revocations
        .iter()
        .any(|r| merkle::eq(&r.leaf_hash, leaf_hash))
    {
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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec;

    struct ExactAnchor {
        message: [u8; CHECKPOINT_MESSAGE_LEN],
        signature: &'static [u8],
    }

    impl CheckpointAuthenticator for ExactAnchor {
        fn verify_checkpoint(&self, message: &[u8], signature: &[u8]) -> bool {
            message == self.message && signature == self.signature
        }
    }

    fn checkpoint(root_hash: Hash) -> Checkpoint {
        Checkpoint {
            log_id: [0xA5; 32],
            tree_size: 1,
            root_hash,
            signature: vec![0xC0, 0xFF, 0xEE],
        }
    }

    #[test]
    fn signed_message_binds_context_log_size_and_root() {
        let root = [0x11; 32];
        let checkpoint = checkpoint(root);
        let message = checkpoint.signed_message();
        assert_eq!(&message[..CHECKPOINT_CONTEXT.len()], &CHECKPOINT_CONTEXT);
        assert_eq!(
            &message[CHECKPOINT_CONTEXT.len()..CHECKPOINT_CONTEXT.len() + 32],
            &[0xA5; 32]
        );
        assert_eq!(
            &message[CHECKPOINT_CONTEXT.len() + 32..CHECKPOINT_CONTEXT.len() + 40],
            &1u64.to_be_bytes()
        );
        assert_eq!(&message[CHECKPOINT_CONTEXT.len() + 40..], &root);
    }

    #[test]
    fn checkpoint_authentication_rejects_empty_and_forged_signatures() {
        let root = [0x22; 32];
        let mut checkpoint = checkpoint(root);
        let anchor = ExactAnchor {
            message: checkpoint.signed_message(),
            signature: b"\xC0\xFF\xEE",
        };
        assert_eq!(authenticate_checkpoint(&checkpoint, &anchor), Ok(()));

        checkpoint.root_hash[0] ^= 1;
        assert_eq!(
            authenticate_checkpoint(&checkpoint, &anchor),
            Err(LogbookError::UnauthenticatedCheckpoint)
        );

        checkpoint.signature.clear();
        assert_eq!(
            authenticate_checkpoint(&checkpoint, &anchor),
            Err(LogbookError::BadCheckpoint)
        );
    }

    #[test]
    fn inclusion_requires_authenticated_checkpoint_before_root_is_trusted() {
        let leaf = signature_leaf(b"signed-cask");
        let checkpoint = checkpoint(leaf);
        let anchor = ExactAnchor {
            message: checkpoint.signed_message(),
            signature: b"\xC0\xFF\xEE",
        };
        let proof = InclusionProof {
            leaf_index: 0,
            path: vec![],
        };
        assert_eq!(
            verify_inclusion(&leaf, &checkpoint, &proof, &[], &anchor),
            Ok(())
        );

        let forged = ExactAnchor {
            message: [0; CHECKPOINT_MESSAGE_LEN],
            signature: b"\xC0\xFF\xEE",
        };
        assert_eq!(
            verify_inclusion(&leaf, &checkpoint, &proof, &[], &forged),
            Err(LogbookError::UnauthenticatedCheckpoint)
        );
    }
}
