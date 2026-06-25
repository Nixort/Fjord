// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Brine — storage encryption
//!
//! Authenticated encryption for data at rest: AEAD for confidentiality and the
//! filesystem BLAKE3 Merkle tree for integrity, in a single pass. An envelope
//! key hierarchy (DEK -> VK -> KEK) enables instant rekey and crypto-erase;
//! volume keys are sealed to the [`anchor`] measured-boot state. Keys live in
//! `cryptd` — callers get operations, never raw key material.
//! See `docs/ARCHITECTURE.md` §6.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

pub mod keys;
pub mod aead;

/// AEAD cipher selection. XChaCha20 in software, AES-256-GCM with AES-NI.
#[derive(Clone, Copy, Debug)]
pub enum Cipher {
    /// 256-bit key, 192-bit nonce — safe with random nonces, constant-time SW.
    XChaCha20Poly1305,
    /// Hardware-accelerated AEAD where AES-NI / ARMv8-CE is available.
    Aes256Gcm,
}
