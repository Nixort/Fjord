//! Envelope key hierarchy.
//!
//! ```text
//! KEK  (from Harbormaster MFA + Anchor/TPM seal)
//!  |__ VK   (per-volume; rotate = rewrap, no data re-encryption)
//!       |__ DEK   (per-object; enables fine-grained crypto-erase)
//! ```
//! TODO(brine): wrap/unwrap (AES-256-KW), rotate VK, crypto-erase (drop VK).
pub struct DataKey;   // per-object DEK
pub struct VolumeKey;  // per-volume VK
pub struct KeyEncryptionKey; // KEK derived from auth + hardware

/// Destroy a volume key, making all its ciphertext permanently unrecoverable.
pub fn crypto_erase(_vk: VolumeKey) { todo!("crypto-shredding") }
