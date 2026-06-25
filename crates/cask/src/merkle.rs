// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! BLAKE3 Merkle tree over page-sized leaves (fs-verity style).
//! TODO(cask): build tree (host), verify a single page against the root
//! (loader, on page fault) without hashing the whole image.
pub fn verify_page() { todo!("lazy page verification") }
