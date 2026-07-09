// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Lading — the signed bill-of-lading manifest
//!
//! The Lading is the declarative, signed description embedded in a [`cask`]:
//! identity, version, required capabilities, and the license budget. It is the
//! source of truth Helm intersects against at launch. See `ARCHITECTURE.md` §4-5.
#![no_std]
#![allow(dead_code)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

/// Errors raised while validating or intersecting a manifest and license.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LadingError {
    /// Package identity is empty or not reverse-DNS-like enough for policy.
    BadIdentity,
    /// Version zero is reserved and cannot be launched.
    BadVersion,
    /// A capability selector is empty or contains a control byte.
    BadCapability,
    /// A requested capability is outside the license budget or delegation.
    CapabilityDenied,
}

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

impl Manifest {
    /// Performs syntax and minimum anti-rollback sanity checks.
    pub fn validate(&self) -> Result<(), LadingError> {
        if self.id.is_empty() || !self.id.contains('.') {
            return Err(LadingError::BadIdentity);
        }
        if self.version == 0 {
            return Err(LadingError::BadVersion);
        }
        for cap in &self.requested_caps {
            cap.validate()?;
        }
        Ok(())
    }
}

/// A single requested authority (e.g. "net.connect:443", "fs.read:/etc").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapRequest(pub String);

impl CapRequest {
    /// Constructs and validates a capability request string.
    pub fn new(value: String) -> Result<Self, LadingError> {
        let request = Self(value);
        request.validate()?;
        Ok(request)
    }

    /// Returns the selector as UTF-8 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Validates the selector grammar used by the launch policy.
    pub fn validate(&self) -> Result<(), LadingError> {
        let s = self.as_str();
        if s.is_empty() || s.bytes().any(|b| b <= 0x20 || b == 0x7F) {
            return Err(LadingError::BadCapability);
        }
        if !s.contains(':') {
            return Err(LadingError::BadCapability);
        }
        Ok(())
    }

    /// Returns true if this selector authorizes `requested`.
    ///
    /// Exact selectors match one authority. Selectors ending in `:*` grant all
    /// resources under the same operation prefix, e.g. `net.connect:*` covers
    /// `net.connect:443` but not `net.listen:443`.
    #[must_use]
    pub fn allows(&self, requested: &CapRequest) -> bool {
        let allowed = self.as_str();
        let requested = requested.as_str();
        if allowed == requested {
            return true;
        }
        match allowed.strip_suffix("*") {
            Some(prefix) => requested.starts_with(prefix),
            None => false,
        }
    }
}

/// The license budget: the maximum authority the publisher is licensed to ship.
#[derive(Debug)]
pub struct License {
    /// Upper bound on granted capabilities.
    pub allowed_caps: Vec<CapRequest>,
}

impl License {
    /// Validates every selector in the license budget.
    pub fn validate(&self) -> Result<(), LadingError> {
        for cap in &self.allowed_caps {
            cap.validate()?;
        }
        Ok(())
    }

    /// Returns true if the license covers `requested`.
    #[must_use]
    pub fn allows(&self, requested: &CapRequest) -> bool {
        self.allowed_caps.iter().any(|allowed| allowed.allows(requested))
    }
}

/// The result Helm should use to mint the launched task's CSpace.
#[derive(Debug)]
pub struct EffectiveAuthority {
    /// Capability selectors that survived manifest ∩ license ∩ delegation.
    pub granted_caps: Vec<CapRequest>,
}

/// Intersects manifest requests with the publisher license and delegated budget.
pub fn derive_effective_authority(
    manifest: &Manifest,
    license: &License,
    delegated: &[CapRequest],
) -> Result<EffectiveAuthority, LadingError> {
    manifest.validate()?;
    license.validate()?;
    for cap in delegated {
        cap.validate()?;
    }

    let mut granted_caps = Vec::new();
    for requested in &manifest.requested_caps {
        let licensed = license.allows(requested);
        let delegated = delegated.iter().any(|cap| cap.allows(requested));
        if !licensed || !delegated {
            return Err(LadingError::CapabilityDenied);
        }
        granted_caps.push(requested.clone());
    }
    Ok(EffectiveAuthority { granted_caps })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn wildcard_budget_intersects() {
        let manifest = Manifest {
            id: "os.fjord.shell".to_string(),
            version: 1,
            requested_caps: vec![CapRequest::new("net.connect:443".to_string()).unwrap()],
        };
        let license = License {
            allowed_caps: vec![CapRequest::new("net.connect:*".to_string()).unwrap()],
        };
        let delegated = [CapRequest::new("net.connect:*".to_string()).unwrap()];
        let effective = derive_effective_authority(&manifest, &license, &delegated).unwrap();
        assert_eq!(effective.granted_caps.len(), 1);
    }

    #[test]
    fn unlicensed_request_is_denied() {
        let manifest = Manifest {
            id: "os.fjord.shell".to_string(),
            version: 1,
            requested_caps: vec![CapRequest::new("fs.write:/etc".to_string()).unwrap()],
        };
        let license = License {
            allowed_caps: vec![CapRequest::new("fs.read:*".to_string()).unwrap()],
        };
        let delegated = [CapRequest::new("fs.write:*".to_string()).unwrap()];
        assert!(matches!(
            derive_effective_authority(&manifest, &license, &delegated),
            Err(LadingError::CapabilityDenied)
        ));
    }
}
