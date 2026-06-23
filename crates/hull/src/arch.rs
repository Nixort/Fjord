//! Per-architecture backends (CPU init, registers, context switch).
//!
//! TODO(hull): `#[cfg(target_arch = ...)]` modules for x86_64 and aarch64,
//! each exposing the same trait-shaped API to the rest of Hull.
