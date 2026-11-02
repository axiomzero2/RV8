# BLAKE3 Content Addressing

## Abstract
`rv8_core::blake3` implements unkeyed BLAKE3 hashing (one-shot and incremental, with extendable output) with zero external dependencies. Inputs are byte slices; outputs are 32-byte digests or arbitrary-length XOF streams. `rv8_driver::cache::content_id` is the single consumer entry point for cache identity.

## Formal Definitions
```rust
pub const BLAKE3_OUT_BYTES: usize = 32;
pub const BLAKE3_BLOCK_BYTES: usize = 64;
pub const BLAKE3_CHUNK_BYTES: usize = 1024;
pub const MAX_CV_STACK: usize = 54;
pub struct Hasher { /* chunk state + cv stack */ }
impl Hasher {
    pub fn new() -> Hasher;
    pub fn update(&mut self, input: &[u8]);
    pub fn finalize(&self, out: &mut [u8]);
    pub fn finalize32(&self) -> [u8; 32];
}
pub fn hash(input: &[u8]) -> [u8; 32];
```
`Hasher` holds one open `ChunkState` (chaining value, 64-bit chunk counter, 64-byte block buffer, compressed-block count) and a chaining-value stack of at most `MAX_CV_STACK` entries. Keyed hashing and key derivation are out of scope (the cache uses unkeyed hashing only).

## Algorithm
1. `update` absorbs at most one open chunk (1024 bytes); at block rollovers the buffered block is compressed (CHUNK_START on the first block only) and the chaining value advances.
2. When a chunk completes and input remains, its chaining value is merged by `add_chunk_cv`: while the completed-chunk count is even, the stack top is popped and parented (PARENT flag, no ROOT); the result is pushed.
3. `finalize` computes the root: a single-chunk input compresses the open chunk (CHUNK_END, plus ROOT per output group); otherwise the stack folds top-down and the bottom entry forms the root block `[stack[0], folded right side]` compressed with PARENT|ROOT.
4. XOF extension recompresses the SAME root state with increasing 64-bit counters, emitting all 16 words (64 bytes) per group.

Time is O(bytes) with 7-round compression per 64-byte block; space is O(1) beyond the buffers. Assumption: the BLAKE3 reference algorithm.

## Invariants
1. `hash(bytes)` equals a streaming `Hasher` over the same bytes for every split of the input (pinned by test for split sizes 1..=4096).
2. The digest is pinned to the official implementation's vectors for empty, abc, 63/64/65, 1024/1025/2048/3072, a 1280-byte pattern, and XOF lengths 64/100/131.
3. Every byte length 0..=1030 yields a distinct digest (chunk/block boundary sweep).
4. The cv-stack underflow state is unreachable by construction; debug builds trap, release degrades deterministically (span.rs convention).
5. Output is a pure function of input bytes (no keys, no randomness).

## Integration Points
Depends on: none (std only). Depended on by: `rv8_driver::cache::content_id(bytes: &[u8]) -> [u8; 32]` (RV8-133), `rv8_driver::query::QueryDatabase::set_input` (input content hashing).

## Known Limitations
Keyed hashing, key derivation, and multi-threaded or SIMD compression are not implemented; compression is scalar and sequential. Hashing inputs larger than 2^54 chunks exceeds the stack budget and is rejected by construction.
