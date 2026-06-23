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
