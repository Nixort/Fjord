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
//! large volume. TODO(brine): fuse decrypt-on-fault with lazy Merkle verify.
pub fn seal_block() { todo!("encrypt + authenticate a block") }
pub fn open_block() { todo!("verify + decrypt a block") }
