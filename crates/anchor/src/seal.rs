// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Sealing/unsealing secrets to a PCR policy.
//!
//! Anchor does not fake a TPM in software. It models the policy digest and the
//! sealed-object handle, then fails closed unless the caller presents the exact
//! measured-boot state that was authorized.

use crate::measure::Digest;

/// A PCR policy digest accepted for unseal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcrPolicy {
    /// Expected measured-boot accumulator.
    pub expected_pcr: Digest,
}

/// Opaque handle to a secret sealed by hardware firmware/TPM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SealedHandle {
    /// Platform-specific TPM/NV/secure-element handle.
    pub handle: u64,
    /// Policy that must match before the handle may be unsealed.
    pub policy: PcrPolicy,
}

/// Why a sealed object cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// The current measured boot state differs from the policy.
    PolicyMismatch,
    /// Zero is reserved for "no sealed object".
    InvalidHandle,
}

/// Creates an opaque sealed-object descriptor.
pub fn bind_handle(handle: u64, policy: PcrPolicy) -> Result<SealedHandle, SealError> {
    if handle == 0 {
        return Err(SealError::InvalidHandle);
    }
    Ok(SealedHandle { handle, policy })
}

/// Checks whether a sealed handle is usable under `current_pcr`.
///
/// A successful return authorizes the platform TPM/secure-element call; it does
/// not synthesize secret bytes in software.
pub fn authorize_unseal(handle: SealedHandle, current_pcr: &Digest) -> Result<u64, SealError> {
    if handle.handle == 0 {
        return Err(SealError::InvalidHandle);
    }
    if &handle.policy.expected_pcr != current_pcr {
        return Err(SealError::PolicyMismatch);
    }
    Ok(handle.handle)
}
