// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Capabilities and capability spaces (CSpace).
//!
//! Every kernel object is named by an unforgeable capability. Authority is
//! delegated by minting derived capabilities; revocation walks a derivation
//! tree. There is no ambient authority anywhere in the system.
//!
//! See `docs/ARCHITECTURE.md` §1 and `docs/GLOSSARY.md`.

/// Rights carried by a capability (subset granted to a delegate).
// TODO(keel): model as bitflags (read/write/grant/execute/seal).
#[derive(Clone, Copy, Debug)]
pub struct Rights(pub u32);

/// A typed reference to a kernel object plus its [`Rights`].
// TODO(keel): tagged union over Untyped/Page/Endpoint/Notification/TCB/IRQ.
#[derive(Clone, Copy, Debug)]
pub struct Capability {
    /// Object identity (kernel-internal handle).
    pub object: u64,
    /// Rights granted to the holder.
    pub rights: Rights,
}

/// Mint a derived capability with a (necessarily) reduced rights set.
///
/// TODO(keel): enforce monotonic rights reduction; record in derivation tree.
pub fn mint(_parent: &Capability, _new_rights: Rights) -> Capability {
    todo!("capability derivation")
}

/// Revoke a capability and all capabilities derived from it.
///
/// TODO(keel): recursive revocation over the derivation tree (seL4 CDT).
pub fn revoke(_cap: &Capability) {
    todo!("recursive revoke")
}
