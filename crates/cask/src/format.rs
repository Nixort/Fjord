//! On-disk container layout.
//!
//! ```text
//! +----------------+-------------------------------------------------+
//! | Header         | magic "CASK", version, flags, section table      |
//! | Lading         | signed manifest (see crate `lading`)            |
//! | Merkle root    | BLAKE3 root over the code/data pages            |
//! | Code/Data      | W^X segments, page-aligned for lazy verification |
//! | Signatures     | Ed25519 + ML-DSA over (Lading || Merkle root)   |
//! | Logbook proof  | inclusion proof + log checkpoint                |
//! +----------------+-------------------------------------------------+
//! ```
//! TODO(cask): zero-copy parser with strict bounds checks (fuzz target).
pub const MAGIC: [u8; 4] = *b"CASK";
