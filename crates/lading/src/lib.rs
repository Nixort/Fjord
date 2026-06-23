//! # Lading — the signed bill-of-lading manifest
//!
//! The Lading is the declarative, signed description embedded in a [`cask`]:
//! identity, version, required capabilities, and the license budget. It is the
//! source of truth Helm intersects against at launch. See `ARCHITECTURE.md` §4-5.
#![no_std]
#![allow(dead_code)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

/// The declared identity + requirements of a Cask.
#[derive(Debug)]
pub struct Manifest {
    /// Reverse-DNS package identity, e.g. "os.fjord.shell".
    pub id: String,
    /// Monotonic version used for anti-rollback.
    pub version: u64,
    /// Capabilities the program requests (subject to license + delegation).
    pub requested_caps: Vec<CapRequest>,
}

/// A single requested authority (e.g. "net.connect:443", "fs.read:/etc").
#[derive(Debug)]
pub struct CapRequest(pub String);

/// The license budget: the maximum authority the publisher is licensed to ship.
// TODO(lading): expiry, hardware binding, tenant scoping; signed separately.
#[derive(Debug)]
pub struct License {
    /// Upper bound on granted capabilities.
    pub allowed_caps: Vec<CapRequest>,
}
