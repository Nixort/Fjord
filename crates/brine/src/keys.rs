// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Envelope key hierarchy.
//!
//! ```text
//! KEK  (from Harbormaster MFA + Anchor/TPM seal)
//!  |__ VK   (per-volume; rotate = rewrap, no data re-encryption)
//!       |__ DEK   (per-object; enables fine-grained crypto-erase)
//! ```
//!
//! This slice implements the key-state model and zeroization boundary. Actual
//! AES-KW/XChaCha20-Poly1305 backends stay fail-closed in [`crate::aead`] until
//! an audited primitive is connected.

use core::ptr::write_volatile;

/// Length of every symmetric key in Brine.
pub const KEY_LEN: usize = 32;

fn wipe(bytes: &mut [u8; KEY_LEN]) {
    for byte in bytes.iter_mut() {
        // SAFETY: writing a byte through its own mutable reference is valid; the
        // volatile write prevents the wipe from being optimized away.
        unsafe { write_volatile(byte, 0) };
    }
}

/// Per-object data-encryption key.
pub struct DataKey([u8; KEY_LEN]);

/// Per-volume wrapping key.
pub struct VolumeKey([u8; KEY_LEN]);

/// Key-encryption key derived from auth + hardware state.
pub struct KeyEncryptionKey([u8; KEY_LEN]);

macro_rules! key_impl {
    ($name:ident) => {
        impl $name {
            /// Constructs a key from raw bytes owned by the caller.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
                Self(bytes)
            }

            /// Borrows the raw key material for backend code.
            #[must_use]
            pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }

            /// Explicitly zeroizes the key material.
            pub fn zeroize(&mut self) {
                wipe(&mut self.0);
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                self.zeroize();
            }
        }
    };
}

key_impl!(DataKey);
key_impl!(VolumeKey);
key_impl!(KeyEncryptionKey);

/// Destroy a volume key, making all its ciphertext permanently unrecoverable.
pub fn crypto_erase(mut vk: VolumeKey) {
    vk.zeroize();
}
