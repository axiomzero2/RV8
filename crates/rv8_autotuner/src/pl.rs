// CEP:FILE: crates/rv8_autotuner/src/pl.rs
// CEP:WHAT: Plackett-Luce pass-order sampling without replacement and exact order log-probability.
// CEP:WHY: Standard 5.3 defines the pass-ordering search space as Plackett-Luce logits; sampling
//   must be sequential softmax without replacement and exactly invertible through its
//   log-probability for scoring candidate orders.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: sample_order returns None for empty or non-finite logits; order_log_probability
//   returns NEG_INFINITY for invalid permutations or non-finite logits; no panic paths.
// CEP:ASSUMES: the rng (SplitMix64) is caller-seeded; exactly one uniform draw is consumed per
//   selected pass, so identical rng state yields identical orders.
// CEP:COST: sample_order is O(n^2) worst case over at most MAX_PASS_COUNT passes (176); each step
//   is one softmax over remaining candidates. order_log_probability is O(n^2) with stable
//   max-shifted exponentials.
// CEP:EVIDENCE: unit tests pl::tests::{uniform_four_enumerates_all_24, peaked_prefers_peaked,
//   deterministic_given_fixed_seed, empty_returns_none, non_finite_returns_none,
//   uniform_logprob_is_log_factorial, peaked_order_beats_swap}.

use rv8_core::rng::SplitMix64;

/// CEP:WHAT: Samples a full pass order by sequential softmax sampling without replacement.
/// CEP:WHY: The Plackett-Luce ranking model: repeatedly draw from softmax over the remaining
///   passes; the draw sequence is the sampled permutation and its probability factors exactly.
/// CEP:STATUS: complete
/// CEP:FAILURE: None when `logits` is empty or contains a non-finite value (fail closed; NaN
///   weights would otherwise silently corrupt the sampling distribution).
/// CEP:ASSUMES: logits are finite; rng draws are the only randomness (one next_f64 per pick).
/// CEP:COST: O(n^2) time, O(n) space, n <= MAX_PASS_COUNT.
/// CEP:EVIDENCE: pl::tests::{uniform_four_enumerates_all_24, peaked_prefers_peaked,
///   deterministic_given_fixed_seed}.
pub fn sample_order(logits: &[f64], rng: &mut SplitMix64) -> Option<Vec<usize>> {
    if logits.is_empty() || logits.iter().any(|l| !l.is_finite()) {
        return None;
    }
    let mut remaining: Vec<usize> = (0..logits.len()).collect();
    let mut order = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let weights: Vec<f64> = remaining.iter().map(|&i| logits[i]).collect();
        let pos = rng.sample_softmax(&weights)?;
        order.push(remaining.remove(pos));
    }
    Some(order)
}

/// CEP:WHAT: Exact log-probability of a pass order under the Plackett-Luce model:
///   `sum_i logits[order[i]] - log(sum_{j in remaining_i} exp(logits[j]))`.
/// CEP:WHY: Scoring sampled orders and warm-start priors needs the exact model density.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns NEG_INFINITY when logits is non-empty with a wrongly sized order, when
///   order is not a permutation of 0..n, or when any logit is non-finite; returns 0.0 for two
///   empty slices (probability 1 of the empty order).
/// CEP:ASSUMES: finite logits; max-shifted exponentials keep exp() in range, so no overflow.
/// CEP:COST: O(n^2) time, O(n) space.
/// CEP:EVIDENCE: pl::tests::{uniform_logprob_is_log_factorial, peaked_order_beats_swap}.
pub fn order_log_probability(logits: &[f64], order: &[usize]) -> f64 {
    let n = logits.len();
    if n == 0 {
        return if order.is_empty() { 0.0 } else { f64::NEG_INFINITY };
    }
    if order.len() != n || logits.iter().any(|l| !l.is_finite()) {
        return f64::NEG_INFINITY;
    }
    let mut placed = vec![false; n];
    let mut total = 0.0f64;
    for &idx in order {
        if idx >= n || placed[idx] {
            return f64::NEG_INFINITY;
        }
        // The candidate set for this step is every unplaced index, including idx itself.
        let mut max_w = f64::NEG_INFINITY;
        for i in 0..n {
            if !placed[i] {
                max_w = max_w.max(logits[i]);
            }
        }
        let mut z = 0.0f64;
        for i in 0..n {
            if !placed[i] {
                z += (logits[i] - max_w).exp();
            }
        }
        let log_z = max_w + z.ln();
        total += logits[idx] - log_z;
        placed[idx] = true;
    }
    total
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Collects permutations over 4 uniform-logit passes across many fixed seeds.
    /// CEP:WHY: Uniform logits must be able to generate all 24 permutations.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: 1500 deterministic seeds are ample to cover 24 equally likely permutations.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn uniform_four_enumerates_all_24() {
        let logits = [0.0f64; 4];
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..1500u64 {
            let mut rng = SplitMix64::seed_from_u64(seed);
            let order = sample_order(&logits, &mut rng).unwrap();
            assert_eq!(order.len(), 4);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, vec![0, 1, 2, 3], "order must be a permutation");
            seen.insert(order);
        }
        assert_eq!(seen.len(), 24, "all 24 permutations must appear across seeds");
    }

    /// CEP:WHAT: Peaked logits strongly prefer the peaked order.
    /// CEP:WHY: PL sampling must concentrate on high-logit passes first.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: exp(8) ratio makes non-peaked first picks vanishingly rare.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn peaked_prefers_peaked() {
        let logits = [8.0, 0.0, 0.0, 0.0];
        let mut peaked_first = 0;
        let runs = 500;
        for seed in 0..runs as u64 {
            let mut rng = SplitMix64::seed_from_u64(10_000 + seed);
            let order = sample_order(&logits, &mut rng).unwrap();
            if order[0] == 0 {
                peaked_first += 1;
            }
        }
        assert!(peaked_first > 490, "peaked pass should lead almost always: {peaked_first}/{runs}");
    }

    /// CEP:WHAT: Identical seeds produce identical orders; different seeds (almost surely)
    ///   produce different order streams.
    /// CEP:WHY: Determinism contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn deterministic_given_fixed_seed() {
        let logits = [0.3, -1.0, 2.2, 0.0, 1.1];
        let mut a = SplitMix64::seed_from_u64(42);
        let mut b = SplitMix64::seed_from_u64(42);
        for _ in 0..20 {
            assert_eq!(sample_order(&logits, &mut a), sample_order(&logits, &mut b));
        }
    }

    /// CEP:WHAT: Empty logits yield None, never a panic.
    /// CEP:WHY: Fail-closed contract for the empty search space.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn empty_returns_none() {
        let mut rng = SplitMix64::seed_from_u64(1);
        assert!(sample_order(&[], &mut rng).is_none());
    }

    /// CEP:WHAT: Non-finite logits yield None (NaN cannot enter the sampler).
    /// CEP:WHY: Float hygiene rule: reject non-finite inputs at the boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn non_finite_returns_none() {
        let mut rng = SplitMix64::seed_from_u64(2);
        assert!(sample_order(&[0.0, f64::NAN], &mut rng).is_none());
        assert!(sample_order(&[0.0, f64::INFINITY], &mut rng).is_none());
    }

    /// CEP:WHAT: Under uniform logits every order has log-probability -ln(n!).
    /// CEP:WHY: Exact PL density check against the closed form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn uniform_logprob_is_log_factorial() {
        let logits = [0.0f64; 4];
        let expected = -(24.0f64).ln();
        for seed in 0..50u64 {
            let mut rng = SplitMix64::seed_from_u64(seed);
            let order = sample_order(&logits, &mut rng).unwrap();
            let lp = order_log_probability(&logits, &order);
            assert!((lp - expected).abs() < 1e-9, "lp={lp} expected={expected}");
        }
        // Sampling probability and density agree: density of a sampled order is exp(lp).
        let mut rng = SplitMix64::seed_from_u64(7);
        let order = sample_order(&logits, &mut rng).unwrap();
        assert!((order_log_probability(&logits, &order).exp() - 1.0 / 24.0).abs() < 1e-12);
    }

    /// CEP:WHAT: The peaked order has strictly higher log-probability than a swapped order.
    /// CEP:WHY: Density must agree with the sampling bias.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn peaked_order_beats_swap() {
        let logits = [2.0, 1.0, 0.5];
        let lp_best = order_log_probability(&logits, &[0, 1, 2]);
        let lp_swap = order_log_probability(&logits, &[2, 1, 0]);
        assert!(lp_best > lp_swap);
        // Invalid permutations fail closed.
        assert_eq!(order_log_probability(&logits, &[0, 0, 1]), f64::NEG_INFINITY);
        assert_eq!(order_log_probability(&logits, &[0, 1]), f64::NEG_INFINITY);
        assert_eq!(order_log_probability(&logits, &[0, 1, 9]), f64::NEG_INFINITY);
        assert_eq!(order_log_probability(&[], &[]), 0.0);
    }
}
