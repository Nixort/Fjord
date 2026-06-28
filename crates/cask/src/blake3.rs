// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! A self-contained, `no_std`, heap-free BLAKE3 hasher.
//!
//! Fjord pins **zero** third-party crates (see `Cargo.lock`): every primitive
//! the trusted path depends on is auditable in-tree. BLAKE3 is the integrity
//! hash for the whole chain of trust — the Cask Merkle tree (`cask::merkle`),
//! the filesystem tree (`brine`), and the DICE measurements (`anchor`) — so it
//! lives here, implemented directly from the reference specification.
//!
//! This is the default (unkeyed) hash. Keyed-hash and derive-key modes are not
//! needed by the Phase-2 loader path and are deliberately omitted; the flag
//! plumbing is present so they can be added without touching the core.
//!
//! The implementation is allocation-free: the incremental [`Hasher`] keeps a
//! fixed 54-entry subtree stack on its own storage, which is the maximum depth
//! for any input addressable by the 64-bit chunk counter. It therefore runs
//! unchanged inside the kernel-adjacent loader, where there is no heap.
//!
//! Correctness is pinned to the upstream known-answer vectors in the unit tests
//! (`cargo test`), covering inputs that span multiple chunks and parent nodes.

/// Length of a BLAKE3 hash / chaining value, in bytes.
pub const OUT_LEN: usize = 32;
/// Length of one compression input block, in bytes.
const BLOCK_LEN: usize = 64;
/// Length of one chunk (the leaf granularity of the hash tree), in bytes.
const CHUNK_LEN: usize = 1024;

// Domain-separation flags mixed into the compression function.
const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;

/// The BLAKE3 initialization vector (the SHA-256 fractional-square-root words).
const IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

/// Per-round message-word permutation applied between the seven rounds.
const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

/// The quarter-round mixing function `G`.
#[inline]
#[allow(clippy::many_single_char_names, clippy::too_many_arguments)]
fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

/// One full round: four column mixes followed by four diagonal mixes.
fn round(state: &mut [u32; 16], m: &[u32; 16]) {
    // Columns.
    g(state, 0, 4, 8, 12, m[0], m[1]);
    g(state, 1, 5, 9, 13, m[2], m[3]);
    g(state, 2, 6, 10, 14, m[4], m[5]);
    g(state, 3, 7, 11, 15, m[6], m[7]);
    // Diagonals.
    g(state, 0, 5, 10, 15, m[8], m[9]);
    g(state, 1, 6, 11, 12, m[10], m[11]);
    g(state, 2, 7, 8, 13, m[12], m[13]);
    g(state, 3, 4, 9, 14, m[14], m[15]);
}

/// Applies [`MSG_PERMUTATION`] to the message words in place.
fn permute(m: &mut [u32; 16]) {
    let mut permuted = [0u32; 16];
    for i in 0..16 {
        permuted[i] = m[MSG_PERMUTATION[i]];
    }
    *m = permuted;
}

/// The BLAKE3 compression function: seven rounds of mixing, then the feed-forward.
///
/// Returns the full 16-word state. The first eight words are the chaining value;
/// callers extracting root output use all sixteen (see [`Output::root_hash`]).
fn compress(
    chaining_value: &[u32; 8],
    block_words: &[u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
) -> [u32; 16] {
    #[allow(clippy::cast_possible_truncation)]
    let counter_low = counter as u32;
    let counter_high = (counter >> 32) as u32;
    let mut state = [
        chaining_value[0],
        chaining_value[1],
        chaining_value[2],
        chaining_value[3],
        chaining_value[4],
        chaining_value[5],
        chaining_value[6],
        chaining_value[7],
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        counter_low,
        counter_high,
        block_len,
        flags,
    ];
    let mut block = *block_words;

    round(&mut state, &block); // round 1
    permute(&mut block);
    round(&mut state, &block); // round 2
    permute(&mut block);
    round(&mut state, &block); // round 3
    permute(&mut block);
    round(&mut state, &block); // round 4
    permute(&mut block);
    round(&mut state, &block); // round 5
    permute(&mut block);
    round(&mut state, &block); // round 6
    permute(&mut block);
    round(&mut state, &block); // round 7

    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= chaining_value[i];
    }
    state
}

/// Reads little-endian `u32` words out of a 64-byte block.
fn words_from_block(block: &[u8; BLOCK_LEN]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (word, chunk) in words.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    words
}

/// The first eight words of a compression result — a chaining value.
fn first_8(words: [u32; 16]) -> [u32; 8] {
    [
        words[0], words[1], words[2], words[3], words[4], words[5], words[6], words[7],
    ]
}

/// A node (chunk or parent) ready to emit either a chaining value or, when it is
/// the root, the final hash. Keeping the compression inputs rather than the
/// output lets the root be finalized with the [`ROOT`] flag set exactly once.
struct Output {
    input_chaining_value: [u32; 8],
    block_words: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    /// The chaining value passed to this node's parent.
    fn chaining_value(&self) -> [u32; 8] {
        first_8(compress(
            &self.input_chaining_value,
            &self.block_words,
            self.counter,
            self.block_len,
            self.flags,
        ))
    }

    /// The 32-byte root hash (a single output block, which is all Fjord needs).
    fn root_hash(&self) -> [u8; OUT_LEN] {
        let words = compress(
            &self.input_chaining_value,
            &self.block_words,
            0,
            self.block_len,
            self.flags | ROOT,
        );
        let mut out = [0u8; OUT_LEN];
        for (word, chunk) in words.iter().take(8).zip(out.chunks_exact_mut(4)) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

/// Accumulates one chunk (up to [`CHUNK_LEN`] bytes) of input.
struct ChunkState {
    chaining_value: [u32; 8],
    chunk_counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: u8,
    blocks_compressed: u8,
}

impl ChunkState {
    fn new(chunk_counter: u64) -> Self {
        Self {
            chaining_value: IV,
            chunk_counter,
            block: [0u8; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.blocks_compressed as usize + self.block_len as usize
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 {
            CHUNK_START
        } else {
            0
        }
    }

    /// Folds `input` into the chunk, compressing full blocks as they fill.
    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            // A full block is only compressed once we know more input follows,
            // so the final block of the chunk can carry CHUNK_END at output().
            if self.block_len as usize == BLOCK_LEN {
                let block_words = words_from_block(&self.block);
                self.chaining_value = first_8(compress(
                    &self.chaining_value,
                    &block_words,
                    self.chunk_counter,
                    BLOCK_LEN as u32,
                    self.start_flag(),
                ));
                self.blocks_compressed += 1;
                self.block = [0u8; BLOCK_LEN];
                self.block_len = 0;
            }

            let want = BLOCK_LEN - self.block_len as usize;
            let take = want.min(input.len());
            self.block[self.block_len as usize..self.block_len as usize + take]
                .copy_from_slice(&input[..take]);
            self.block_len += take as u8;
            input = &input[take..];
        }
    }

    /// Seals the chunk into an [`Output`] carrying its final (CHUNK_END) block.
    fn output(&self) -> Output {
        Output {
            input_chaining_value: self.chaining_value,
            block_words: words_from_block(&self.block),
            counter: self.chunk_counter,
            block_len: u32::from(self.block_len),
            flags: self.start_flag() | CHUNK_END,
        }
    }
}

/// Builds the parent [`Output`] that combines two child chaining values.
fn parent_output(left: &[u32; 8], right: &[u32; 8]) -> Output {
    let mut block_words = [0u32; 16];
    block_words[..8].copy_from_slice(left);
    block_words[8..].copy_from_slice(right);
    Output {
        input_chaining_value: IV,
        block_words,
        counter: 0,
        block_len: BLOCK_LEN as u32,
        flags: PARENT,
    }
}

/// An incremental BLAKE3 hasher.
///
/// Feed input with [`Hasher::update`] (any number of calls, any sizes) and read
/// the digest with [`Hasher::finalize`]. Allocation-free.
pub struct Hasher {
    chunk_state: ChunkState,
    // One chaining value per completed left-edge subtree, deepest last. 54 is
    // enough for the entire 2^64-chunk address space.
    cv_stack: [[u32; 8]; 54],
    cv_stack_len: u8,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    /// Creates a fresh hasher in the default (unkeyed) mode.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunk_state: ChunkState::new(0),
            cv_stack: [[0u32; 8]; 54],
            cv_stack_len: 0,
        }
    }

    fn push_stack(&mut self, cv: [u32; 8]) {
        self.cv_stack[self.cv_stack_len as usize] = cv;
        self.cv_stack_len += 1;
    }

    fn pop_stack(&mut self) -> [u32; 8] {
        self.cv_stack_len -= 1;
        self.cv_stack[self.cv_stack_len as usize]
    }

    /// Folds a freshly completed chunk's chaining value into the subtree stack,
    /// merging perfect subtrees as the binary counter of total chunks demands.
    fn add_chunk_chaining_value(&mut self, mut new_cv: [u32; 8], mut total_chunks: u64) {
        // Every trailing 1 bit in `total_chunks` marks a left subtree of equal
        // height waiting on the stack to merge with this new right subtree.
        while total_chunks & 1 == 0 {
            let left = self.pop_stack();
            new_cv = parent_output(&left, &new_cv).chaining_value();
            total_chunks >>= 1;
        }
        self.push_stack(new_cv);
    }

    /// Adds `input` to the hash. May be called repeatedly.
    pub fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.chunk_state.len() == CHUNK_LEN {
                let chunk_cv = self.chunk_state.output().chaining_value();
                let total_chunks = self.chunk_state.chunk_counter + 1;
                self.add_chunk_chaining_value(chunk_cv, total_chunks);
                self.chunk_state = ChunkState::new(total_chunks);
            }

            let want = CHUNK_LEN - self.chunk_state.len();
            let take = want.min(input.len());
            self.chunk_state.update(&input[..take]);
            input = &input[take..];
        }
    }

    /// Finalizes the hash and writes the 32-byte digest.
    #[must_use]
    pub fn finalize(&self) -> [u8; OUT_LEN] {
        // Walk the current chunk up through the pending left subtrees. The node
        // with no parent left to merge is the root and is finalized with ROOT.
        let mut output = self.chunk_state.output();
        let mut parent_nodes_remaining = self.cv_stack_len as usize;
        while parent_nodes_remaining > 0 {
            parent_nodes_remaining -= 1;
            let left = self.cv_stack[parent_nodes_remaining];
            output = parent_output(&left, &output.chaining_value());
        }
        output.root_hash()
    }
}

/// One-shot convenience: the BLAKE3 digest of `input`.
#[must_use]
pub fn hash(input: &[u8]) -> [u8; OUT_LEN] {
    let mut hasher = Hasher::new();
    hasher.update(input);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// The upstream test inputs are a repeating 0,1,..,250 byte sequence.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn hex(bytes: &[u8]) -> std::string::String {
        use std::fmt::Write;
        let mut s = std::string::String::new();
        for b in bytes {
            write!(s, "{b:02x}").unwrap();
        }
        s
    }

    /// Known-answer vectors from the upstream BLAKE3 `test_vectors.json`
    /// (unkeyed `hash`, first 32 output bytes). Lengths span single blocks,
    /// the chunk boundary, and several parent-node levels.
    #[test]
    fn known_answer_vectors() {
        let cases: &[(usize, &str)] = &[
            (0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
            (1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"),
            (2, "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63"),
            (63, "e9bc37a594daad83be9470df7f7b3798297c3d834ce80ba85d6e207627b7db7b"),
            (64, "4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98"),
            (65, "de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee"),
            (1023, "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11"),
            (1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
            (1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"),
            (2048, "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a"),
            (2049, "5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030"),
            (3072, "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2"),
            (4096, "015094013f57a5277b59d8475c0501042c0b642e531b0a1c8f58d2163229e969"),
        ];
        for (len, expected) in cases {
            let got = hash(&pattern(*len));
            assert_eq!(&hex(&got), expected, "BLAKE3 mismatch for input_len={len}");
        }
    }

    /// Streaming in arbitrary-sized pieces must equal the one-shot hash.
    #[test]
    fn incremental_matches_oneshot() {
        let input = pattern(5000);
        for split in [1usize, 63, 64, 100, 1023, 1024, 1025, 2048, 3001] {
            let mut h = Hasher::new();
            h.update(&input[..split]);
            h.update(&input[split..]);
            assert_eq!(h.finalize(), hash(&input), "split={split}");
        }
    }
}
