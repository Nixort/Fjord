//! The AEAD data path.
//!
//! Nonce discipline (the classic FDE failure): a deterministic counter
//! `(object_id || block_index)` under a unique per-object key, or a random
//! 192-bit XChaCha nonce. Never a shared key with a random 96-bit nonce on a
//! large volume. TODO(brine): fuse decrypt-on-fault with lazy Merkle verify.
pub fn seal_block() { todo!("encrypt + authenticate a block") }
pub fn open_block() { todo!("verify + decrypt a block") }
