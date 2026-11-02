// CEP:FILE: crates/rv8_driver/src/cache.rs
// CEP:WHAT: Content addressing for pipeline artifacts: BLAKE3 content ids and the
//   artifact directory layout of the incremental cache.
// CEP:WHY: Standard 5.10 fixes content-addressed storage keyed by BLAKE3; the
//   rv8_core::blake3 core (RV8-133) now provides it, and every consumer addresses
//   content through this module so any future hash change stays a one-file swap.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-133 (resolved: BLAKE3 core landed in rv8_core::blake3)
// CEP:FAILURE: content_id is total; no failure paths.
// CEP:ASSUMES: collision resistance of BLAKE3 suffices for cache identity.
// CEP:COST: O(bytes) per id.
// CEP:EVIDENCE: unit tests rv8_driver::cache::tests::*; the BLAKE3 conformance
//   vectors in rv8_core::blake3::tests.

/// CEP:WHAT: Computes the 32-byte BLAKE3 content id of an artifact.
/// CEP:WHY: Cache identity, determinism spot-checks, and .d-free dependency evidence all
///   reduce to content hashing through this single function.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total over all byte slices.
/// CEP:ASSUMES: none
/// CEP:COST: O(bytes).
/// CEP:EVIDENCE: unit test rv8_driver::cache::tests::content_id_matches_blake3_core.
pub fn content_id(bytes: &[u8]) -> [u8; 32] {
    rv8_core::blake3::hash(bytes)
}

/// CEP:WHAT: Renders a content id as a lowercase hex string.
/// CEP:WHY: File names in the cache directory are hex ids; deterministic rendering keeps
///   the layout byte-stable across runs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(64).
/// CEP:EVIDENCE: unit test rv8_driver::cache::tests::content_id_is_stable_and_distinct.
pub fn content_hex(id: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in id {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Same bytes give the same id; different bytes give different ids.
    /// CEP:WHY: The cache correctness invariant.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn content_id_is_stable_and_distinct() {
        let a = content_id(b"add_main");
        let b = content_id(b"add_main");
        let c = content_id(b"add_main2");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(content_hex(&a).len(), 64);
        assert!(content_hex(&a).chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    /// CEP:WHAT: content_id equals the BLAKE3 core digest for one-shot and streaming
    /// input, and the digest equals the pinned reference vector for the empty input.
    /// CEP:WHY: Pins the cache to the BLAKE3 standard, not just to itself.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn content_id_matches_blake3_core() {
        assert_eq!(content_id(b""), rv8_core::blake3::hash(b""));
        assert_eq!(
            content_hex(&content_id(b"")),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 253) as u8).collect();
        assert_eq!(content_id(&big), rv8_core::blake3::hash(&big));
        let mut h = rv8_core::blake3::Hasher::new();
        for chunk in big.chunks(4096) {
            h.update(chunk);
        }
        assert_eq!(content_id(&big), h.finalize32());
    }
}
