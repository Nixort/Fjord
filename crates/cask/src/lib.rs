// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Cask — the tamper-evident executable format
//!
//! A `.cask` is a signed, integrity-protected, license-bearing container.
//! Guarantees:
//! * **Integrity** — BLAKE3 Merkle tree; pages verified lazily on fault.
//! * **Authenticity** — Ed25519 + ML-DSA hybrid signatures over the root.
//! * **Authorization** — an embedded license is a signed capability budget.
//! * **Anti-rollback** — a monotonic version counter (TPM NV / RPMB).
//! * **Transparency** — the signature must appear in [`logbook`].
//!
//! This crate is `no_std` so the loader can use it inside the kernel-adjacent
//! path; the host sealing side lives in `xtask/shipwright`.
//! See `docs/ARCHITECTURE.md` §5.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

pub mod format;
pub mod merkle;
pub mod verify;

/// Errors that make a Cask untrusted. Verification is fail-closed.
#[derive(Debug)]
pub enum CaskError {
    /// Container header/layout is malformed.
    Malformed,
    /// A Merkle page hash did not match (tampering).
    IntegrityFailed,
    /// Signature invalid or signer not a trusted anchor.
    BadSignature,
    /// Version is older than the recorded monotonic counter (rollback).
    Rollback,
    /// Signature is absent from the transparency log.
    NotInLogbook,
    /// Requested authority exceeds the license budget.
    LicenseExceeded,
}
