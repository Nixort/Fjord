// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Authentication factors. Policy requires N-of-M by sensitivity.

/// Kind of authentication factor presented by a user or device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl FactorKind {
    /// Relative assurance points used by the local unlock policy.
    #[must_use]
    pub const fn assurance_points(self) -> u8 {
        match self {
            Self::Passkey => 3,
            Self::Passphrase => 2,
            Self::Biometric => 1,
            Self::DeviceAttestation => 2,
        }
    }

    /// Returns true if this factor is phishing-resistant.
    #[must_use]
    pub const fn phishing_resistant(self) -> bool {
        matches!(self, Self::Passkey | Self::DeviceAttestation)
    }
}

/// One verified factor, carrying a monotonic freshness timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifiedFactor {
    /// Factor kind.
    pub kind: FactorKind,
    /// Monotonic tick at which verification completed.
    pub verified_at: u64,
}

impl VerifiedFactor {
    /// Creates a verified factor record.
    #[must_use]
    pub const fn new(kind: FactorKind, verified_at: u64) -> Self {
        Self { kind, verified_at }
    }
}
