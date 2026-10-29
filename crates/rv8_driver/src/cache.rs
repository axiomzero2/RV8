// CEP:FILE: crates/rv8_driver/src/cache.rs
// CEP:WHAT: Content addressing for pipeline artifacts: SHA-256 content ids and the
//   artifact directory layout of the incremental cache.
// CEP:WHY: Standard 5.10 fixes content-addressed storage keyed by BLAKE3; the SHA-256
//   core from rv8_core fills the same role until the BLAKE3 core lands (RV8-133), and
//   every consumer addresses content through this module so the swap is one file.
// CEP:CLASS: HPC-1
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-133
// CEP:FAILURE: content_id is total; no failure paths.
// CEP:ASSUMES: collision resistance of SHA-256 suffices for cache identity.
// CEP:COST: O(bytes) per id.
// CEP:EVIDENCE: unit tests rv8_driver::cache::tests::*.

/// CEP:WHAT: Computes the 32-byte content id of an artifact.
/// CEP:WHY: Cache identity, determinism spot-checks, and .d-free dependency evidence all
///   reduce to content hashing through this single function.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total over all byte slices.
/// CEP:ASSUMES: none
/// CEP:COST: O(bytes).
/// CEP:EVIDENCE: unit test rv8_driver::cache::tests::content_id_is_stable_and_distinct.
pub fn content_id(bytes: &[u8]) -> [u8; 32] {
    rv8_core::sha256::sha256(bytes)
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
}
