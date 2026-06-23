// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Virtual address spaces and page-table management.
//!
//! Enforces W^X (a page is never simultaneously writable and executable) and
//! exposes map/unmap as capability-mediated operations.

/// Architecture-independent page-table handle.
pub struct VSpace; // TODO(keel): wrap arch page tables from `hull`.

impl VSpace {
    /// Map a frame capability into this address space.
    /// TODO(keel): validate W^X, flush TLB (ASID/PCID aware).
    pub fn map(&mut self) { todo!("map frame") }
    /// Unmap a previously mapped frame.
    pub fn unmap(&mut self) { todo!("unmap frame") }
}
