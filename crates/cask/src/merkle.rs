//! BLAKE3 Merkle tree over page-sized leaves (fs-verity style).
//! TODO(cask): build tree (host), verify a single page against the root
//! (loader, on page fault) without hashing the whole image.
pub fn verify_page() { todo!("lazy page verification") }
