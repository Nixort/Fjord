// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Interrupt controller abstraction (APIC on x86_64, GIC on aarch64).
//! TODO(hull): mask/unmask, EOI, route IRQs to userspace driver caps.
