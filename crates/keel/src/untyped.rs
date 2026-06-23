//! Untyped memory and the retype model.
//!
//! All kernel objects are carved out of untyped memory by an explicit retype
//! operation, so kernel memory is accounted to whoever holds the untyped cap.
//! This removes implicit kernel heaps from the TCB (seL4 model).

/// A region of untyped (as-yet-unstructured) physical memory.
pub struct Untyped; // TODO(keel): track watermark + children for revoke.

/// Retype untyped memory into typed kernel objects.
/// TODO(keel): size/alignment checks; record derivation for revoke.
pub fn retype(_src: &Untyped) { todo!("retype untyped -> object") }
