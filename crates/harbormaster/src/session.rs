//! Session minting and capability leases.
//!
//! On success: unseal KEK -> VK (Brine), mount volumes, mint a scoped session
//! CSpace. Leases are time-boxed and auto-revoke (least privilege over time);
//! sensitive requests trigger step-up re-authentication.
//! TODO(harbormaster): lease accounting, lockout via timed monotonic counter.
pub fn mint_session() { todo!("unlock flow + session CSpace") }
