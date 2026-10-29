// CEP:FILE: crates/rv8_autotuner/src/minhash.rs
// CEP:WHAT: MinHash signatures over u64 item sets, Jaccard estimation from matching minima, and
//   the topological warm start that blends the best-matching prior genome toward the query.
// CEP:WHY: Standard 5.3 mandates MinHash/Jaccard topological warm start: prior IR shapes similar
//   to the query should seed the search near their known-good configurations.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: minhash_signature is total (empty items yield the EMPTY_SET_MINIMUM sentinel);
//   jaccard returns 0.0 for mismatched or empty signatures (incomparable, never NaN);
//   warm_start returns None when no valid prior exists and skips invalid prior genomes; no panic
//   paths and no NaN anywhere.
// CEP:ASSUMES: items are content-derived u64 digests (e.g. fxhash64 over a per-pass topological
//   fingerprint); similarity of signatures estimates Jaccard similarity of the underlying sets.
// CEP:COST: minhash_signature is O(k * |items|) SplitMix64 blocks; jaccard is O(k);
//   warm_start is O(|candidates| * k * |items|).
// CEP:EVIDENCE: unit tests minhash::tests::{jaccard_identical_is_one, jaccard_disjoint_is_zero,
//   jaccard_with_empty_is_zero, signature_length_is_k, signature_deterministic,
//   jaccard_tracks_true_overlap, warm_start_none_on_empty_candidates,
//   warm_start_blends_best_prior, warm_start_deterministic, warm_start_skips_invalid_priors}.

use crate::genome::{Genome, FACTOR_MAX, FACTOR_MIN, ITERATIONS_MAX, ITERATIONS_MIN};
use rv8_core::hash::fx_mix;
use rv8_core::rng::SplitMix64;

/// CEP:WHAT: Signature length used by the warm-start path.
/// CEP:WHY: Named hyperparameter; 32 minima estimate Jaccard with a standard error around 0.09.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: minhash::tests::signature_length_is_k.
pub const WARM_START_K: usize = 32;

/// CEP:WHAT: Seed for the warm-start hash-function streams; both query and priors use it so
///   signatures are comparable.
/// CEP:WHY: Comparable signatures require identical hash function families across calls.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: minhash::tests::warm_start_deterministic.
pub const WARM_START_SEED: u64 = 0x5741_524D_5EED_0003;

/// CEP:WHAT: Blend weight toward the best prior genome (0.8 prior + 0.2 zero-vector fallback).
/// CEP:WHY: Named constant mandated by the design (~80% blend toward the best prior).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: strictly inside (0, 1).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: minhash::tests::warm_start_blends_best_prior.
pub const WARM_START_BLEND: f64 = 0.8;

/// CEP:WHAT: Sentinel minimum for an empty item set.
/// CEP:WHY: min over the empty set is +infinity mathematically; u64::MAX keeps signatures
///   comparable, makes jaccard(X, []) exactly 0.0, and makes two empty sets exactly equal.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: a real splitmix output collides with u64::MAX with probability 2^-64 (accepted).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: minhash::tests::jaccard_with_empty_is_zero.
pub const EMPTY_SET_MINIMUM: u64 = u64::MAX;

/// CEP:WHAT: Salt mixed into every item before per-function hashing.
/// CEP:WHY: Law 7 named constant; decorrelates item digests from the per-function seeds.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: minhash::tests::signature_deterministic.
pub const MINHASH_ITEM_SALT: u64 = 0x6D69_6E68_6173_6801;

/// CEP:WHAT: Computes a k-element MinHash signature: for each of k hash functions derived from
///   the seed stream, the minimum hashed value over the items.
/// CEP:WHY: Signatures turn set similarity into cheap vector comparison.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total function. k = 0 yields an empty signature (incomparable by
///   jaccard).
/// CEP:ASSUMES: identical (items, k, seed) produce identical signatures on every run.
/// CEP:COST: O(k * |items|), one SplitMix64 block per (function, item) pair.
/// CEP:EVIDENCE: minhash::tests::{signature_length_is_k, signature_deterministic}.
pub fn minhash_signature(items: &[u64], k: usize, seed: u64) -> Vec<u64> {
    let mut stream = SplitMix64::seed_from_u64(seed);
    let mut signature = vec![EMPTY_SET_MINIMUM; k];
    for slot in signature.iter_mut() {
        let func_seed = stream.next_u64();
        let mut min = EMPTY_SET_MINIMUM;
        for item in items {
            let h = SplitMix64::seed_from_u64(func_seed ^ fx_mix(MINHASH_ITEM_SALT, *item)).next_u64();
            min = min.min(h);
        }
        *slot = min;
    }
    signature
}

/// CEP:WHAT: Jaccard estimate from matching signature minima: matches / k.
/// CEP:WHY: One comparison per slot estimates the true set Jaccard without materializing sets.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns 0.0 for mismatched signature lengths or empty signatures (incomparable);
///   the result is always finite and in [0, 1]; NaN is unreachable.
/// CEP:ASSUMES: signatures come from the same k and seed.
/// CEP:COST: O(k).
/// CEP:EVIDENCE: minhash::tests::{jaccard_identical_is_one, jaccard_disjoint_is_zero,
///   jaccard_with_empty_is_zero, jaccard_tracks_true_overlap}.
pub fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / a.len() as f64
}

/// CEP:WHAT: Topological warm start: finds the prior whose MinHash signature best matches the
///   query items and returns its genome blended WARM_START_BLEND toward it with a zero-vector
///   fallback prior (i.e. 0.8 * prior + 0.2 * 0).
/// CEP:WHY: Searching near a known-good configuration for a topologically similar IR dominates
///   cold-start search.
/// CEP:STATUS: complete
/// CEP:FAILURE: None when there are no candidates or no candidate passes validation; prior
///   genomes that fail validate() are skipped; the blended result is re-clamped to named bounds
///   so it always validates; no panic paths.
/// CEP:ASSUMES: candidates are (topological items, prior genome) pairs; ties keep the earliest
///   candidate (deterministic linear scan); continuous axes blend toward zero, iteration counts
///   blend then round into [ITERATIONS_MIN, ITERATIONS_MAX].
/// CEP:COST: O(|candidates| * WARM_START_K * |items|).
/// CEP:EVIDENCE: minhash::tests::{warm_start_blends_best_prior, warm_start_deterministic,
///   warm_start_none_on_empty_candidates, warm_start_skips_invalid_priors}.
pub fn warm_start(candidates: &[(Vec<u64>, Genome)], query: &[u64]) -> Option<Genome> {
    if candidates.is_empty() {
        return None;
    }
    let query_sig = minhash_signature(query, WARM_START_K, WARM_START_SEED);
    let mut best: Option<(f64, &Genome)> = None;
    for (items, genome) in candidates {
        if genome.validate().is_err() {
            continue;
        }
        let sig = minhash_signature(items, WARM_START_K, WARM_START_SEED);
        let j = jaccard(&sig, &query_sig);
        if best.is_none_or(|(best_j, _)| j > best_j) {
            best = Some((j, genome));
        }
    }
    let (_, prior) = best?;
    let blend = WARM_START_BLEND;
    let fallback = 0.0f64; // zero-vector fallback prior.
    let logits = prior.logits.iter().map(|l| blend * l + (1.0 - blend) * fallback).collect();
    let iterations = prior
        .iterations
        .iter()
        .map(|it| {
            (blend * *it as f64)
                .round()
                .clamp(ITERATIONS_MIN as f64, ITERATIONS_MAX as f64) as u32
        })
        .collect();
    let factors = prior
        .factors
        .iter()
        .map(|f| (blend * f + (1.0 - blend) * fallback).clamp(FACTOR_MIN, FACTOR_MAX))
        .collect();
    Some(Genome { logits, iterations, factors })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: A signature compared with itself has Jaccard exactly 1.0, and identical item
    ///   sets produce identical signatures.
    /// CEP:WHY: Core MinHash identity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn jaccard_identical_is_one() {
        let items: Vec<u64> = (0..50u64).map(|i| i * 2654435761).collect();
        let sig = minhash_signature(&items, WARM_START_K, WARM_START_SEED);
        assert_eq!(sig.len(), WARM_START_K);
        assert_eq!(jaccard(&sig, &sig), 1.0);
        let sig2 = minhash_signature(&items, WARM_START_K, WARM_START_SEED);
        assert_eq!(sig, sig2);
        assert_eq!(jaccard(&sig, &sig2), 1.0);
    }

    /// CEP:WHAT: Disjoint item sets estimate Jaccard exactly 0.0 (deterministic inputs, no
    ///   minima collisions).
    /// CEP:WHY: The lower end of the similarity range.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: splitmix collisions on these fixed inputs do not occur.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn jaccard_disjoint_is_zero() {
        let a: Vec<u64> = (0..40u64).collect();
        let b: Vec<u64> = (1000..1040u64).collect();
        let sa = minhash_signature(&a, WARM_START_K, WARM_START_SEED);
        let sb = minhash_signature(&b, WARM_START_K, WARM_START_SEED);
        assert_eq!(jaccard(&sa, &sb), 0.0);
    }

    /// CEP:WHAT: jaccard(X, []) is exactly 0.0 and empty-signature comparisons never produce NaN.
    /// CEP:WHY: Mandated negative case; NaN is banned.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn jaccard_with_empty_is_zero() {
        let items: Vec<u64> = (0..20u64).collect();
        let sig = minhash_signature(&items, WARM_START_K, WARM_START_SEED);
        let empty_sig = minhash_signature(&[], WARM_START_K, WARM_START_SEED);
        assert_eq!(jaccard(&sig, &empty_sig), 0.0);
        assert_eq!(jaccard(&empty_sig, &sig), 0.0);
        assert_eq!(jaccard(&[], &[]), 0.0, "empty signatures are incomparable, not equal");
        assert_eq!(jaccard(&sig[..4], &sig), 0.0, "length mismatch is incomparable");
    }

    /// CEP:WHAT: Signature length equals k, including k = 0.
    /// CEP:WHY: API contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn signature_length_is_k() {
        let items: Vec<u64> = vec![1, 2, 3];
        assert_eq!(minhash_signature(&items, 16, 5).len(), 16);
        assert_eq!(minhash_signature(&items, 0, 5).len(), 0);
        assert_eq!(minhash_signature(&[], 9, 5).len(), 9);
    }

    /// CEP:WHAT: Same seed reproduces signatures bit-for-bit; a different seed changes them.
    /// CEP:WHY: Determinism contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn signature_deterministic() {
        let items: Vec<u64> = (0..30u64).map(|i| i * 0x9E37_79B9).collect();
        let a = minhash_signature(&items, WARM_START_K, 123);
        let b = minhash_signature(&items, WARM_START_K, 123);
        assert_eq!(a, b);
        let c = minhash_signature(&items, WARM_START_K, 124);
        assert_ne!(a, c);
    }

    /// CEP:WHAT: The signature Jaccard tracks the true set Jaccard within tolerance.
    /// CEP:WHY: Statistical correctness of the estimator at WARM_START_K.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: tolerance 0.15 at k = 32 (standard error ~0.087 at J = 0.6).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn jaccard_tracks_true_overlap() {
        let a: Vec<u64> = (0..64u64).collect();
        let b: Vec<u64> = (16..80u64).collect();
        let sa = minhash_signature(&a, WARM_START_K, WARM_START_SEED);
        let sb = minhash_signature(&b, WARM_START_K, WARM_START_SEED);
        let estimate = jaccard(&sa, &sb);
        let truth = 48.0 / 80.0; // |A∩B| = 48 (16..64), |A∪B| = 80 (0..80)
        assert!(
            (estimate - truth).abs() < 0.15,
            "estimate {estimate} must track true Jaccard {truth}"
        );
    }

    /// CEP:WHAT: warm_start returns None for an empty candidate list.
    /// CEP:WHY: No prior, no warm start.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn warm_start_none_on_empty_candidates() {
        assert!(warm_start(&[], &[1, 2, 3]).is_none());
    }

    /// CEP:WHAT: The best-Jaccard prior wins and its genome is blended by exactly
    ///   WARM_START_BLEND toward the zero-vector fallback; the result validates.
    /// CEP:WHY: The core warm-start semantics (80% prior, 20% shrinkage to zero).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn warm_start_blends_best_prior() {
        let query: Vec<u64> = (0..40u64).collect();
        let near: Vec<u64> = (0..40u64).collect(); // identical topology
        let far: Vec<u64> = (500..540u64).collect(); // disjoint topology
        let best = Genome::new(
            (0..8).map(|i| (i as f64) + 0.25).collect(),
            vec![4, 8, 1, 2, 3, 4, 5, 6],
            (0..8).map(|i| (i as f64) * 2.0).collect(),
        )
        .unwrap();
        let other = Genome::new(vec![-9.0; 8], vec![1; 8], vec![0.5; 8]).unwrap();
        let candidates = vec![(far.clone(), other.clone()), (near, best.clone())];
        let warmed = warm_start(&candidates, &query).unwrap();
        assert!(warmed.validate().is_ok());
        for (w, b) in warmed.logits.iter().zip(best.logits.iter()) {
            assert_eq!(w.to_bits(), (WARM_START_BLEND * b).to_bits());
        }
        // iterations blend then round into bounds: 4 -> 3.2 -> 3; 8 -> 6.4 -> 6.
        assert_eq!(warmed.iterations, vec![3, 6, 1, 2, 2, 3, 4, 5]);
        for (w, b) in warmed.factors.iter().zip(best.factors.iter()) {
            assert_eq!(w.to_bits(), (WARM_START_BLEND * b).to_bits());
        }
    }

    /// CEP:WHAT: warm_start is bit-identical across repeated calls.
    /// CEP:WHY: Determinism contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn warm_start_deterministic() {
        let query: Vec<u64> = (0..24u64).collect();
        let g = Genome::new(vec![1.5; 4], vec![3; 4], vec![2.0; 4]).unwrap();
        let candidates = vec![(query.clone(), g)];
        let a = warm_start(&candidates, &query).unwrap();
        let b = warm_start(&candidates, &query).unwrap();
        assert!(a.bit_identical(&b));
    }

    /// CEP:WHAT: Priors that fail validation are skipped; the best valid prior is used.
    /// CEP:WHY: Fail-closed handling of corrupt priors without aborting the search.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn warm_start_skips_invalid_priors() {
        let query: Vec<u64> = vec![1, 2, 3];
        let corrupt = Genome { logits: vec![f64::NAN], iterations: vec![1], factors: vec![] };
        let valid = Genome::new(vec![2.0], vec![2], vec![4.0]).unwrap();
        let candidates = vec![(query.clone(), corrupt.clone()), (query.clone(), valid.clone())];
        let warmed = warm_start(&candidates, &query).unwrap();
        assert_eq!(warmed.logits[0].to_bits(), (WARM_START_BLEND * 2.0).to_bits());
        // All-invalid priors yield None rather than a poisoned genome.
        let only_bad = vec![(query.clone(), corrupt)];
        assert!(warm_start(&only_bad, &query).is_none());
    }
}
