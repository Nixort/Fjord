// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Full verification pipeline used by `helm::launch`.
//!
//! Order (fail-closed): parse -> check anti-rollback -> verify signatures
//! against trust-anchor capabilities -> check Logbook inclusion -> return the
//! license budget for Helm to intersect. Page integrity is then enforced
//! lazily at runtime via `merkle::verify_page`.
use crate::CaskError;

/// Verify everything except per-page integrity (which is lazy).
/// TODO(cask): implement; see ARCHITECTURE §5.
pub fn verify() -> Result<(), CaskError> {
    todo!("Cask verification pipeline")
}
