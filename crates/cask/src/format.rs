// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 28 june 2026

//! On-disk container layout and its zero-copy parser.
//!
//! ```text
//! +----------------+-------------------------------------------------+
//! | Header         | magic "CASK", version, flags, section table     |
//! | Lading         | signed manifest (see crate `lading`)            |
//! | Code/Data      | W^X segments, page-aligned for lazy verification|
//! | Signatures     | Ed25519 + ML-DSA over (Lading || Merkle root)   |
//! | Logbook proof  | inclusion proof + log checkpoint                |
//! +----------------+-------------------------------------------------+
//! ```
//!
//! The [`Header`] is a fixed 120-byte little-endian record. Every offset/length
//! pair in it ("the section table") is validated against the image bounds at
//! parse time with checked arithmetic, so a malicious or truncated container can
//! never make an accessor read out of bounds. [`Cask::parse`] borrows the input
//! and hands back slices into it — no copying, no allocation — which makes this
//! the natural `cargo-fuzz` target (`docs/ROADMAP.md`, Phase 6).
//!
//! The Merkle root lives *in the header* (not as a separate section) so it is
//! covered by the fixed-layout bounds check and is cheap to read on the loader
//! fast path. See `docs/ARCHITECTURE.md` §5.

use crate::merkle::Hash;
use crate::CaskError;

/// Container magic: the ASCII bytes `CASK`.
pub const MAGIC: [u8; 4] = *b"CASK";

/// The container-format version this build understands.
pub const FORMAT_VERSION: u16 = 1;

/// Size of the fixed header record, in bytes.
pub const HEADER_LEN: usize = 120;

/// Largest page size we accept (1 GiB), to bound `page_count * page_size`.
const MAX_PAGE_SIZE: u32 = 1 << 30;

/// A located byte range within the container (`offset`, `len`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// Byte offset from the start of the image.
    pub offset: u64,
    /// Length in bytes.
    pub len: u64,
}

impl Section {
    /// Validates that `self` lies fully within an image of `total` bytes,
    /// using checked arithmetic so a crafted `offset + len` cannot wrap.
    fn end_within(self, total: u64) -> Result<(), CaskError> {
        match self.offset.checked_add(self.len) {
            Some(end) if end <= total => Ok(()),
            _ => Err(CaskError::Malformed),
        }
    }
}

/// The fixed-layout container header (the trusted description of the image).
#[derive(Clone, Copy, Debug)]
pub struct Header {
    /// Container format version (see [`FORMAT_VERSION`]).
    pub format_version: u16,
    /// Implementation-defined feature flags.
    pub flags: u16,
    /// Size of each code/data page, in bytes (a power of two).
    pub page_size: u32,
    /// Number of pages (Merkle leaves) in the body.
    pub page_count: u64,
    /// BLAKE3 Merkle root over the body pages.
    pub merkle_root: Hash,
    /// The signed Lading manifest.
    pub lading: Section,
    /// The page-aligned code/data body.
    pub body: Section,
    /// The detached signature block (Ed25519 + ML-DSA).
    pub signature: Section,
    /// The Logbook inclusion proof + checkpoint.
    pub logbook: Section,
}

/// Reads a little-endian `u16` at `off` within a header-sized buffer.
fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// Reads a little-endian `u32` at `off`.
fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Reads a little-endian `u64` at `off`.
fn rd_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

impl Header {
    /// Serializes the header to its canonical 120-byte little-endian form.
    ///
    /// Used by the host sealer (`xtask/shipwright`) and the self-test; the
    /// loader only ever parses, never writes.
    #[must_use]
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..6].copy_from_slice(&self.format_version.to_le_bytes());
        b[6..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..12].copy_from_slice(&self.page_size.to_le_bytes());
        // 12..16 reserved (zero).
        b[16..24].copy_from_slice(&self.page_count.to_le_bytes());
        b[24..56].copy_from_slice(&self.merkle_root);
        let mut put = |off: usize, s: Section| {
            b[off..off + 8].copy_from_slice(&s.offset.to_le_bytes());
            b[off + 8..off + 16].copy_from_slice(&s.len.to_le_bytes());
        };
        put(56, self.lading);
        put(72, self.body);
        put(88, self.signature);
        put(104, self.logbook);
        b
    }
}

/// A parsed, bounds-checked view over a `.cask` image.
///
/// All accessors return slices that borrow the original image; constructing a
/// `Cask` guarantees every section and page index it can yield is in range.
pub struct Cask<'a> {
    image: &'a [u8],
    header: Header,
}

impl<'a> Cask<'a> {
    /// Parses and fully validates a container header against `image`.
    ///
    /// Fail-closed: any inconsistency (bad magic/version, non-power-of-two page
    /// size, a section that runs past the end of the image, or a body whose size
    /// disagrees with `page_count * page_size`) yields [`CaskError::Malformed`].
    /// No allocation; the returned view borrows `image`.
    pub fn parse(image: &'a [u8]) -> Result<Self, CaskError> {
        if image.len() < HEADER_LEN {
            return Err(CaskError::Malformed);
        }
        let h = &image[..HEADER_LEN];
        if h[0..4] != MAGIC {
            return Err(CaskError::Malformed);
        }

        let format_version = rd_u16(h, 4);
        if format_version != FORMAT_VERSION {
            return Err(CaskError::Malformed);
        }

        let page_size = rd_u32(h, 8);
        if page_size == 0 || !page_size.is_power_of_two() || page_size > MAX_PAGE_SIZE {
            return Err(CaskError::Malformed);
        }

        let page_count = rd_u64(h, 16);
        if page_count == 0 {
            return Err(CaskError::Malformed);
        }

        let mut merkle_root = [0u8; 32];
        merkle_root.copy_from_slice(&h[24..56]);

        let section = |off: usize| Section {
            offset: rd_u64(h, off),
            len: rd_u64(h, off + 8),
        };
        let header = Header {
            format_version,
            flags: rd_u16(h, 6),
            page_size,
            page_count,
            merkle_root,
            lading: section(56),
            body: section(72),
            signature: section(88),
            logbook: section(104),
        };

        // The body must hold exactly `page_count` whole pages, computed without
        // overflow, and every section must lie within the image.
        let total = image.len() as u64;
        let body_expected = page_count
            .checked_mul(u64::from(page_size))
            .ok_or(CaskError::Malformed)?;
        if header.body.len != body_expected {
            return Err(CaskError::Malformed);
        }
        header.lading.end_within(total)?;
        header.body.end_within(total)?;
        header.signature.end_within(total)?;
        header.logbook.end_within(total)?;

        Ok(Self { image, header })
    }

    /// The validated header.
    #[must_use]
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The sealed Merkle root over the body pages.
    #[must_use]
    pub fn merkle_root(&self) -> &Hash {
        &self.header.merkle_root
    }

    /// Number of body pages (Merkle leaves).
    #[must_use]
    pub fn page_count(&self) -> u64 {
        self.header.page_count
    }

    /// Returns the bytes of the given section, which parsing proved in range.
    fn slice(&self, s: Section) -> &'a [u8] {
        let start = s.offset as usize;
        let end = start + s.len as usize;
        &self.image[start..end]
    }

    /// The signed Lading manifest bytes.
    #[must_use]
    pub fn lading_bytes(&self) -> &'a [u8] {
        self.slice(self.header.lading)
    }

    /// The detached signature-block bytes.
    #[must_use]
    pub fn signature_bytes(&self) -> &'a [u8] {
        self.slice(self.header.signature)
    }

    /// The Logbook inclusion-proof bytes.
    #[must_use]
    pub fn logbook_bytes(&self) -> &'a [u8] {
        self.slice(self.header.logbook)
    }

    /// Borrows page `index` from the body, or `None` if out of range.
    ///
    /// This is the slice the loader feeds to `merkle::verify_page` on a fault.
    #[must_use]
    pub fn page(&self, index: u64) -> Option<&'a [u8]> {
        if index >= self.header.page_count {
            return None;
        }
        let page_size = u64::from(self.header.page_size);
        // In range by construction: body holds exactly page_count*page_size.
        let rel = index * page_size;
        let start = (self.header.body.offset + rel) as usize;
        let end = start + page_size as usize;
        Some(&self.image[start..end])
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::merkle::MerkleTree;
    use std::vec;
    use std::vec::Vec;

    const PAGE: u32 = 64;

    /// Assembles a minimal valid image: header, lading, body, sig, logbook,
    /// laid out back to back. Returns the bytes and the page payloads.
    fn build_image(n_pages: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
        let lading = b"manifest".to_vec();
        let signature = b"sig-bytes".to_vec();
        let logbook = b"proof".to_vec();
        let pages: Vec<Vec<u8>> = (0..n_pages)
            .map(|i| (0..PAGE).map(|j| (i as u32 + j) as u8).collect())
            .collect();

        let page_refs: Vec<&[u8]> = pages.iter().map(Vec::as_slice).collect();
        let root = MerkleTree::build(&page_refs).unwrap().root();

        let mut body = Vec::new();
        for p in &pages {
            body.extend_from_slice(p);
        }

        let lading_off = HEADER_LEN as u64;
        let body_off = lading_off + lading.len() as u64;
        let sig_off = body_off + body.len() as u64;
        let logbook_off = sig_off + signature.len() as u64;

        let header = Header {
            format_version: FORMAT_VERSION,
            flags: 0,
            page_size: PAGE,
            page_count: n_pages as u64,
            merkle_root: root,
            lading: Section {
                offset: lading_off,
                len: lading.len() as u64,
            },
            body: Section {
                offset: body_off,
                len: body.len() as u64,
            },
            signature: Section {
                offset: sig_off,
                len: signature.len() as u64,
            },
            logbook: Section {
                offset: logbook_off,
                len: logbook.len() as u64,
            },
        };

        let mut image = Vec::new();
        image.extend_from_slice(&header.encode());
        image.extend_from_slice(&lading);
        image.extend_from_slice(&body);
        image.extend_from_slice(&signature);
        image.extend_from_slice(&logbook);
        (image, pages)
    }

    #[test]
    fn parses_valid_image() {
        let (image, pages) = build_image(5);
        let cask = Cask::parse(&image).unwrap();
        assert_eq!(cask.page_count(), 5);
        assert_eq!(cask.lading_bytes(), b"manifest");
        assert_eq!(cask.signature_bytes(), b"sig-bytes");
        assert_eq!(cask.logbook_bytes(), b"proof");
        for (i, p) in pages.iter().enumerate() {
            assert_eq!(cask.page(i as u64).unwrap(), p.as_slice());
        }
        assert!(cask.page(5).is_none());
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(matches!(Cask::parse(&[0u8; 10]), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_bad_magic() {
        let (mut image, _) = build_image(2);
        image[0] = b'X';
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_wrong_version() {
        let (mut image, _) = build_image(2);
        image[4] = 0xFF;
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_non_power_of_two_page_size() {
        let (mut image, _) = build_image(2);
        image[8..12].copy_from_slice(&63u32.to_le_bytes());
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_body_size_mismatch() {
        let (mut image, _) = build_image(2);
        // page_count claims 3 while the body holds 2 pages worth of bytes.
        image[16..24].copy_from_slice(&3u64.to_le_bytes());
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_section_past_end() {
        let (mut image, _) = build_image(2);
        // Push the logbook length far past the image end.
        image[112..120].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    #[test]
    fn rejects_zero_pages() {
        // Hand-build a header claiming zero pages.
        let header = Header {
            format_version: FORMAT_VERSION,
            flags: 0,
            page_size: PAGE,
            page_count: 0,
            merkle_root: [0u8; 32],
            lading: Section {
                offset: HEADER_LEN as u64,
                len: 0,
            },
            body: Section {
                offset: HEADER_LEN as u64,
                len: 0,
            },
            signature: Section {
                offset: HEADER_LEN as u64,
                len: 0,
            },
            logbook: Section {
                offset: HEADER_LEN as u64,
                len: 0,
            },
        };
        let image = header.encode().to_vec();
        assert!(matches!(Cask::parse(&image), Err(CaskError::Malformed)));
    }

    /// A spray of random-ish truncations and byte flips must never panic.
    #[test]
    fn fuzz_never_panics() {
        let (base, _) = build_image(4);
        for cut in 0..base.len() {
            let _ = Cask::parse(&base[..cut]);
        }
        for i in 0..base.len() {
            let mut m = base.clone();
            m[i] ^= 0xA5;
            let _ = Cask::parse(&m);
        }
        let _ = vec![0u8; 0];
    }
}
