// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Sealing/unsealing secrets to a PCR policy.
//!
//! TODO(anchor): bind Brine VKs and Cask anti-rollback counters to the
//! expected measured-boot state; refuse to unseal on mismatch (evil-maid).
