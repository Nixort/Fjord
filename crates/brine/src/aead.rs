// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! The AEAD data path.
//!
//! Nonce discipline (the classic FDE failure): a deterministic counter
//! `(object_id || block_index)` under a unique per-object key, or a random
//! 192-bit XChaCha nonce. Never a shared key with a random 96-bit nonce on a
//! large volume. This module now exposes the validated block protocol and nonce
//! construction while returning fail-closed until an audited cipher backend is
//! linked.

use crate::keys::DataKey;

/// XChaCha-style nonce length.
pub const NONCE_LEN: usize = 24;
/// Authentication tag length.
pub const TAG_LEN: usize = 32;

/// Block nonce: object id, block index, and an 8-byte domain separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nonce(pub [u8; NONCE_LEN]);

/// Detached authentication tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag(pub [u8; TAG_LEN]);

/// Associated data bound to every encrypted block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockAad {
    /// Stable object identifier.
    pub object_id: u64,
    /// Block index within the object.
    pub block_index: u64,
    /// Monotonic object generation to prevent stale-block replay.
    pub generation: u64,
}

/// Why a Brine AEAD request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AeadError {
    /// Output buffer length must match input buffer length.
    BadBufferLength,
    /// A cipher backend has not been wired in yet; never treat as success.
    BackendUnavailable,
}

/// Deterministically derives the per-block nonce.
#[must_use]
pub fn derive_nonce(object_id: u64, block_index: u64) -> Nonce {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[0..8].copy_from_slice(&object_id.to_le_bytes());
    nonce[8..16].copy_from_slice(&block_index.to_le_bytes());
    nonce[16..24].copy_from_slice(b"fjordv1\0");
    Nonce(nonce)
}

/// Encrypts and authenticates one block.
///
/// The protocol validation is implemented, but the actual primitive is not
/// stubbed as success. This preserves Brine's fail-closed invariant until the
/// audited AEAD backend lands.
pub fn seal_block(
    key: &DataKey,
    nonce: Nonce,
    aad: BlockAad,
    plaintext: &[u8],
    ciphertext_out: &mut [u8],
) -> Result<Tag, AeadError> {
    let _ = (key, nonce, aad);
    if plaintext.len() != ciphertext_out.len() {
        return Err(AeadError::BadBufferLength);
    }
    Err(AeadError::BackendUnavailable)
}

/// Verifies and decrypts one block.
pub fn open_block(
    key: &DataKey,
    nonce: Nonce,
    aad: BlockAad,
    ciphertext: &[u8],
    tag: Tag,
    plaintext_out: &mut [u8],
) -> Result<(), AeadError> {
    let _ = (key, nonce, aad, tag);
    if ciphertext.len() != plaintext_out.len() {
        return Err(AeadError::BadBufferLength);
    }
    Err(AeadError::BackendUnavailable)
}
