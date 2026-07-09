// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Session minting and capability leases.
//!
//! On success: unseal KEK -> VK (Brine), mount volumes, mint a scoped session
//! CSpace. Leases are time-boxed and auto-revoke (least privilege over time);
//! sensitive requests trigger step-up re-authentication.

use crate::factors::VerifiedFactor;

/// Local session identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionId(pub u64);

/// Unlock policy for minting a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionPolicy {
    /// Minimum number of distinct verified factors.
    pub min_factors: usize,
    /// Minimum assurance point total.
    pub min_assurance_points: u8,
    /// Whether at least one phishing-resistant factor is required.
    pub require_phishing_resistant: bool,
    /// Maximum factor age in monotonic ticks.
    pub max_factor_age: u64,
    /// Session lease duration in monotonic ticks.
    pub lease_ticks: u64,
}

impl SessionPolicy {
    /// Balanced default for unlocking user storage.
    pub const DEFAULT: Self = Self {
        min_factors: 2,
        min_assurance_points: 4,
        require_phishing_resistant: true,
        max_factor_age: 30_000,
        lease_ticks: 300_000,
    };
}

/// A minted scoped session lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    /// Session id derived by caller from identity and counter state.
    pub id: SessionId,
    /// Monotonic expiry tick.
    pub expires_at: u64,
    /// Assurance points that justified the session.
    pub assurance_points: u8,
}

impl Session {
    /// Returns true if the lease is expired at `now`.
    #[must_use]
    pub const fn is_expired(self, now: u64) -> bool {
        now >= self.expires_at
    }
}

/// Why session minting was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// Not enough fresh factors were presented.
    InsufficientFactors,
    /// Assurance point total was below policy.
    InsufficientAssurance,
    /// Policy requires a phishing-resistant factor.
    MissingPhishingResistantFactor,
    /// Lease arithmetic overflowed.
    TimeOverflow,
}

/// Mint a scoped session if the presented factors satisfy `policy`.
pub fn mint_session(
    id: SessionId,
    policy: SessionPolicy,
    factors: &[VerifiedFactor],
    now: u64,
) -> Result<Session, SessionError> {
    let mut fresh = 0usize;
    let mut assurance = 0u8;
    let mut has_phishing_resistant = false;

    for factor in factors {
        if now.saturating_sub(factor.verified_at) > policy.max_factor_age {
            continue;
        }
        fresh += 1;
        assurance = assurance.saturating_add(factor.kind.assurance_points());
        has_phishing_resistant |= factor.kind.phishing_resistant();
    }

    if fresh < policy.min_factors {
        return Err(SessionError::InsufficientFactors);
    }
    if assurance < policy.min_assurance_points {
        return Err(SessionError::InsufficientAssurance);
    }
    if policy.require_phishing_resistant && !has_phishing_resistant {
        return Err(SessionError::MissingPhishingResistantFactor);
    }

    let expires_at = now
        .checked_add(policy.lease_ticks)
        .ok_or(SessionError::TimeOverflow)?;
    Ok(Session {
        id,
        expires_at,
        assurance_points: assurance,
    })
}
