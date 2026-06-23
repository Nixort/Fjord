// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Authentication factors. Policy requires N-of-M by sensitivity.
//!
//! TODO(harbormaster): implement verifiers; biometrics match on-chip only.
#[derive(Clone, Copy, Debug)]
pub enum FactorKind {
    /// FIDO2/WebAuthn passkey or hardware token (phishing-resistant).
    Passkey,
    /// Passphrase stretched with Argon2id.
    Passphrase,
    /// On-chip biometric match (template never leaves the secure element).
    Biometric,
    /// DICE/TPM device attestation (the device authenticates itself).
    DeviceAttestation,
}
