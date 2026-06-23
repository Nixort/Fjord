// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! # Anchor — secure boot + DICE root of trust
//!
//! Anchor measures every boot stage into TPM PCRs and derives a layered DICE
//! identity. Keys (including Brine volume keys) are *sealed* to the expected
//! measurements, so a tampered chain cannot unseal them. This is the anchor of
//! the end-to-end chain of trust. See `docs/ARCHITECTURE.md` §3.
#![no_std]
#![allow(dead_code)]

pub mod measure;
pub mod seal;

/// Measure the next stage, extend the PCR, then transfer control.
///
/// TODO(anchor): hash image with BLAKE3, `TPM2_PCR_Extend`, derive next CDI.
pub fn measure_and_launch_next() -> ! {
    todo!("measured boot handoff — ROADMAP Phase 3")
}
