// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Harbormaster — authentication + authorization
//!
//! Bridges human/device authentication and the object-capability model.
//! Multi-factor auth establishes a principal; Harbormaster then unseals the
//! [`brine`] volume keys and mints a scoped session CSpace with time-boxed
//! capability leases. There is no ambient root. See `ARCHITECTURE.md` §7.
#![no_std]
#![allow(dead_code)]
extern crate alloc;
use alloc::vec::Vec;

pub mod factors;
pub mod session;

/// Outcome of an authentication attempt.
pub enum AuthOutcome {
    /// Enough factors satisfied for the requested sensitivity.
    Granted,
    /// Insufficient/invalid factors — disk stays encrypted.
    Denied,
    /// More factors required (step-up).
    StepUp(Vec<factors::FactorKind>),
}
