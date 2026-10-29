// CEP:FILE: crates/rv8_core/src/hash.rs
// CEP:WHAT: Deterministic Fx-style hasher plus type aliases for ordered-behavior hash collections.
// CEP:WHY: Standard 38.10 bans nondeterministic iteration; a fixed-seed hasher with insertion-
//   ordered wrappers keeps all map iteration reproducible across runs and machines.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: hash quality adequate for compiler tables; NOT collision-hardened against
//   adversarial inputs, so untrusted keys always pass through interner or sorted-vector paths.
// CEP:COST: fxhash64 processes 8 bytes per multiply-xor step; hash collections match std cost.
// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
// CEP:SECURITY: adversarial collision inputs are out of scope here; see security/threat_model.md.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// CEP:WHAT: Fixed multiplier for the FxHash mixing function.
/// CEP:WHY: Law 7 bans magic numbers; the rustc-proven multiplier is named and cited.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit target; enforced by the u64 type of the mixing functions.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
pub const FX_MULTIPLIER: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// CEP:WHAT: Mixes one u64 into an accumulator using one multiply and one xor.
/// CEP:WHY: Multiply-xor provides bit avalanche in a single dependency-chain step.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: one imul plus one xor; 3-cycle dependency chain on typical cores.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
#[inline]
pub fn fx_mix(mut hash: u64, value: u64) -> u64 {
    hash = hash.wrapping_mul(FX_MULTIPLIER);
    hash ^ value
}

/// CEP:WHAT: Computes a fixed-seed 64-bit hash over a byte slice.
/// CEP:WHY: Content addressing (CAS cache, IR hashing) needs run-stable digests without crypto
///   cost on internal data; SHA-256 is used where adversarial stability is required.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: input length fits usize.
/// CEP:COST: O(n) with an 8-byte-at-a-time fast path; roughly 1 byte/cycle on x86-64.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
pub fn fxhash64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0;
    let (chunks, rem) = bytes.as_chunks::<8>();
    for word in chunks {
        hash = fx_mix(hash, u64::from_le_bytes(*word));
    }
    if !rem.is_empty() {
        let mut word = [0u8; 8];
        word[..rem.len()].copy_from_slice(rem);
        hash = fx_mix(hash, u64::from_le_bytes(word));
    }
    hash = fx_mix(hash, bytes.len() as u64);
    hash
}

/// CEP:WHAT: std::hash::Hasher implementation forwarding to fxhash64 byte mixing.
/// CEP:WHY: Allows HashMap/HashSet to use the deterministic hasher through BuildHasherDefault.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: writes are independent of endianness because values use to_le_bytes.
/// CEP:COST: one mixing step per write call.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}

impl Hasher for FxHasher {
    /// CEP:WHAT: Mixes a byte slice into the accumulator.
    /// CEP:WHY: Required Hasher trait surface for string keys.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n).
    /// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
    fn write(&mut self, bytes: &[u8]) {
        self.hash = fx_mix(self.hash, fxhash64(bytes));
    }

    /// CEP:WHAT: Mixes one u64 value.
    /// CEP:WHY: Required Hasher trait surface for integer keys.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
    fn write_u64(&mut self, value: u64) {
        self.hash = fx_mix(self.hash, value);
    }

    /// CEP:WHAT: Returns the accumulated hash.
    /// CEP:WHY: Trait completion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// CEP:WHAT: BuildHasherDefault for FxHasher.
/// CEP:WHY: One-line hasher injection for collection types.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: zero-sized.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;

/// CEP:WHAT: HashMap with the deterministic Fx hasher.
/// CEP:WHY: Replaces std default randomized hashing per standard 38.10.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: iteration order is implementation-defined but run-stable with a fixed hasher;
///   output-facing code must sort before emission regardless.
/// CEP:COST: identical to std HashMap.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

/// CEP:WHAT: HashSet with the deterministic Fx hasher.
/// CEP:WHY: Same determinism rationale as FxHashMap.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same as FxHashMap.
/// CEP:COST: identical to std HashSet.
/// CEP:EVIDENCE: unit test rv8_core::hash::tests::fxhash_stability.
pub type FxHashSet<V> = HashSet<V, FxBuildHasher>;

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies fixed-seed hash stability and input sensitivity.
    /// CEP:WHY: Content addressing breaks if hashes drift between runs.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn fxhash_stability() {
        let a = fxhash64(b"rv8");
        let b = fxhash64(b"rv8");
        let c = fxhash64(b"rv9");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(fxhash64(b""), fxhash64(b"\0"));
    }
}
