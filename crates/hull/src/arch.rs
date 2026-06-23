// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Per-architecture backends (CPU init, registers, context switch).
//!
//! TODO(hull): `#[cfg(target_arch = ...)]` modules for x86_64 and aarch64,
//! each exposing the same trait-shaped API to the rest of Hull.
