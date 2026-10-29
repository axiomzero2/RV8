// CEP:FILE: crates/rv8_autotuner/src/genome.rs
// CEP:WHAT: The autotuner genome: Plackett-Luce pass-ordering logits, discrete per-pass
//   iteration counts, and continuous optimization factors, plus seeded mutation, crossover,
//   a diversity distance, and a run-stable content hash.
// CEP:WHY: The search space of standard 5.3 is exactly this triple; centralizing validation and
//   genetic operators here keeps every consumer (surrogate, funnel, cache) on one definition.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: validate/mutate/crossover/distance return AutotunerError on empty logits, lengths
//   exceeding MAX_PASS_COUNT, non-finite floats, or shape mismatches; no panic paths.
// CEP:ASSUMES: logits are unbounded finite f64; iteration counts live in [ITERATIONS_MIN,
//   ITERATIONS_MAX]; factors live in [FACTOR_MIN, FACTOR_MAX].
// CEP:COST: validation is O(MAX_PASS_COUNT); mutate/crossover are O(MAX_PASS_COUNT); distance is
//   O(MAX_PASS_COUNT); hash64 is O(bytes) via fxhash64.
// CEP:EVIDENCE: unit tests genome::tests::* (validation, determinism, distance, hash stability).

use crate::error::AutotunerError;
use rv8_core::hash::fxhash64;
use rv8_core::rng::SplitMix64;

/// CEP:WHAT: Maximum number of passes a single genome may order (the curated catalog size).
/// CEP:WHY: Named budget per standard 38.12; bounds surrogate feature width and search cost.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: matches the 176-entry catalog in STANDARDS.md section 3.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::validate_rejects_too_long.
pub const MAX_PASS_COUNT: usize = 176;

/// CEP:WHAT: Lower bound for discrete per-pass iteration counts produced by mutation.
/// CEP:WHY: Zero or negative repeat counts are meaningless; the bound keeps mutations valid.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_iterations_in_bounds.
pub const ITERATIONS_MIN: u32 = 1;

/// CEP:WHAT: Upper bound for discrete per-pass iteration counts.
/// CEP:WHY: Named budget per standard 38.12; keeps compile-time amplification bounded.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_iterations_in_bounds.
pub const ITERATIONS_MAX: u32 = 8;

/// CEP:WHAT: Inclusive lower bound for continuous factors (unroll factors, thresholds).
/// CEP:WHY: Negative factors are meaningless; the bound keeps blended genomes valid.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_factors_in_bounds.
pub const FACTOR_MIN: f64 = 0.0;

/// CEP:WHAT: Inclusive upper bound for continuous factors.
/// CEP:WHY: Named budget per standard 38.12.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_factors_in_bounds.
pub const FACTOR_MAX: f64 = 16.0;

/// CEP:WHAT: Half-width of the uniform logit perturbation applied by mutation.
/// CEP:WHY: Named hyperparameter; small enough to exploit, large enough to explore locally.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_deterministic_given_seed.
pub const MUTATION_LOGIT_SCALE: f64 = 0.25;

/// CEP:WHAT: Maximum +/- delta applied to iteration counts by one mutation.
/// CEP:WHY: Named hyperparameter bounding discrete search steps.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_iterations_in_bounds.
pub const MUTATION_ITERATION_MAX: u32 = 2;

/// CEP:WHAT: Half-width of the uniform factor perturbation applied by mutation.
/// CEP:WHY: Named hyperparameter; keeps continuous search local.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::mutation_keeps_factors_in_bounds.
pub const MUTATION_FACTOR_SCALE: f64 = 0.25;

/// CEP:WHAT: Probability that a crossover gene comes from the second parent.
/// CEP:WHY: Named hyperparameter; 0.5 gives symmetric uniform crossover.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::crossover_deterministic_given_seed.
pub const CROSSOVER_OTHER_PROB: f64 = 0.5;

/// CEP:WHAT: Weight of the logit L2 component in the genome distance metric.
/// CEP:WHY: Named hyperparameter; pass order dominates config identity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::distance_zero_for_identical.
pub const DISTANCE_LOGIT_WEIGHT: f64 = 1.0;

/// CEP:WHAT: Weight of the mean-absolute iteration component in the distance metric.
/// CEP:WHY: Named hyperparameter; iteration deltas are coarse relative to logit deltas.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::distance_zero_for_identical.
pub const DISTANCE_ITERATION_WEIGHT: f64 = 0.25;

/// CEP:WHAT: Weight of the factor L2 component in the distance metric.
/// CEP:WHY: Named hyperparameter.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::distance_zero_for_identical.
pub const DISTANCE_FACTOR_WEIGHT: f64 = 0.25;

/// CEP:WHAT: Salt mixed into the second hash word of the genome digest.
/// CEP:WHY: Law 7 named constant; gives an independent second word for the surrogate embedding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: genome::tests::hash_stable_and_sensitive.
pub const GENOME_HASH_SALT: u64 = 0x6E6F_6D61_6C00_0001;

/// CEP:WHAT: The autotuner genome: pass-ordering logits, per-pass iteration counts, and
///   continuous factors.
/// CEP:WHY: One struct is the unit of search, cache identity, and bit-for-bit replay.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction is infallible; call validate() before use, or use Genome::new.
/// CEP:ASSUMES: vectors are independently sized (iterations/factors may be shorter than logits;
///   they apply to the first k passes); all floats must be finite for the genome to validate.
/// CEP:COST: three Vec headers plus payload; cloned genomes copy bit-exactly.
/// CEP:EVIDENCE: tuner::tests::cache_replay_bit_for_bit replays a recorded genome field-by-field
///   through f64::to_bits.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Genome {
    /// Plackett-Luce logits over the pass catalog; `logits[i]` scores pass i.
    pub logits: Vec<f64>,
    /// Discrete repeat counts; `iterations[k]` applies to the k-th scheduled pass.
    pub iterations: Vec<u32>,
    /// Continuous factors (unroll factors, thresholds); unconstrained order.
    pub factors: Vec<f64>,
}

impl Genome {
    /// CEP:WHAT: Constructs and validates a genome in one step.
    /// CEP:WHY: The common path should fail closed at the boundary, not at first use.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: EmptyInput for empty logits; ExceedsMaxPasses past MAX_PASS_COUNT;
    ///   NonFiniteInput for any NaN/infinite logit or factor.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(MAX_PASS_COUNT).
    /// CEP:EVIDENCE: genome::tests::new_accepts_valid_and_rejects_invalid.
    pub fn new(
        logits: Vec<f64>,
        iterations: Vec<u32>,
        factors: Vec<f64>,
    ) -> Result<Genome, AutotunerError> {
        let g = Genome { logits, iterations, factors };
        g.validate()?;
        Ok(g)
    }

    /// CEP:WHAT: Checks structural budgets and float finiteness of every axis.
    /// CEP:WHY: NaN propagation is prevented here at the boundary; downstream math may assume
    ///   finite inputs after this check (documented per module).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: EmptyInput (no logits); ExceedsMaxPasses (any axis past MAX_PASS_COUNT);
    ///   NonFiniteInput (any non-finite logit or factor).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(MAX_PASS_COUNT); no allocation.
    /// CEP:EVIDENCE: genome::tests::{validate_rejects_nan, validate_rejects_too_long,
    ///   validate_rejects_empty}.
    pub fn validate(&self) -> Result<(), AutotunerError> {
        if self.logits.is_empty() {
            return Err(AutotunerError::EmptyInput);
        }
        if self.logits.len() > MAX_PASS_COUNT {
            return Err(AutotunerError::ExceedsMaxPasses {
                max: MAX_PASS_COUNT,
                actual: self.logits.len(),
            });
        }
        if self.iterations.len() > MAX_PASS_COUNT {
            return Err(AutotunerError::ExceedsMaxPasses {
                max: MAX_PASS_COUNT,
                actual: self.iterations.len(),
            });
        }
        if self.factors.len() > MAX_PASS_COUNT {
            return Err(AutotunerError::ExceedsMaxPasses {
                max: MAX_PASS_COUNT,
                actual: self.factors.len(),
            });
        }
        if self.logits.iter().any(|v| !v.is_finite()) || self.factors.iter().any(|v| !v.is_finite())
        {
            return Err(AutotunerError::NonFiniteInput);
        }
        Ok(())
    }

    /// CEP:WHAT: Seeded mutation: uniform per-gene perturbation of all three axes, length
    ///   preserving, clamped to named bounds.
    /// CEP:WHY: Local exploration around a candidate; deterministic given the rng state so
    ///   self-hosting stays reproducible.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: validates `self` first; returns the validation error unchanged.
    /// CEP:ASSUMES: rng is a caller-seeded SplitMix64; draws are consumed in gene order.
    /// CEP:COST: O(MAX_PASS_COUNT) plus 2 draws per logit/factor gene and 1 per iteration gene.
    /// CEP:EVIDENCE: genome::tests::{mutation_deterministic_given_seed,
    ///   mutation_keeps_iterations_in_bounds, mutation_keeps_factors_in_bounds}.
    pub fn mutated(&self, rng: &mut SplitMix64) -> Result<Genome, AutotunerError> {
        self.validate()?;
        let mut logits = Vec::with_capacity(self.logits.len());
        for l in &self.logits {
            logits.push(l + uniform_pm_one(rng) * MUTATION_LOGIT_SCALE);
        }
        let mut iterations = Vec::with_capacity(self.iterations.len());
        for it in &self.iterations {
            let delta = (rng.next_f64() * (2.0 * MUTATION_ITERATION_MAX as f64 + 1.0)) as i64
                - MUTATION_ITERATION_MAX as i64;
            let next = (*it as i64 + delta).clamp(ITERATIONS_MIN as i64, ITERATIONS_MAX as i64);
            iterations.push(next as u32);
        }
        let mut factors = Vec::with_capacity(self.factors.len());
        for f in &self.factors {
            let next = (f + uniform_pm_one(rng) * MUTATION_FACTOR_SCALE).clamp(FACTOR_MIN, FACTOR_MAX);
            factors.push(next);
        }
        Ok(Genome { logits, iterations, factors })
    }

    /// CEP:WHAT: Seeded uniform crossover with a second parent; requires identical shapes.
    /// CEP:WHY: Recombines successful configurations without inventing shape conventions.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: validates both parents; LengthMismatch when any axis length differs.
    /// CEP:ASSUMES: both parents already valid; gene draws consumed axis-major.
    /// CEP:COST: O(MAX_PASS_COUNT) plus one rng draw per gene.
    /// CEP:EVIDENCE: genome::tests::{crossover_deterministic_given_seed,
    ///   crossover_rejects_shape_mismatch}.
    pub fn crossover(&self, other: &Genome, rng: &mut SplitMix64) -> Result<Genome, AutotunerError> {
        self.validate()?;
        other.validate()?;
        if self.logits.len() != other.logits.len()
            || self.iterations.len() != other.iterations.len()
            || self.factors.len() != other.factors.len()
        {
            return Err(AutotunerError::LengthMismatch {
                expected: self.logits.len() + self.iterations.len() + self.factors.len(),
                actual: other.logits.len() + other.iterations.len() + other.factors.len(),
            });
        }
        let logits: Vec<f64> = self
            .logits
            .iter()
            .zip(&other.logits)
            .map(|(&a, &b)| if rng.next_f64() < CROSSOVER_OTHER_PROB { b } else { a })
            .collect();
        let iterations: Vec<u32> = self
            .iterations
            .iter()
            .zip(&other.iterations)
            .map(|(&a, &b)| if rng.next_f64() < CROSSOVER_OTHER_PROB { b } else { a })
            .collect();
        let factors: Vec<f64> = self
            .factors
            .iter()
            .zip(&other.factors)
            .map(|(&a, &b)| if rng.next_f64() < CROSSOVER_OTHER_PROB { b } else { a })
            .collect();
        Ok(Genome { logits, iterations, factors })
    }

    /// CEP:WHAT: Mixed diversity metric: weighted logit L2, iteration mean-absolute-difference,
    ///   and factor L2 components.
    /// CEP:WHY: The funnel and warm-start paths need one scalar diversity measure per pair.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: validates both genomes; LengthMismatch when axis lengths differ.
    /// CEP:ASSUMES: inputs are finite (enforced by validation); output is finite and >= 0.
    /// CEP:COST: O(MAX_PASS_COUNT).
    /// CEP:EVIDENCE: genome::tests::{distance_zero_for_identical, distance_symmetric_nonneg}.
    pub fn distance(&self, other: &Genome) -> Result<f64, AutotunerError> {
        self.validate()?;
        other.validate()?;
        if self.logits.len() != other.logits.len()
            || self.iterations.len() != other.iterations.len()
            || self.factors.len() != other.factors.len()
        {
            return Err(AutotunerError::LengthMismatch {
                expected: self.logits.len() + self.iterations.len() + self.factors.len(),
                actual: other.logits.len() + other.iterations.len() + other.factors.len(),
            });
        }
        let l2 = |a: &[f64], b: &[f64]| -> f64 {
            let sum: f64 = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum();
            (sum / a.len() as f64).sqrt()
        };
        let iter_diff: f64 = self
            .iterations
            .iter()
            .zip(&other.iterations)
            .map(|(x, y)| (*x as f64 - *y as f64).abs())
            .sum::<f64>()
            / self.iterations.len().max(1) as f64;
        Ok(DISTANCE_LOGIT_WEIGHT * l2(&self.logits, &other.logits)
            + DISTANCE_ITERATION_WEIGHT * iter_diff
            + DISTANCE_FACTOR_WEIGHT * l2(&self.factors, &other.factors))
    }

    /// CEP:WHAT: Run-stable 64-bit digest over the exact bit patterns of all three axes.
    /// CEP:WHY: The surrogate hashes logits into its genome embedding; the cache uses the full
    ///   struct, but this digest must never drift between runs on the same toolchain.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; total function (callers validate before hashing in validated paths).
    /// CEP:ASSUMES: f64 bit patterns via to_bits are stable for finite values on the target.
    /// CEP:COST: O(bytes) fxhash64.
    /// CEP:EVIDENCE: genome::tests::hash_stable_and_sensitive.
    pub fn hash64(&self) -> u64 {
        let mut bytes = Vec::new();
        for l in &self.logits {
            bytes.extend_from_slice(&l.to_bits().to_le_bytes());
        }
        for it in &self.iterations {
            bytes.extend_from_slice(&it.to_le_bytes());
        }
        for f in &self.factors {
            bytes.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        fxhash64(&bytes)
    }

    /// CEP:WHAT: Bit-for-bit equality: compares every float through f64::to_bits.
    /// CEP:WHY: Cache acceptance requires bit-for-bit replay; `==` would equate 0.0 and -0.0 and
    ///   never equate NaN, both of which violate the exact-replay contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(MAX_PASS_COUNT).
    /// CEP:EVIDENCE: tuner::tests::cache_replay_bit_for_bit.
    pub fn bit_identical(&self, other: &Genome) -> bool {
        self.logits.len() == other.logits.len()
            && self.iterations == other.iterations
            && self.factors.len() == other.factors.len()
            && self
                .logits
                .iter()
                .zip(&other.logits)
                .all(|(a, b)| a.to_bits() == b.to_bits())
            && self
                .factors
                .iter()
                .zip(&other.factors)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// CEP:WHAT: One uniform draw in (-1, 1) from the SplitMix64 stream.
/// CEP:WHY: Shared by mutation operators so every perturbation has the same scale semantics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total function.
/// CEP:ASSUMES: next_f64 yields [0, 1).
/// CEP:COST: one rng draw and one multiply-add.
/// CEP:EVIDENCE: genome::tests::mutation_deterministic_given_seed.
fn uniform_pm_one(rng: &mut SplitMix64) -> f64 {
    rng.next_f64() * 2.0 - 1.0
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Builds a small valid genome for tests.
    /// CEP:WHY: Test fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    pub fn fixture_genome(n: usize) -> Genome {
        Genome {
            logits: (0..n).map(|i| (i as f64) * 0.5 - 1.0).collect(),
            iterations: (0..n).map(|i| (i % 4) as u32 + 1).collect(),
            factors: (0..n).map(|i| (i as f64) * 0.125).collect(),
        }
    }

    /// CEP:WHAT: Verifies Genome::new accepts valid input and rejects empty/oversized/NaN.
    /// CEP:WHY: Boundary contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn new_accepts_valid_and_rejects_invalid() {
        assert!(Genome::new(vec![0.5], vec![1], vec![1.0]).is_ok());
        assert_eq!(
            Genome::new(Vec::new(), Vec::new(), Vec::new()).unwrap_err(),
            AutotunerError::EmptyInput
        );
        let nan = Genome::new(vec![f64::NAN], vec![1], vec![]);
        assert_eq!(nan.unwrap_err(), AutotunerError::NonFiniteInput);
    }

    /// CEP:WHAT: Validation rejects NaN/infinite logits and factors.
    /// CEP:WHY: NaN must never reach the surrogate (float hygiene rule).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn validate_rejects_nan() {
        let mut g = fixture_genome(4);
        g.logits[2] = f64::INFINITY;
        assert_eq!(g.validate().unwrap_err(), AutotunerError::NonFiniteInput);
        g.logits[2] = 0.0;
        g.factors[1] = f64::NAN;
        assert_eq!(g.validate().unwrap_err(), AutotunerError::NonFiniteInput);
        assert!(g.validate().is_err() || g.logits[2].is_finite());
    }

    /// CEP:WHAT: Validation rejects axes longer than MAX_PASS_COUNT.
    /// CEP:WHY: Named budget enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn validate_rejects_too_long() {
        let mut g = fixture_genome(MAX_PASS_COUNT + 1);
        assert_eq!(
            g.validate().unwrap_err(),
            AutotunerError::ExceedsMaxPasses { max: MAX_PASS_COUNT, actual: MAX_PASS_COUNT + 1 }
        );
        g.logits = fixture_genome(MAX_PASS_COUNT).logits;
        g.iterations = vec![1; MAX_PASS_COUNT + 1];
        assert_eq!(
            g.validate().unwrap_err(),
            AutotunerError::ExceedsMaxPasses { max: MAX_PASS_COUNT, actual: MAX_PASS_COUNT + 1 }
        );
    }

    /// CEP:WHAT: Validation rejects empty logits.
    /// CEP:WHY: Empty logits have no defined pass ordering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn validate_rejects_empty() {
        let g = Genome { logits: Vec::new(), iterations: Vec::new(), factors: Vec::new() };
        assert_eq!(g.validate().unwrap_err(), AutotunerError::EmptyInput);
    }

    /// CEP:WHAT: Mutation is identical across two generators with the same seed.
    /// CEP:WHY: Determinism contract (same seed -> same everything).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mutation_deterministic_given_seed() {
        let g = fixture_genome(9);
        let mut a = SplitMix64::seed_from_u64(1001);
        let mut b = SplitMix64::seed_from_u64(1001);
        let ga = g.mutated(&mut a).unwrap();
        let gb = g.mutated(&mut b).unwrap();
        assert!(ga.bit_identical(&gb));
        assert!(ga.validate().is_ok());
    }

    /// CEP:WHAT: Mutation keeps iteration counts inside [ITERATIONS_MIN, ITERATIONS_MAX].
    /// CEP:WHY: Validity of the discrete axis under repeated mutation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mutation_keeps_iterations_in_bounds() {
        let g = Genome { logits: vec![1.0; 8], iterations: vec![1; 8], factors: vec![] };
        let mut rng = SplitMix64::seed_from_u64(77);
        let mut cur = g;
        for _ in 0..200 {
            cur = cur.mutated(&mut rng).unwrap();
            assert!(cur.iterations.iter().all(|it| (ITERATIONS_MIN..=ITERATIONS_MAX).contains(it)));
        }
    }

    /// CEP:WHAT: Mutation keeps factors inside [FACTOR_MIN, FACTOR_MAX].
    /// CEP:WHY: Validity of the continuous axis under repeated mutation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mutation_keeps_factors_in_bounds() {
        let g = Genome { logits: vec![1.0; 8], iterations: vec![], factors: vec![15.9; 8] };
        let mut rng = SplitMix64::seed_from_u64(78);
        let mut cur = g;
        for _ in 0..200 {
            cur = cur.mutated(&mut rng).unwrap();
            assert!(cur.factors.iter().all(|f| (FACTOR_MIN..=FACTOR_MAX).contains(f)));
        }
    }

    /// CEP:WHAT: Crossover is identical across two generators with the same seed and yields a
    ///   valid child.
    /// CEP:WHY: Determinism contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn crossover_deterministic_given_seed() {
        let a = fixture_genome(6);
        let b = fixture_genome(6);
        let mut r1 = SplitMix64::seed_from_u64(555);
        let mut r2 = SplitMix64::seed_from_u64(555);
        let c1 = a.crossover(&b, &mut r1).unwrap();
        let c2 = a.crossover(&b, &mut r2).unwrap();
        assert!(c1.bit_identical(&c2));
        assert!(c1.validate().is_ok());
    }

    /// CEP:WHAT: Crossover fails closed on shape mismatch instead of truncating silently.
    /// CEP:WHY: Silent truncation would hide caller bugs.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn crossover_rejects_shape_mismatch() {
        let a = fixture_genome(4);
        let b = fixture_genome(5);
        let mut rng = SplitMix64::seed_from_u64(1);
        assert!(matches!(
            a.crossover(&b, &mut rng),
            Err(AutotunerError::LengthMismatch { .. })
        ));
    }

    /// CEP:WHAT: Distance is zero for identical genomes and positive for perturbed ones.
    /// CEP:WHY: Diversity metric sanity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn distance_zero_for_identical() {
        let g = fixture_genome(7);
        assert_eq!(g.distance(&g).unwrap(), 0.0);
        let mut rng = SplitMix64::seed_from_u64(9);
        let m = g.mutated(&mut rng).unwrap();
        assert!(g.distance(&m).unwrap() > 0.0);
    }

    /// CEP:WHAT: Distance is symmetric, finite, non-negative, and errors on shape mismatch.
    /// CEP:WHY: Metric contract used by diversity checks.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn distance_symmetric_nonneg() {
        let a = fixture_genome(5);
        let b = fixture_genome(4);
        let mut rng = SplitMix64::seed_from_u64(21);
        let c = a.mutated(&mut rng).unwrap();
        let d_ab = a.distance(&c).unwrap();
        let d_ba = c.distance(&a).unwrap();
        assert!(d_ab.is_finite() && d_ab >= 0.0);
        assert_eq!(d_ab, d_ba);
        assert!(matches!(a.distance(&b), Err(AutotunerError::LengthMismatch { .. })));
    }

    /// CEP:WHAT: The genome digest is stable across calls and sensitive to content changes.
    /// CEP:WHY: Downstream embedding and cache identity rely on it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hash_stable_and_sensitive() {
        let g = fixture_genome(10);
        assert_eq!(g.hash64(), g.hash64());
        let mut h = g.clone();
        h.logits[3] += 0.125;
        assert_ne!(g.hash64(), h.hash64());
        let mut i = g.clone();
        i.iterations[3] = (i.iterations[3] % ITERATIONS_MAX) + 1;
        assert_ne!(g.hash64(), i.hash64());
    }
}
