// CEP:FILE: crates/rv8_autotuner/src/tuner.rs
// CEP:WHAT: The autotuner proper: a content-addressed configuration cache keyed on
//   (LIR hash, hardware fingerprint, compiler version), the CGT surrogate, and the deterministic
//   funnel-driven search that selects a genome for a key.
// CEP:WHY: Standard 5.3 and 5.10 require cache hits to reproduce prior configurations
//   bit-for-bit, zero allocation on the lookup hot path, and an rng derived from the key so
//   self-hosting stays reproducible.
// CEP:CLASS: CEP-1
// CEP:STATUS: complete
// CEP:FAILURE: lookup is infallible; record and tune return AutotunerError for invalid genomes,
//   non-finite floats, wrong-length features, or empty candidate sets; the search path never
//   panics (survivor selection uses ok_or, not indexing).
// CEP:ASSUMES: callers hash the LIR and hardware fingerprint deterministically (fxhash64/SHA-256
//   upstream); the same TunerKey and same model state always produce the same tuned genome.
// CEP:COST: lookup is a single BTreeMap::get - O(log n) comparisons, ZERO allocation (see
//   CEP:EVIDENCE); tune is O(pool * predict + tiers * pool log pool) with pool =
//   candidates * (1 + LOCAL_MUTATION_ROLLOUTS).
// CEP:EVIDENCE: unit tests tuner::tests::{cache_replay_bit_for_bit, tune_twice_bit_identical,
//   tune_bit_identical_across_fresh_tuners, lookup_large_cache_returns_correct_entry,
//   tune_rejects_empty_candidates, tune_rejects_nan_features, record_rejects_bad_input,
//   tuner_seed_stability, tune_budget_surrogate_filter_works}. Zero-allocation evidence for
//   lookup is code review: the body is exactly `self.cache.get(key)` returning a borrowed
//   Option<&CachedConfig>; no clone, collect, Arc, or Box appears on the path (stable Rust has no
//   allocation-counting harness; the 10k-entry test exercises the exact path at scale).

use crate::acquisition::acquire;
use crate::cgt::Cgt;
use crate::error::AutotunerError;
use crate::funnel::{Funnel, Tier};
use crate::genome::Genome;
use rv8_core::rng::SplitMix64;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// CEP:WHAT: Rotation applied to the hardware fingerprint when deriving the tuner rng seed.
/// CEP:WHY: Named constant mandated by the design (seed from LIR hash + rotate(hw, 17) ^
///   compiler version); rotation decorrelates fingerprint bits from LIR hash bits.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: tuner::tests::tuner_seed_stability.
pub const TUNER_SEED_ROTATE_BITS: u32 = 17;

/// CEP:WHAT: Mutated variants generated per candidate genome to expand the search pool.
/// CEP:WHY: Named hyperparameter; local exploration around each candidate using the key-seeded
///   rng, which is the tuner's only randomness source.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: pool size = candidates * (1 + LOCAL_MUTATION_ROLLOUTS) stays affordable.
/// CEP:COST: one mutated() call and one predict() call per variant.
/// CEP:EVIDENCE: tuner::tests::tune_bit_identical_across_fresh_tuners.
pub const LOCAL_MUTATION_ROLLOUTS: usize = 2;

/// CEP:WHAT: Cache key: LIR content hash, hardware fingerprint, and compiler version.
/// CEP:WHY: Content addressing per standards 5.3/5.10; Ord makes it a BTreeMap key with ordered
///   iteration, so no HashMap iteration order can leak into output.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: upstream hashes are run-stable; field order in Ord is lir_hash, then
///   hw_fingerprint, then compiler_version (declaration order).
/// CEP:COST: 24 bytes, Copy.
/// CEP:EVIDENCE: tuner::tests::{cache_replay_bit_for_bit, lookup_large_cache_returns_correct_entry}.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TunerKey {
    /// Hash of the level-3 IR for the unit being tuned.
    pub lir_hash: u64,
    /// Hardware fingerprint of the target machine.
    pub hw_fingerprint: u64,
    /// Compiler version tag participating in cache identity.
    pub compiler_version: u64,
}

/// CEP:WHAT: A cached configuration: the winning genome and its measured cost delta.
/// CEP:WHY: A cache hit must reproduce the prior configuration bit-for-bit, including the
///   measurement that justified it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; cloning copies floats bit-exactly.
/// CEP:ASSUMES: observed_delta is finite (enforced by Tuner::record).
/// CEP:COST: genome vectors, cloned only on explicit retrieval.
/// CEP:EVIDENCE: tuner::tests::cache_replay_bit_for_bit.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedConfig {
    /// The recorded genome; replayed bit-for-bit on cache hits.
    pub genome: Genome,
    /// The observed cost delta that the measurement tier produced.
    pub observed_delta: f64,
}

/// CEP:WHAT: The autotuner: cache + surrogate + funnel in one deterministic unit.
/// CEP:WHY: One owner of tuning state keeps the cache/model/funnel invariants enforceable.
/// CEP:STATUS: complete
/// CEP:FAILURE: see method contracts.
/// CEP:ASSUMES: a Tuner is not shared across threads (BTreeMap is not Sync); the driver owns
///   synchronization.
/// CEP:COST: one Cgt (~119 KiB weights) plus the cache payload.
/// CEP:EVIDENCE: tuner::tests::*.
#[derive(Debug)]
pub struct Tuner {
    cache: BTreeMap<TunerKey, CachedConfig>,
    model: Cgt,
    funnel: Funnel,
}

/// CEP:WHAT: Derives the tuner rng seed from the key: lir_hash ^ rotate(hw, 17) ^ version.
/// CEP:WHY: The design mandates the exact seed composition; exposing it makes the derivation
///   testable and keeps every tuner randomness source content-derived.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: three xors and one rotate.
/// CEP:EVIDENCE: tuner::tests::tuner_seed_stability.
pub fn tuner_seed(key: &TunerKey) -> u64 {
    key.lir_hash ^ key.hw_fingerprint.rotate_left(TUNER_SEED_ROTATE_BITS) ^ key.compiler_version
}

impl Tuner {
    /// CEP:WHAT: Creates an empty tuner with the default-seeded surrogate and funnel.
    /// CEP:WHY: Identical constructions must be bit-identical (no hidden state).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(CGT weights).
    /// CEP:EVIDENCE: tuner::tests::tune_bit_identical_across_fresh_tuners.
    pub fn new() -> Tuner {
        Tuner { cache: BTreeMap::new(), model: Cgt::new(), funnel: Funnel::new() }
    }

    /// CEP:WHAT: Cache lookup for a key.
    /// CEP:WHY: The hot path of the content-addressed configuration cache.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; None on miss.
    /// CEP:ASSUMES: ZERO ALLOCATION: the body is exactly BTreeMap::get returning a reference;
    ///   no clone, collect, Box, or Arc is reachable from this function (code-review evidence;
    ///   stable Rust cannot count allocations in a test, so the 10k-entry functional test plus
    ///   this review statement is the evidence).
    /// CEP:COST: O(log n) key comparisons; zero heap traffic.
    /// CEP:EVIDENCE: tuner::tests::lookup_large_cache_returns_correct_entry exercises the exact
    ///   function at 10k entries; the body is a single expression (review).
    pub fn lookup(&self, key: &TunerKey) -> Option<&CachedConfig> {
        self.cache.get(key)
    }

    /// CEP:WHAT: Records a measured configuration under a key, replacing any prior entry.
    /// CEP:WHY: This is how a cache entry that replays bit-for-bit comes to exist.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: genome validation errors; NonFiniteInput for a non-finite observed_delta.
    /// CEP:ASSUMES: the caller measured observed_delta at the budget tier.
    /// CEP:COST: O(log n) insert plus genome move; no clone.
    /// CEP:EVIDENCE: tuner::tests::{cache_replay_bit_for_bit, record_rejects_bad_input}.
    pub fn record(
        &mut self,
        key: TunerKey,
        genome: Genome,
        observed_delta: f64,
    ) -> Result<(), AutotunerError> {
        genome.validate()?;
        if !observed_delta.is_finite() {
            return Err(AutotunerError::NonFiniteInput);
        }
        self.cache.insert(key, CachedConfig { genome, observed_delta });
        Ok(())
    }

    /// CEP:WHAT: Tunes for a key: returns the cached genome on a cache hit; otherwise validates
    ///   inputs, expands the candidate pool with key-seeded mutations, scores every pool entry
    ///   with the surrogate + acquisition at the budget tier's cost, walks the funnel tiers up to
    ///   the budget (filtering at each tier, including the budget tier), and returns the best
    ///   surviving genome.
    /// CEP:WHY: This is the end-to-end deterministic entry point of the autotuner.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: EmptyInput for an empty candidate set; surrogate/acquisition/funnel errors
    ///   propagate (non-finite features, invalid genomes, invalid scores); never panics.
    /// CEP:ASSUMES: DETERMINISM CONTRACT: the rng is SplitMix64 seeded from
    ///   tuner_seed(&key); predictions depend only on (model state, features, genome); the funnel
    ///   is pure - therefore identical (key, model state, features, candidates, budget) produce
    ///   bit-identical output. tune() never fits the model and never writes the cache; callers
    ///   record measured results explicitly.
    /// CEP:COST: O(pool * predict + 4 * pool log pool), pool = n * (1 + LOCAL_MUTATION_ROLLOUTS).
    /// CEP:EVIDENCE: tuner::tests::{tune_twice_bit_identical, tune_bit_identical_across_fresh_tuners,
    ///   tune_budget_surrogate_filter_works, tune_rejects_empty_candidates, tune_rejects_nan_features}.
    pub fn tune(
        &mut self,
        key: TunerKey,
        features: &[f64],
        candidate_genomes: &[Genome],
        budget_tier: Tier,
    ) -> Result<Genome, AutotunerError> {
        // Cache hit: reproduce the prior configuration bit-for-bit (acceptance criterion).
        if let Some(cached) = self.lookup(&key) {
            return Ok(cached.genome.clone());
        }
        if candidate_genomes.is_empty() {
            return Err(AutotunerError::EmptyInput);
        }
        for g in candidate_genomes {
            g.validate()?;
        }
        // All tuner randomness flows from this key-derived seed; no clock, no addresses.
        let mut rng = SplitMix64::seed_from_u64(tuner_seed(&key));
        let mut pool: Vec<Genome> = Vec::with_capacity(
            candidate_genomes.len() * (1 + LOCAL_MUTATION_ROLLOUTS),
        );
        pool.extend(candidate_genomes.iter().cloned());
        for g in candidate_genomes {
            for _ in 0..LOCAL_MUTATION_ROLLOUTS {
                pool.push(g.mutated(&mut rng)?);
            }
        }
        // Score the pool once: acquisition at the budget tier's fidelity cost.
        let mut scores: Vec<f64> = Vec::with_capacity(pool.len());
        for g in &pool {
            let p = self.model.predict(features, g)?;
            scores.push(acquire(p.delta_mean, p.delta_var, p.value, budget_tier.cost())?);
        }
        // Walk the funnel: filter at every tier from SurrogateFilter through budget_tier.
        let mut tier = Tier::SurrogateFilter;
        let mut alive: Vec<usize> = (0..pool.len()).collect();
        loop {
            let scored: Vec<(usize, f64)> = alive.iter().map(|&i| (i, scores[i])).collect();
            let kept = self.funnel.filter(tier, &scored)?;
            match tier.next_tier() {
                Some(next) if next <= budget_tier => {
                    alive = kept;
                    tier = next;
                }
                _ => {
                    // tier == budget_tier: pick the best survivor (score desc, index asc).
                    let best = kept
                        .iter()
                        .copied()
                        .min_by(|&a, &b| {
                            scores[b]
                                .partial_cmp(&scores[a])
                                .unwrap_or(Ordering::Equal)
                                .then(a.cmp(&b))
                        })
                        .ok_or(AutotunerError::EmptyInput)?;
                    return Ok(pool[best].clone());
                }
            }
        }
    }

    /// CEP:WHAT: Number of entries currently cached.
    /// CEP:WHY: Diagnostics and the large-cache functional test.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: tuner::tests::lookup_large_cache_returns_correct_entry.
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// CEP:WHAT: Read access to the surrogate model (for fit/measure drivers).
    /// CEP:WHY: Measurement loops call Cgt::fit through this handle without exposing internals.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: callers keep tuning deterministic (fit outside tune()).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: tuner::tests::tune_budget_surrogate_filter_works.
    pub fn model(&self) -> &Cgt {
        &self.model
    }

    /// CEP:WHAT: Read access to the funnel.
    /// CEP:WHY: Drivers query keep fractions and costs for scheduling.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: tuner::tests::tune_budget_surrogate_filter_works.
    pub fn funnel(&self) -> &Funnel {
        &self.funnel
    }
}

impl Default for Tuner {
    /// CEP:WHAT: Default construction identical to Tuner::new.
    /// CEP:WHY: Idiomatic Default; clippy new_without_default.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: same as new.
    /// CEP:EVIDENCE: tuner::tests::tune_bit_identical_across_fresh_tuners.
    fn default() -> Tuner {
        Tuner::new()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::cgt::FEATURE_DIM;

    /// CEP:WHAT: The shared cache key used across tests.
    /// CEP:WHY: Test fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn key(seed: u64) -> TunerKey {
        TunerKey { lir_hash: seed, hw_fingerprint: seed.wrapping_mul(31), compiler_version: 7 }
    }

    /// CEP:WHAT: A genome with full-precision irrational-ish floats for bit comparisons.
    /// CEP:WHY: Test fixture; 1/sqrt primes exercise low mantissa bits.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn precise_genome(n: usize, salt: u64) -> Genome {
        Genome::new(
            (0..n).map(|i| ((i as f64) + 1.0) / (7.0 + salt as f64) + fractal_jitter(i, salt)).collect(),
            (0..n).map(|i| 1 + ((i as u32 + salt as u32) % 8)).collect(),
            (0..n).map(|i| ((i as f64) + 1.0).sqrt() / (3.0 + salt as f64)).collect(),
        )
        .expect("fixture genome must validate")
    }

    /// CEP:WHAT: Tiny deterministic jitter so floats carry full mantissas.
    /// CEP:WHY: Test fixture helper.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn fractal_jitter(i: usize, salt: u64) -> f64 {
        let x = ((i as u64).wrapping_mul(0x9E37_79B9) ^ salt) as f64 / u64::MAX as f64;
        x * 1e-6
    }

    /// CEP:WHAT: A finite feature vector for tests.
    /// CEP:WHY: Test fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn features(salt: u64) -> Vec<f64> {
        let mut rng = SplitMix64::seed_from_u64(salt);
        (0..FEATURE_DIM).map(|_| rng.next_f64()).collect()
    }

    /// CEP:WHAT: ACCEPTANCE: a recorded configuration is replayed bit-for-bit by lookup: every
    ///   f64 compared through to_bits, iterations compared exactly, observed_delta via to_bits.
    /// CEP:WHY: The section 5.3 acceptance check "cache hit reproduces a prior configuration
    ///   bit-for-bit".
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn cache_replay_bit_for_bit() {
        let mut tuner = Tuner::new();
        let k = key(0xDEAD_BEEF);
        let genome = precise_genome(12, 3);
        let observed = -0.137_029_619_777_141_5_f64;
        tuner.record(k, genome.clone(), observed).unwrap();
        let cached = tuner.lookup(&k).expect("recorded key must hit");
        assert!(cached.genome.bit_identical(&genome));
        for (a, b) in cached.genome.logits.iter().zip(genome.logits.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        assert_eq!(cached.genome.iterations, genome.iterations);
        for (a, b) in cached.genome.factors.iter().zip(genome.factors.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        assert_eq!(cached.observed_delta.to_bits(), observed.to_bits());
        // -0.0 vs 0.0 would pass == but must fail the bit contract.
        tuner.record(key(2), Genome::new(vec![0.0], vec![1], vec![0.0]).unwrap(), 0.0).unwrap();
        tuner.record(key(3), Genome::new(vec![-0.0], vec![1], vec![-0.0]).unwrap(), -0.0).unwrap();
        assert!(!tuner.lookup(&key(2)).unwrap().genome.bit_identical(
            &tuner.lookup(&key(3)).unwrap().genome
        ));
    }

    /// CEP:WHAT: Calling tune twice with identical key and inputs returns bit-identical genomes;
    ///   after recording, tune returns the recorded genome exactly (cache-hit path).
    /// CEP:WHY: Acceptance criterion 1, search-path and cache-path halves.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tune_twice_bit_identical() {
        let mut tuner = Tuner::new();
        let k = key(1000);
        let f = features(1001);
        let candidates: Vec<Genome> = (0..6).map(|i| precise_genome(6, i)).collect();
        let a = tuner.tune(k, &f, &candidates, Tier::CycleSim).unwrap();
        let b = tuner.tune(k, &f, &candidates, Tier::CycleSim).unwrap();
        assert!(a.bit_identical(&b));
        assert!(a.validate().is_ok());
        // Record the winner; from now on tune must replay it exactly.
        tuner.record(k, a.clone(), -1.5).unwrap();
        let c = tuner.tune(k, &f, &candidates, Tier::Silicon).unwrap();
        assert!(c.bit_identical(&a));
    }

    /// CEP:WHAT: Two freshly constructed Tuners produce bit-identical tuned genomes for the same
    ///   key and inputs (search-path determinism independent of cache state).
    /// CEP:WHY: Same seed -> same everything, across instances.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tune_bit_identical_across_fresh_tuners() {
        let k = key(2000);
        let f = features(2001);
        let candidates: Vec<Genome> = (0..5).map(|i| precise_genome(5, 10 + i)).collect();
        let mut t1 = Tuner::new();
        let mut t2 = Tuner::new();
        let g1 = t1.tune(k, &f, &candidates, Tier::PartialIr).unwrap();
        let g2 = t2.tune(k, &f, &candidates, Tier::PartialIr).unwrap();
        assert!(g1.bit_identical(&g2));
    }

    /// CEP:WHAT: lookup on a 10k-entry cache returns the correct entry by bits and None for a
    ///   missing key; exercises the zero-allocation lookup path at scale.
    /// CEP:WHY: Required evidence for the no-alloc lookup claim (see CEP:EVIDENCE on lookup).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: seeded construction keeps the fixture deterministic.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lookup_large_cache_returns_correct_entry() {
        let mut tuner = Tuner::new();
        let mut rng = SplitMix64::seed_from_u64(4242);
        let n = 10_000;
        let probe_idx = 5432;
        let mut probe_key = key(0);
        let mut probe_genome = precise_genome(1, 0);
        for i in 0..n {
            let k = TunerKey {
                lir_hash: rng.next_u64(),
                hw_fingerprint: rng.next_u64(),
                compiler_version: i as u64,
            };
            let g = precise_genome(2, (i % 97) as u64 + 1);
            tuner.record(k, g, -(i as f64) * 0.5).unwrap();
            if i == probe_idx {
                probe_key = k;
                probe_genome = precise_genome(2, (i % 97) as u64 + 1);
            }
        }
        assert_eq!(tuner.cache_len(), n);
        let cached = tuner.lookup(&probe_key).expect("probe key must hit");
        assert!(cached.genome.bit_identical(&probe_genome));
        assert_eq!(
            cached.observed_delta.to_bits(),
            (-(probe_idx as f64) * 0.5).to_bits()
        );
        assert!(tuner.lookup(&key(u64::MAX)).is_none());
    }

    /// CEP:WHAT: tune fails closed on an empty candidate set and on NaN features.
    /// CEP:WHY: Zero-candidate and NaN handling must be Err, never panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tune_rejects_empty_candidates() {
        let mut tuner = Tuner::new();
        let err = tuner.tune(key(1), &features(2), &[], Tier::Silicon).unwrap_err();
        assert_eq!(err, AutotunerError::EmptyInput);
        let mut f = features(3);
        f[0] = f64::NAN;
        let candidates = vec![precise_genome(4, 1)];
        assert_eq!(
            tuner.tune(key(4), &f, &candidates, Tier::Silicon).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
    }

    /// CEP:WHAT: tune rejects candidate genomes that fail validation (NaN logit).
    /// CEP:WHY: Invalid genomes must not reach the surrogate.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tune_rejects_nan_features() {
        let mut tuner = Tuner::new();
        let bad = Genome { logits: vec![f64::NAN], iterations: vec![1], factors: vec![] };
        let err = tuner.tune(key(9), &features(10), &[bad], Tier::PartialIr).unwrap_err();
        assert_eq!(err, AutotunerError::NonFiniteInput);
    }

    /// CEP:WHAT: record rejects invalid genomes and non-finite deltas.
    /// CEP:WHY: The cache must never hold a configuration that cannot replay validly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn record_rejects_bad_input() {
        let mut tuner = Tuner::new();
        let bad = Genome { logits: vec![f64::INFINITY], iterations: vec![], factors: vec![] };
        assert_eq!(
            tuner.record(key(5), bad, 1.0).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
        let good = precise_genome(2, 1);
        assert_eq!(
            tuner.record(key(5), good, f64::NAN).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
        assert_eq!(tuner.cache_len(), 0, "failed records must not insert");
    }

    /// CEP:WHAT: The tuner seed is stable for identical keys and changes with any field.
    /// CEP:WHY: The seed composition is normative (lir ^ rotate(hw, 17) ^ version).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tuner_seed_stability() {
        let a = TunerKey { lir_hash: 11, hw_fingerprint: 22, compiler_version: 33 };
        let b = TunerKey { lir_hash: 11, hw_fingerprint: 22, compiler_version: 33 };
        assert_eq!(tuner_seed(&a), tuner_seed(&b));
        assert_eq!(tuner_seed(&a), 11u64 ^ 22u64.rotate_left(TUNER_SEED_ROTATE_BITS) ^ 33);
        let c = TunerKey { hw_fingerprint: 23, ..a };
        assert_ne!(tuner_seed(&a), tuner_seed(&c));
    }

    /// CEP:WHAT: A SurrogateFilter-budget tune succeeds, returns a valid genome, and the funnel
    ///   handle reports the mandated tier-0 keep fraction.
    /// CEP:WHY: The lowest budget must still work end to end.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tune_budget_surrogate_filter_works() {
        let mut tuner = Tuner::new();
        let candidates: Vec<Genome> = (0..9).map(|i| precise_genome(4, 20 + i)).collect();
        let g = tuner
            .tune(key(77), &features(78), &candidates, Tier::SurrogateFilter)
            .unwrap();
        assert!(g.validate().is_ok());
        assert_eq!(
            tuner.funnel().keep_count(Tier::SurrogateFilter, candidates.len()),
            1
        );
        assert_eq!(tuner.funnel().keep_count(Tier::SurrogateFilter, 25), 3);
        assert!(tuner.model().predict(&features(78), &g).is_ok());
    }
}
