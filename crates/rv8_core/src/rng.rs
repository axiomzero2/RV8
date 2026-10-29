// CEP:FILE: crates/rv8_core/src/rng.rs
// CEP:WHAT: A deterministic SplitMix64 pseudo-random number generator with softmax sampling.
// CEP:WHY: The autotuner requires reproducible sampling (standard 38.10); seeds derive from
//   content hashes, never from the system clock.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; total functions.
// CEP:ASSUMES: statistical quality of SplitMix64 suffices for search sampling, not security.
// CEP:COST: next_u64 is one add, one multiply, three xor-shifts: 5-cycle dependency chain.
// CEP:EVIDENCE: unit tests rv8_core::rng::tests::{sequence_stability, softmax_distribution}.
// CEP:SECURITY: not a cryptographic PRNG; must never be used for secrets.

/// CEP:WHAT: Golden-ratio increment constant of SplitMix64.
/// CEP:WHY: Named constant per Law 7.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::rng::tests::sequence_stability.
pub const SPLITMIX_GOLDEN: u64 = 0x9E3779B97F4A7C15;

/// CEP:WHAT: Deterministic SplitMix64 generator state.
/// CEP:WHY: Tiny state, exact reproducibility given the same seed.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: 8 bytes of state.
/// CEP:EVIDENCE: unit test rv8_core::rng::tests::sequence_stability.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// CEP:WHAT: Creates a generator from an explicit 64-bit seed.
    /// CEP:WHY: Callers pass content-derived seeds for reproducibility.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::rng::tests::sequence_stability.
    pub fn seed_from_u64(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// CEP:WHAT: Returns the next 64-bit output and advances state.
    /// CEP:WHY: Core sampling primitive.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: one add, one multiply, three xor-shift steps.
    /// CEP:EVIDENCE: unit test rv8_core::rng::tests::sequence_stability.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(SPLITMIX_GOLDEN);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// CEP:WHAT: Returns the next uniform f64 in [0, 1).
    /// CEP:WHY: Probability sampling for the Plackett-Luce model.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: f64 has a 53-bit mantissa (IEEE 754, documented target property).
    /// CEP:COST: one next_u64 plus one multiply.
    /// CEP:EVIDENCE: unit test rv8_core::rng::tests::softmax_distribution.
    pub fn next_f64(&mut self) -> f64 {
        const FIFTY_THREE_BITS: u64 = (1u64 << 53) - 1;
        let mantissa = self.next_u64() & FIFTY_THREE_BITS;
        mantissa as f64 / (1u64 << 53) as f64
    }

    /// CEP:WHAT: Samples an index according to softmax(weights) using one uniform draw.
    /// CEP:WHY: Plackett-Luce sequential sampling reduces to repeated softmax draws.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns None for an empty weight slice.
    /// CEP:ASSUMES: weights are finite; a maximum weight guard prevents exp overflow.
    /// CEP:COST: O(n) per draw with two passes (max, then accumulate).
    /// CEP:EVIDENCE: unit test rv8_core::rng::tests::softmax_distribution.
    pub fn sample_softmax(&mut self, weights: &[f64]) -> Option<usize> {
        if weights.is_empty() {
            return None;
        }
        let max_w = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mut cum = Vec::with_capacity(weights.len());
        let mut total = 0.0f64;
        for w in weights {
            total += (w - max_w).exp();
            cum.push(total);
        }
        let pick = self.next_f64() * total;
        match cum.iter().position(|&c| pick < c) {
            Some(idx) => Some(idx),
            None => Some(weights.len() - 1),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies identical seeds produce identical sequences.
    /// CEP:WHY: Reproducibility contract of the autotuner.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn sequence_stability() {
        let mut a = SplitMix64::seed_from_u64(42);
        let mut b = SplitMix64::seed_from_u64(42);
        let mut c = SplitMix64::seed_from_u64(43);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(a.next_u64(), c.next_u64());
    }

    /// CEP:WHAT: Verifies softmax sampling concentrates on the maximum weight and stays in range.
    /// CEP:WHY: PL sampling correctness.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: statistical behavior over 1000 samples.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn softmax_distribution() {
        let mut rng = SplitMix64::seed_from_u64(7);
        let weights = [1.0, 1.0, 10.0, 1.0];
        let mut counts = [0usize; 4];
        for _ in 0..1000 {
            let idx = rng.sample_softmax(&weights).unwrap();
            counts[idx] += 1;
        }
        assert_eq!(counts.iter().sum::<usize>(), 1000);
        assert!(counts[2] > 500, "dominant weight should dominate: {counts:?}");
    }
}
