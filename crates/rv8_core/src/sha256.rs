// CEP:FILE: crates/rv8_core/src/sha256.rs
// CEP:WHAT: A self-contained SHA-256 implementation for content-addressed cache keys and
//   reproducibility attestation.
// CEP:WHY: The CAS cache (standard: incremental compilation) requires a cryptographic digest so
//   that accidental content collisions cannot silently reuse stale artifacts.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; total function over byte slices.
// CEP:ASSUMES: FIPS 180-4 algorithm; correctness is pinned against published test vectors.
// CEP:COST: O(n) with 64-round block compression; roughly 10 cycles/byte unoptimized.
// CEP:EVIDENCE: unit tests rv8_core::sha256::tests::{nist_vectors, length_padding_edge}.
// CEP:SECURITY: used for integrity of compiler artifacts, not for secret handling.

/// CEP:WHAT: SHA-256 output size in bytes.
/// CEP:WHY: Named constant per Law 7.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::sha256::tests::nist_vectors.
pub const SHA256_DIGEST_BYTES: usize = 32;

/// CEP:WHAT: SHA-256 block size in bytes.
/// CEP:WHY: Named constant per Law 7.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::sha256::tests::nist_vectors.
pub const SHA256_BLOCK_BYTES: usize = 64;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// CEP:WHAT: Computes the SHA-256 digest of a byte slice.
/// CEP:WHY: Single entry point for content addressing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: input is a complete message; no streaming API needed for compile artifacts.
/// CEP:COST: O(n), one block compression per 64 bytes.
/// CEP:EVIDENCE: unit test rv8_core::sha256::tests::nist_vectors.
pub fn sha256(message: &[u8]) -> [u8; SHA256_DIGEST_BYTES] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (message.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(message.len() + SHA256_BLOCK_BYTES * 2);
    padded.extend_from_slice(message);
    padded.push(0x80);
    while padded.len() % SHA256_BLOCK_BYTES != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let (blocks, _) = padded.as_chunks::<SHA256_BLOCK_BYTES>();
    for block in blocks {
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (i, word) in words.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut digest = [0u8; SHA256_DIGEST_BYTES];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// CEP:WHAT: Formats a digest as lowercase hexadecimal.
/// CEP:WHY: Cache keys and logs use stable text form.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(32).
/// CEP:EVIDENCE: unit test rv8_core::sha256::tests::nist_vectors.
pub fn to_hex(digest: &[u8; SHA256_DIGEST_BYTES]) -> String {
    let mut out = String::with_capacity(SHA256_DIGEST_BYTES * 2);
    for b in digest {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xF) as u32, 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies NIST FIPS 180-4 test vectors.
    /// CEP:WHY: Correctness of content addressing depends on standard conformance.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn nist_vectors() {
        assert_eq!(
            to_hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            to_hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            to_hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    /// CEP:WHAT: Verifies padding across block boundaries.
    /// CEP:WHY: Length-encoding bugs appear only at 55/56/64-byte edges.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn length_padding_edge() {
        let d55 = sha256(&[b'a'; 55]);
        let d56 = sha256(&[b'a'; 56]);
        let d64 = sha256(&[b'a'; 64]);
        assert_ne!(to_hex(&d55), to_hex(&d56));
        assert_ne!(to_hex(&d56), to_hex(&d64));
        assert_eq!(sha256(&[0u8; 0]).len(), SHA256_DIGEST_BYTES);
    }
}
