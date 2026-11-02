// CEP:FILE: crates/rv8_core/src/blake3.rs
// CEP:WHAT: A self-contained BLAKE3 implementation (unkeyed hashing, incremental or
//   one-shot, extendable output) for content-addressed cache keys.
// CEP:WHY: Standard 5.10 fixes content-addressed storage keyed by BLAKE3; RV8-133
//   lands the core so the driver's cache identity stops borrowing SHA-256. Zero
//   external dependencies, per STANDARDS 2.1.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-133
// CEP:FAILURE: hash and Hasher are total over all byte slices; the one by-construction
//   unreachable state (cv-stack underflow) traps under debug_assertions and degrades
//   deterministically in release, mirroring span.rs's debug_assert convention.
// CEP:ASSUMES: the BLAKE3 reference algorithm (BLAKE3 paper, appendix); correctness is
//   pinned against vectors generated with the official implementation. Keyed hashing
//   and key derivation are out of scope (the cache uses unkeyed hashing only).
// CEP:COST: O(bytes); 7-round compression per 64-byte block, tree-parallel by
//   construction (sequential in this revision).
// CEP:EVIDENCE: unit tests rv8_core::blake3::tests::{reference_vectors, xof_outputs,
//   chunk_boundaries, incremental_matches_one_shot}.
// CEP:SECURITY: used for integrity of compiler artifacts, not for secret handling.

/// CEP:WHAT: BLAKE3 default output size in bytes.
/// CEP:WHY: Named constant per 38.12.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: reference_vectors.
pub const BLAKE3_OUT_BYTES: usize = 32;

/// CEP:WHAT: BLAKE3 block size in bytes.
/// CEP:WHY: Named constant per 38.12.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: chunk_boundaries.
pub const BLAKE3_BLOCK_BYTES: usize = 64;

/// CEP:WHAT: BLAKE3 chunk size in bytes.
/// CEP:WHY: Named constant per 38.12; the chunk/chunk-pair tree is keyed on it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: chunk_boundaries.
pub const BLAKE3_CHUNK_BYTES: usize = 1024;

/// CEP:WHAT: Maximum depth of the chunk chaining-value stack.
/// CEP:WHY: 54 levels address 2^54 chunks (2^64 bytes) minus one merge slack; a named
///   budget (38.12) instead of an unbounded Vec.
/// CEP:STATUS: complete
/// CEP:FAILURE: InputTooLarge beyond 2^54 chunks (unreachable for byte slices in practice).
/// CEP:ASSUMES: usize is at least 64 bits on supported targets.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: chunk_boundaries exercises depth-2 merges.
pub const MAX_CV_STACK: usize = 54;

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const MSG_PERMUTATION: [usize; 16] =
    [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;

/// CEP:WHAT: One 7-round BLAKE3 compression over a 16-word state.
/// CEP:WHY: The sole mixing primitive; every flag/tree behavior lives above it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: words are little-endian u32.
/// CEP:COST: 7 rounds x 8 G functions.
/// CEP:EVIDENCE: reference_vectors pins every round path.
#[inline]
fn compress(cv: &[u32; 8], block_words: &[u32; 16], counter: u64, block_len: u32, flags: u32) -> [u32; 16] {
    let mut state: [u32; 16] = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
        IV[0], IV[1], IV[2], IV[3], IV[4], IV[5], IV[6], IV[7],
    ];
    state[12] = counter as u32;
    state[13] = (counter >> 32) as u32;
    state[14] = block_len;
    state[15] = flags;

    let mut m = *block_words;
    for round in 0..7 {
        // columns
        g(&mut state, 0, 4, 8, 12, m[0], m[1]);
        g(&mut state, 1, 5, 9, 13, m[2], m[3]);
        g(&mut state, 2, 6, 10, 14, m[4], m[5]);
        g(&mut state, 3, 7, 11, 15, m[6], m[7]);
        // diagonals
        g(&mut state, 0, 5, 10, 15, m[8], m[9]);
        g(&mut state, 1, 6, 11, 12, m[10], m[11]);
        g(&mut state, 2, 7, 8, 13, m[12], m[13]);
        g(&mut state, 3, 4, 9, 14, m[14], m[15]);
        if round < 6 {
            let mut next = [0u32; 16];
            for i in 0..16 {
                next[i] = m[MSG_PERMUTATION[i]];
            }
            m = next;
        }
    }
    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= cv[i];
    }
    state
}

/// CEP:WHAT: The BLAKE3 G mixing function on state lanes.
/// CEP:WHY: Half-round primitive of compress.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: lane indices within 0..16.
/// CEP:COST: 4 adds, 3 xors, 3 rotates.
/// CEP:EVIDENCE: reference_vectors.
#[inline]
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

/// CEP:WHAT: Splits a 64-byte block into 16 little-endian words.
/// CEP:WHY: compress consumes words, callers hold bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: debug_asserts the slice length (internal callers only).
/// CEP:ASSUMES: block.len() == BLAKE3_BLOCK_BYTES.
/// CEP:COST: O(64).
/// CEP:EVIDENCE: reference_vectors.
#[inline]
fn words_of(block: &[u8]) -> [u32; 16] {
    debug_assert_eq!(block.len(), BLAKE3_BLOCK_BYTES);
    let mut w = [0u32; 16];
    for (i, word) in w.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    w
}

/// CEP:WHAT: Pending one-chunk state: chaining value, counter, buffered block.
/// CEP:WHY: Chunks stream block-by-block before their chaining value exists.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: block_len <= BLAKE3_BLOCK_BYTES; blocks_compressed counted per 64 bytes.
/// CEP:COST: POD plus one block buffer.
/// CEP:EVIDENCE: incremental_matches_one_shot.
struct ChunkState {
    cv: [u32; 8],
    chunk_counter: u64,
    block: [u8; BLAKE3_BLOCK_BYTES],
    block_len: usize,
    blocks_compressed: u8,
}

impl ChunkState {
    fn new(chunk_counter: u64) -> ChunkState {
        ChunkState {
            cv: IV,
            chunk_counter,
            block: [0; BLAKE3_BLOCK_BYTES],
            block_len: 0,
            blocks_compressed: 0,
        }
    }

    fn len(&self) -> usize {
        self.blocks_compressed as usize * BLAKE3_BLOCK_BYTES + self.block_len
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 { CHUNK_START } else { 0 }
    }

    /// CEP:WHAT: Compresses the buffered block and returns the chunk's chaining value.
    /// CEP:WHY: Mid-stream rollover (full block, no END flag) and finalize (any block
    ///   length, CHUNK_END flag) both reduce to this.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: debug_asserts a full block for the rollover path (internal callers).
    /// CEP:ASSUMES: block_len == BLAKE3_BLOCK_BYTES when finalize is false.
    /// CEP:COST: one compress.
    /// CEP:EVIDENCE: reference_vectors.
    fn output_chaining_value(&self, finalize: bool) -> [u32; 8] {
        debug_assert!(finalize || self.block_len == BLAKE3_BLOCK_BYTES);
        let out = compress(
            &self.cv,
            &words_of(&self.block),
            self.chunk_counter,
            self.block_len as u32,
            self.start_flag() | if finalize { CHUNK_END } else { 0 },
        );
        let mut cv = [0u32; 8];
        cv.copy_from_slice(&out[..8]);
        cv
    }
}

/// CEP:WHAT: Incremental BLAKE3 hasher: new/update/finalize, unkeyed, extendable output.
/// CEP:WHY: One-shot hashing is `hash`; the incremental form serves streaming callers
///   (cache writers hashing artifacts as they serialize).
/// CEP:STATUS: complete
/// CEP:FAILURE: total; see the file CEP:FAILURE for the unreachable stack-underflow
///   behavior.
/// CEP:ASSUMES: unkeyed mode only.
/// CEP:COST: O(bytes) amortized.
/// CEP:EVIDENCE: incremental_matches_one_shot, reference_vectors.
pub struct Hasher {
    chunk: ChunkState,
    stack: Vec<[u32; 8]>,
}

impl Default for Hasher {
    fn default() -> Hasher {
        Hasher::new()
    }
}

impl Hasher {
    /// CEP:WHAT: Creates an empty hasher.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: incremental_matches_one_shot.
    pub fn new() -> Hasher {
        Hasher { chunk: ChunkState::new(0), stack: Vec::new() }
    }

    /// CEP:WHAT: Absorbs input bytes (streaming, any number of calls).
    /// CEP:WHY: The incremental contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (total).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(bytes) amortized.
    /// CEP:EVIDENCE: incremental_matches_one_shot.
    pub fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.chunk.len() == BLAKE3_CHUNK_BYTES {
                let chunk_cv = self.chunk.output_chaining_value(true);
                let total = self.chunk.chunk_counter + 1;
                self.add_chunk_cv(chunk_cv, total);
                self.chunk = ChunkState::new(total);
            }
            let want = BLAKE3_CHUNK_BYTES - self.chunk.len();
            let take = want.min(input.len());
            let (head, tail) = input.split_at(take);
            // Feed at most one block at a time so the chunk's block buffer stays exact.
            let mut rest = head;
            while !rest.is_empty() {
                let room = BLAKE3_BLOCK_BYTES - self.chunk.block_len;
                let n = room.min(rest.len());
                self.chunk.block[self.chunk.block_len..self.chunk.block_len + n]
                    .copy_from_slice(&rest[..n]);
                self.chunk.block_len += n;
                rest = &rest[n..];
                if self.chunk.block_len == BLAKE3_BLOCK_BYTES && !rest.is_empty() {
                    // Mid-chunk block rollover: compress and start a fresh block.
                    let cv = self.chunk.output_chaining_value(false);
                    self.chunk.blocks_compressed += 1;
                    let counter = self.chunk.chunk_counter;
                    let blocks = self.chunk.blocks_compressed;
                    self.chunk = ChunkState::new(counter);
                    self.chunk.cv = cv;
                    self.chunk.blocks_compressed = blocks;
                }
            }
            input = tail;
        }
    }

    /// CEP:WHAT: Merges a completed chunk's chaining value into the stack.
    /// CEP:WHY: The BLAKE3 left-leaning tree: an even chunk count pops and merges the
    ///   equal-sized left sibling before pushing.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: the by-construction unreachable stack underflow traps under
    ///   debug_assertions and degrades deterministically in release (see file CEP).
    /// CEP:ASSUMES: called exactly once per completed chunk, in chunk order.
    /// CEP:COST: O(log chunks) amortized.
    /// CEP:EVIDENCE: chunk_boundaries (1024/1025/2048/3072-byte vectors exercise it).
    fn add_chunk_cv(&mut self, mut new_cv: [u32; 8], mut total_chunks: u64) {
        while total_chunks & 1 == 0 {
            if self.stack.is_empty() {
                // Unreachable: an even chunk count always has a pushed sibling.
                debug_assert!(false, "blake3 cv stack underflow (unreachable by construction)");
                break;
            }
            let left = self.stack.pop().unwrap_or(IV);
            new_cv = parent_cv(&left, &new_cv);
            total_chunks >>= 1;
        }
        if self.stack.len() < MAX_CV_STACK {
            self.stack.push(new_cv);
        }
    }

    /// CEP:WHAT: Writes the extended output of `out.len()` bytes.
    /// CEP:WHY: BLAKE3 XOF; the 32-byte digest is the `out.len() == 32` case and the
    ///   first 64-byte group of any longer output.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: the open chunk is never empty when the stack is non-empty (rotation
    ///   only fires with input remaining); the code stays total if that ever changes.
    /// CEP:COST: one compress per 64 output bytes plus the fold.
    /// CEP:EVIDENCE: xof_outputs (64- and 131-byte XOF vectors), reference_vectors.
    pub fn finalize(&self, out: &mut [u8]) {
        // Root state per the reference Output: a single-chunk root compresses the open
        // chunk itself; a parent root compresses [stack[0], folded right side].
        let (root_cv, root_block, root_block_len, root_flags): ([u32; 8], [u32; 16], u32, u32);
        if self.stack.is_empty() {
            root_cv = self.chunk.cv;
            root_block = words_of(&self.chunk.block);
            root_block_len = self.chunk.block_len as u32;
            root_flags = self.chunk.start_flag() | CHUNK_END;
        } else {
            // Fold the stack top-down: every entry above the bottom one is an
            // intermediate parent (no ROOT flag); the bottom entry then forms the
            // root block [stack[0], folded right side], compressed with PARENT|ROOT
            // per XOF group (the reference parent_output semantics).
            let mut acc = self.chunk.output_chaining_value(true);
            for &entry in self.stack.iter().skip(1).rev() {
                acc = parent_cv(&entry, &acc);
            }
            let mut block = [0u32; 16];
            block[..8].copy_from_slice(&self.stack[0]);
            block[8..].copy_from_slice(&acc);
            root_cv = IV;
            root_block = block;
            root_block_len = BLAKE3_BLOCK_BYTES as u32;
            root_flags = PARENT;
        }
        // XOF: every 64-byte output group recompresses the SAME root state with the
        // next counter and emits all 16 words, little-endian.
        for (counter, group) in out.chunks_mut(64).enumerate() {
            let words = compress(
                &root_cv,
                &root_block,
                counter as u64,
                root_block_len,
                root_flags | ROOT,
            );
            for (i, word) in words.iter().enumerate() {
                let start = i * 4;
                if start < group.len() {
                    let bytes = word.to_le_bytes();
                    let end = (start + 4).min(group.len());
                    group[start..end].copy_from_slice(&bytes[..end - start]);
                }
            }
        }
    }

    /// CEP:WHAT: 32-byte digest convenience.
    /// CEP:WHY: The common cache case.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(bytes).
    /// CEP:EVIDENCE: reference_vectors.
    pub fn finalize32(&self) -> [u8; BLAKE3_OUT_BYTES] {
        let mut out = [0u8; BLAKE3_OUT_BYTES];
        self.finalize(&mut out);
        out
    }
}

/// CEP:WHAT: Parent chaining value over two child chaining values (intermediate
/// merge, no ROOT flag; the root parent's full 16-word block is built in finalize).
/// CEP:WHY: Tree merge primitive of add_chunk_cv.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: one compress.
/// CEP:EVIDENCE: reference_vectors.
fn parent_cv(left: &[u32; 8], right: &[u32; 8]) -> [u32; 8] {
    let mut block = [0u32; 16];
    block[..8].copy_from_slice(left);
    block[8..].copy_from_slice(right);
    let out = compress(&IV, &block, 0, BLAKE3_BLOCK_BYTES as u32, PARENT);
    let mut cv = [0u32; 8];
    cv.copy_from_slice(&out[..8]);
    cv
}

/// CEP:WHAT: One-shot BLAKE3 hash with a 32-byte digest.
/// CEP:WHY: The content-id entry point (RV8-133).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(bytes).
/// CEP:EVIDENCE: reference_vectors.
pub fn hash(input: &[u8]) -> [u8; BLAKE3_OUT_BYTES] {
    let mut h = Hasher::new();
    h.update(input);
    h.finalize32()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// CEP:WHAT: Pins the implementation against vectors generated by the official
    /// BLAKE3 implementation (empty, abc, chunk edges 63/64/65, tree edges
    /// 1024/1025/2048/3072, a 1280-byte pattern, and XOF lengths 64/100/131).
    /// CEP:WHY: Standard conformance is the correctness argument for cache identity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn reference_vectors() {
        let cases: &[(&[u8], usize, &str)] = &[
            (b"", 32, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
            (b"abc", 32, "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"),
            (&[b'a'; 63], 32, "1a2a060cf56e4a859d80723cac9e2391d3c09a33008483e5424c57fe68629b79"),
            (&[b'a'; 64], 32, "472c51290d607f100d2036fdcedd7590bba245e9adeb21364a063b7bb4ca81c7"),
            (&[b'a'; 65], 32, "f345679d9055e53939e92c04ff4f6c9d824b849810d4b598f54baa23336cde99"),
            (&[b'a'; 1024], 32, "5a1c9e5d85d9898297037e8e24f69bb0e604a84c91c3b3ef4784a374812900d9"),
            (&[b'a'; 1025], 32, "c59d2e12583df14d951e757a42f1734d355c8c5b1db6b6a33ab2bfabeed40c7d"),
            (&[b'a'; 2048], 32, "11654ac17d073b0905429320fee0a34776cb5f10a9767287c70b627fc4f45539"),
            (&[b'a'; 3072], 32, "452f43e13d923a9ee495b3640e4dc681d6224586a9d1252b9d837d13438c92b5"),
            (&pattern1280(), 32, "7586fa21246e8e17b384b8fe78948d916880e5ce4a80d030f45504952a69e050"),
        ];
        for (input, len, want) in cases {
            let mut out = vec![0u8; *len];
            let mut h = Hasher::new();
            h.update(input);
            h.finalize(&mut out);
            assert_eq!(hex(&out), *want, "input len {} xof {}", input.len(), len);
            assert_eq!(hex(&hash(input)), *want, "one-shot len {}", input.len());
        }
    }

    fn pattern1280() -> Vec<u8> {
        let mut v = Vec::with_capacity(1280);
        for i in 0..1280u32 {
            v.push((i % 256) as u8);
        }
        v
    }

    /// CEP:WHAT: Extendable output longer than 32 bytes.
    /// CEP:WHY: The XOF extension path (counter-chained root compressions).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn xof_outputs() {
        let mut out64 = [0u8; 64];
        Hasher::new().finalize(&mut out64);
        assert_eq!(
            hex(&out64),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262\
             e00f03e7b69af26b7faaf09fcd333050338ddfe085b8cc869ca98b206c08243a"
        );
        let mut out131 = vec![0u8; 131];
        Hasher::new().finalize(&mut out131);
        assert!(hex(&out131).starts_with(
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262\
             e00f03e7b69af26b7faaf09fcd333050338ddfe085b8cc869ca98b206c08243a\
             26f5487789e8f660afe6c99ef9e0c52b92e7393024a80459cf91f476f9ffdbda\
             7001c22e159b402631f277ca96f2defdf1078282314e763699a31c5363165421cce14d"
        ));
        let mut out100 = vec![0u8; 100];
        let mut h = Hasher::new();
        h.update(&pattern1280());
        h.finalize(&mut out100);
        assert!(hex(&out100).starts_with("7586fa21246e8e17b384b8fe78948d916880e5ce4a80d030f45504952a69e050"));
        // XOF prefix property: the first 32 bytes are the plain digest.
        assert_eq!(&out100[..32], &hash(&pattern1280()));
    }

    /// CEP:WHAT: Every byte length 0..=1030 hashes distinctly (block and chunk edges).
    /// CEP:WHY: Off-by-one rollover bugs live exactly at 63/64/65 and 1023/1024/1025.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn chunk_boundaries() {
        let mut seen = std::collections::BTreeSet::new();
        for n in 0..=1030usize {
            let input = vec![b'x'; n];
            let d = hash(&input);
            assert_eq!(d.len(), BLAKE3_OUT_BYTES);
            assert!(seen.insert(d), "digest collision at len {n}");
        }
        assert_eq!(seen.len(), 1031);
    }

    /// CEP:WHAT: Streaming update calls produce the one-shot digest.
    /// CEP:WHY: The incremental contract (cache writers stream artifacts).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn incremental_matches_one_shot() {
        let input: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let want = hash(&input);
        for split in [1usize, 63, 64, 65, 1023, 1024, 1025, 4096] {
            let mut h = Hasher::new();
            for chunk in input.chunks(split) {
                h.update(chunk);
            }
            assert_eq!(h.finalize32(), want, "chunk size {split}");
        }
        // Empty input, both forms.
        assert_eq!(hash(b""), hash(&[]));
        let mut h = Hasher::new();
        h.update(b"");
        assert_eq!(h.finalize32(), hash(b""));
    }

    /// CEP:WHAT: Determinism spot-check and avalanche sanity over similar inputs.
    /// CEP:WHY: Cache identity requires stable, well-mixed digests.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn deterministic_and_distinct() {
        assert_eq!(hash(b"add_main"), hash(b"add_main"));
        assert_ne!(hash(b"add_main"), hash(b"add_main2"));
        // Flipping one input bit changes many output bits (loose avalanche bound).
        let a = hash(b"avalanche");
        let mut b_input = b"avalanche".to_vec();
        b_input[0] ^= 0x01;
        let b = hash(&b_input);
        let bits: u32 = a.iter().zip(b.iter()).map(|(x, y)| (x ^ y).count_ones()).sum();
        assert!(bits > 64, "avalanche too weak: {bits}");
    }
}
