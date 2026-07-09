// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 28 june 2026

//! The Cask verification pipeline used by the loader and `helm::launch`.
//!
//! Full fail-closed order (`docs/ARCHITECTURE.md` §5):
//!
//! 1. **parse** the container ([`format::Cask::parse`]) — *implemented*;
//! 2. **anti-rollback**: reject a version below the monotonic counter — *Phase 3*;
//! 3. **authenticity**: Ed25519 + ML-DSA over (Lading ‖ Merkle root) against a
//!    trust-anchor capability — *Phase 3*;
//! 4. **transparency**: a Logbook inclusion proof — *Phase 3*;
//! 5. **integrity**: the BLAKE3 Merkle root, then *lazy* per-page verification
//!    on fault — *implemented*.
//!
//! This module delivers the Phase-2 MVP: steps 1 and 5. The authenticity,
//! rollback, and transparency steps are intentionally **not** stubbed to return
//! success — they are simply absent until `harbormaster`, `anchor`, and
//! `logbook` land, at which point Helm composes them ahead of integrity. A
//! caller must therefore not treat [`open_and_verify`] as proof of authenticity;
//! it proves the image is well-formed and its contents match the sealed root.

use crate::format::Cask;
use crate::merkle::{self, MerkleTree, ProofStep};
use crate::CaskError;
use alloc::vec::Vec;

/// Parses `image` and verifies its contents against the sealed Merkle root.
///
/// This is the eager integrity gate (steps 1 + 5): it rebuilds the tree over
/// every body page and checks the root, so it needs an allocator and is meant
/// for the host sealer and for Helm's pre-flight, not the per-fault fast path.
/// Returns the validated [`Cask`] view borrowing `image`.
pub fn open_and_verify(image: &[u8]) -> Result<Cask<'_>, CaskError> {
    let cask = Cask::parse(image)?;
    verify_integrity(&cask)?;
    Ok(cask)
}

/// Rebuilds the Merkle tree over all body pages and compares it to the header
/// root, returning [`CaskError::IntegrityFailed`] on any mismatch.
pub fn verify_integrity(cask: &Cask<'_>) -> Result<(), CaskError> {
    let count = cask.page_count();
    let mut pages: Vec<&[u8]> = Vec::with_capacity(count as usize);
    for i in 0..count {
        pages.push(cask.page(i).ok_or(CaskError::Malformed)?);
    }
    let tree = MerkleTree::build(&pages).ok_or(CaskError::Malformed)?;
    if merkle::eq(&tree.root(), cask.merkle_root()) {
        Ok(())
    } else {
        Err(CaskError::IntegrityFailed)
    }
}

/// Verifies a single body page lazily against the sealed root.
///
/// This is the loader's on-fault path: no allocation, no whole-image hashing —
/// just the faulting page, its inclusion `proof`, and the trusted header root.
/// Returns the verified page bytes so the caller can map them.
pub fn verify_page<'a>(
    cask: &Cask<'a>,
    index: u64,
    proof: &[ProofStep],
) -> Result<&'a [u8], CaskError> {
    let page = cask.page(index).ok_or(CaskError::Malformed)?;
    if merkle::verify_page_with_count(index, cask.page_count(), page, proof, cask.merkle_root()) {
        Ok(page)
    } else {
        Err(CaskError::IntegrityFailed)
    }
}

/// Boot-time self-test (matches the per-crate `selftest()` convention).
///
/// Heap-free end-to-end exercise of the loader path: it assembles a tiny
/// two-page Cask in a stack buffer, parses it, lazily verifies a page against a
/// hand-built proof, and confirms a flipped byte is rejected. No allocator is
/// touched, so it is safe to run from the early boot path.
pub fn selftest() -> Result<(), CaskError> {
    use crate::format::{Header, Section, FORMAT_VERSION, HEADER_LEN};

    const PAGE: u32 = 4;
    const BODY: usize = 8; // two 4-byte pages
    let page0: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
    let page1: [u8; 4] = [0x55, 0x66, 0x77, 0x88];

    // Root of a two-leaf tree is the parent of the two leaf hashes.
    let root = merkle::parent_hash(&merkle::leaf_hash(&page0), &merkle::leaf_hash(&page1));

    let body_off = HEADER_LEN as u64;
    let tail = body_off + BODY as u64;
    let header = Header {
        format_version: FORMAT_VERSION,
        flags: 0,
        page_size: PAGE,
        page_count: 2,
        merkle_root: root,
        lading: Section {
            offset: tail,
            len: 0,
        },
        body: Section {
            offset: body_off,
            len: BODY as u64,
        },
        signature: Section {
            offset: tail,
            len: 0,
        },
        logbook: Section {
            offset: tail,
            len: 0,
        },
    };

    let mut image = [0u8; HEADER_LEN + BODY];
    image[..HEADER_LEN].copy_from_slice(&header.encode());
    image[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&page0);
    image[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&page1);

    let cask = Cask::parse(&image)?;

    // Page 0's sibling is page 1's leaf, sitting on the right.
    let proof = [ProofStep {
        sibling: merkle::leaf_hash(&page1),
        side: merkle::Side::Right,
    }];

    let verified = verify_page(&cask, 0, &proof)?;
    if verified != page0 {
        return Err(CaskError::IntegrityFailed);
    }

    // A tampered page must be rejected against the same proof and root.
    let mut tampered = image;
    tampered[HEADER_LEN] ^= 0x01;
    let bad = Cask::parse(&tampered)?;
    match verify_page(&bad, 0, &proof) {
        Err(CaskError::IntegrityFailed) => {}
        _ => return Err(CaskError::IntegrityFailed),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::format::{Header, Section, FORMAT_VERSION, HEADER_LEN};
    use std::vec::Vec;

    const PAGE: u32 = 64;

    fn build_image(n: usize) -> (Vec<u8>, MerkleTree, Vec<Vec<u8>>) {
        let pages: Vec<Vec<u8>> = (0..n)
            .map(|i| (0..PAGE).map(|j| (i as u32 * 3 + j) as u8).collect())
            .collect();
        let refs: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();
        let tree = MerkleTree::build(&refs).unwrap();

        let mut body = Vec::new();
        for p in &pages {
            body.extend_from_slice(p);
        }
        let body_off = HEADER_LEN as u64;
        let tail = body_off + body.len() as u64;
        let header = Header {
            format_version: FORMAT_VERSION,
            flags: 0,
            page_size: PAGE,
            page_count: n as u64,
            merkle_root: tree.root(),
            lading: Section {
                offset: tail,
                len: 0,
            },
            body: Section {
                offset: body_off,
                len: body.len() as u64,
            },
            signature: Section {
                offset: tail,
                len: 0,
            },
            logbook: Section {
                offset: tail,
                len: 0,
            },
        };
        let mut image = Vec::new();
        image.extend_from_slice(&header.encode());
        image.extend_from_slice(&body);
        (image, tree, pages)
    }

    #[test]
    fn open_and_verify_accepts_good_image() {
        let (image, _, _) = build_image(9);
        assert!(open_and_verify(&image).is_ok());
    }

    #[test]
    fn open_and_verify_rejects_corrupt_body() {
        let (mut image, _, _) = build_image(9);
        let body_start = HEADER_LEN;
        image[body_start] ^= 0x80;
        assert!(matches!(
            open_and_verify(&image),
            Err(CaskError::IntegrityFailed)
        ));
    }

    #[test]
    fn lazy_verify_page_round_trips_every_page() {
        let (image, tree, pages) = build_image(13);
        let cask = Cask::parse(&image).unwrap();
        for i in 0..pages.len() {
            let proof = tree.proof(i).unwrap();
            let got = verify_page(&cask, i as u64, &proof).unwrap();
            assert_eq!(got, pages[i].as_slice());
        }
    }

    #[test]
    fn lazy_verify_page_rejects_wrong_proof() {
        let (image, tree, _) = build_image(13);
        let cask = Cask::parse(&image).unwrap();
        let proof_for_2 = tree.proof(2).unwrap();
        assert!(matches!(
            verify_page(&cask, 7, &proof_for_2),
            Err(CaskError::IntegrityFailed)
        ));
    }

    #[test]
    fn selftest_passes() {
        selftest().unwrap();
    }
}
