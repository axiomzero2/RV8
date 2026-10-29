// CEP:FILE: crates/rv8_autotuner/src/funnel.rs
// CEP:WHAT: The four-tier fidelity funnel (surrogate filter, partial-IR execution, cycle-accurate
//   simulation, silicon) with per-tier costs, escalation, keep fractions, and deterministic
//   top-k filtering by acquisition score.
// CEP:WHY: Standard 5.3 mandates a 4-tier async funnel that drops cheap-tier losers early; this
//   module is the pure promotion logic shared by Tuner::tune and any async driver.
// CEP:CLASS: CEP-1
// CEP:STATUS: complete
// CEP:FAILURE: filter returns AutotunerError for an empty candidate list or non-finite scores;
//   keep_count is total for every n; no panic paths.
// CEP:ASSUMES: callers pass one (index, score) pair per candidate with unique indices; scores are
//   produced by the acquisition function (finite by construction, re-checked here).
// CEP:COST: filter is O(n log n) for the sort plus O(k) truncation; keep_count and cost are O(1).
// CEP:EVIDENCE: unit tests funnel::tests::{costs_ascending, tier0_filter_respects_keep_fraction,
//   filter_rejects_empty, filter_deterministic, escalation_order, ties_break_toward_lower_index}.

use crate::error::AutotunerError;
use std::cmp::Ordering;

/// CEP:WHAT: Cost of one evaluation at the tier-0 surrogate filter.
/// CEP:WHY: Named hyperparameter; the unit of the cost ladder (ascending by construction).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: relative magnitudes reflect surrogate vs execution vs simulation vs silicon.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::costs_ascending.
pub const TIER0_SURROGATE_COST: f64 = 1.0;

/// CEP:WHAT: Cost of one partial-IR execution (tier 1).
/// CEP:WHY: Named hyperparameter, 16x the surrogate tier.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same ladder as TIER0_SURROGATE_COST.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::costs_ascending.
pub const TIER1_PARTIAL_IR_COST: f64 = 16.0;

/// CEP:WHAT: Cost of one cycle-accurate simulation (tier 2).
/// CEP:WHY: Named hyperparameter, 16x partial-IR.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same ladder.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::costs_ascending.
pub const TIER2_CYCLE_SIM_COST: f64 = 256.0;

/// CEP:WHAT: Cost of one silicon measurement (tier 3).
/// CEP:WHY: Named hyperparameter, 16x simulation; silicon time dominates everything.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same ladder.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::costs_ascending.
pub const TIER3_SILICON_COST: f64 = 4096.0;

/// CEP:WHAT: Fraction of candidates kept after the tier-0 surrogate filter.
/// CEP:WHY: Named constant mandated by the design (drop bottom 90% at tier 0).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: strictly inside (0, 1].
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
pub const TIER0_KEEP_FRACTION: f64 = 0.1;

/// CEP:WHAT: Fraction kept after the tier-1 partial-IR filter.
/// CEP:WHY: Named threshold for tier promotion.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: TIER0_KEEP_FRACTION <= TIER1_KEEP_FRACTION <= TIER2_KEEP_FRACTION <= 1.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
pub const TIER1_KEEP_FRACTION: f64 = 0.25;

/// CEP:WHAT: Fraction kept after the tier-2 cycle-simulation filter.
/// CEP:WHY: Named threshold for tier promotion.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same ladder.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
pub const TIER2_KEEP_FRACTION: f64 = 0.5;

/// CEP:WHAT: Fraction kept at the silicon tier (all finalists are measured).
/// CEP:WHY: Named threshold; silicon never filters, it only ranks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
pub const SILICON_KEEP_FRACTION: f64 = 1.0;

/// CEP:WHAT: Minimum survivors after any filtering tier.
/// CEP:WHY: Rounding to zero survivors would silently kill the search; keep at least one.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
pub const MIN_KEEP_COUNT: usize = 1;

/// CEP:WHAT: One fidelity tier of the async evaluation funnel, ordered cheapest first.
/// CEP:WHY: Budgets and escalation need a total order over tiers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: declaration order is the cost order (SurrogateFilter < PartialIr < CycleSim <
///   Silicon), which costs_ascending pins with a test.
/// CEP:COST: Copy-sized enum.
/// CEP:EVIDENCE: funnel::tests::{costs_ascending, escalation_order}.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Tier 0: surrogate batch filter (drop bottom 90% by predicted value).
    SurrogateFilter,
    /// Tier 1: partial-IR execution.
    PartialIr,
    /// Tier 2: cycle-accurate simulation.
    CycleSim,
    /// Tier 3: silicon measurement.
    Silicon,
}

impl Tier {
    /// CEP:WHAT: Relative cost of one evaluation at this tier.
    /// CEP:WHY: The acquisition function divides expected information by this cost.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: ladder constants are ascending (tested).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: funnel::tests::costs_ascending.
    pub fn cost(self) -> f64 {
        match self {
            Tier::SurrogateFilter => TIER0_SURROGATE_COST,
            Tier::PartialIr => TIER1_PARTIAL_IR_COST,
            Tier::CycleSim => TIER2_CYCLE_SIM_COST,
            Tier::Silicon => TIER3_SILICON_COST,
        }
    }

    /// CEP:WHAT: The next higher tier, or None above Silicon.
    /// CEP:WHY: Escalation walk for the funnel.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: funnel::tests::escalation_order.
    pub fn next_tier(self) -> Option<Tier> {
        match self {
            Tier::SurrogateFilter => Some(Tier::PartialIr),
            Tier::PartialIr => Some(Tier::CycleSim),
            Tier::CycleSim => Some(Tier::Silicon),
            Tier::Silicon => None,
        }
    }
}

/// CEP:WHAT: The stateless, pure funnel: keep-fraction computation, acquisition-score filtering,
///   and tier escalation.
/// CEP:WHY: Pure and deterministic by construction - no clocks, no rng, no interior mutability -
///   so identical inputs always produce identical survivor sets (a hard review requirement).
/// CEP:STATUS: complete
/// CEP:FAILURE: see filter and keep_count contracts.
/// CEP:ASSUMES: zero-sized; sharing one instance across threads is safe.
/// CEP:COST: zero storage.
/// CEP:EVIDENCE: funnel::tests::*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Funnel;

impl Funnel {
    /// CEP:WHAT: Creates the funnel (unit struct; kept for API stability).
    /// CEP:WHY: Callers should not depend on it being zero-sized forever.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: funnel::tests::filter_deterministic.
    pub fn new() -> Funnel {
        Funnel
    }

    /// CEP:WHAT: The keep fraction for a tier, by name.
    /// CEP:WHY: Tuner budgets report fractions without duplicating the tier match.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
    pub fn keep_fraction(&self, tier: Tier) -> f64 {
        match tier {
            Tier::SurrogateFilter => TIER0_KEEP_FRACTION,
            Tier::PartialIr => TIER1_KEEP_FRACTION,
            Tier::CycleSim => TIER2_KEEP_FRACTION,
            Tier::Silicon => SILICON_KEEP_FRACTION,
        }
    }

    /// CEP:WHAT: Number of survivors after filtering `candidate_count` candidates at `tier`:
    ///   ceil(count * keep_fraction(tier)), clamped to [MIN_KEEP_COUNT, candidate_count]; zero
    ///   candidates yield zero.
    /// CEP:WHY: Budget accounting must be predictable before any evaluation happens.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; total function.
    /// CEP:ASSUMES: keep fractions are in (0, 1].
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: funnel::tests::tier0_filter_respects_keep_fraction.
    pub fn keep_count(&self, tier: Tier, candidate_count: usize) -> usize {
        if candidate_count == 0 {
            return 0;
        }
        let kept = (candidate_count as f64 * self.keep_fraction(tier)).ceil() as usize;
        kept.clamp(MIN_KEEP_COUNT, candidate_count)
    }

    /// CEP:WHAT: Filters (index, score) pairs at a tier: sorts by score descending with ties
    ///   broken toward the lower index, and returns the original indices of the top
    ///   keep_count(tier, n) survivors.
    /// CEP:WHY: The tier-promotion primitive used at every funnel level.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: EmptyInput for an empty slice; NonFiniteInput for any non-finite score.
    /// CEP:ASSUMES: indices need not be dense or sorted; output is a deterministic subset.
    /// CEP:COST: O(n log n).
    /// CEP:EVIDENCE: funnel::tests::{tier0_filter_respects_keep_fraction, filter_deterministic,
    ///   ties_break_toward_lower_index}.
    pub fn filter(&self, tier: Tier, scores: &[(usize, f64)]) -> Result<Vec<usize>, AutotunerError> {
        if scores.is_empty() {
            return Err(AutotunerError::EmptyInput);
        }
        if scores.iter().any(|(_, s)| !s.is_finite()) {
            return Err(AutotunerError::NonFiniteInput);
        }
        let mut ranked: Vec<(usize, f64)> = scores.to_vec();
        ranked.sort_by(|a, b| {
            let by_score = b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal);
            by_score.then(a.0.cmp(&b.0))
        });
        ranked.truncate(self.keep_count(tier, ranked.len()));
        Ok(ranked.into_iter().map(|(idx, _)| idx).collect())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Tier costs ascend strictly and match the named constants.
    /// CEP:WHY: The cost ladder is a structural invariant of the funnel.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn costs_ascending() {
        let t = [Tier::SurrogateFilter, Tier::PartialIr, Tier::CycleSim, Tier::Silicon];
        for w in t.windows(2) {
            assert!(w[0].cost() < w[1].cost(), "tier costs must ascend: {}", w[0].cost());
        }
        assert_eq!(Tier::SurrogateFilter.cost(), TIER0_SURROGATE_COST);
        assert_eq!(Tier::Silicon.cost(), TIER3_SILICON_COST);
    }

    /// CEP:WHAT: Tier-0 filtering keeps ceil(n * TIER0_KEEP_FRACTION) candidates (at least one),
    ///   and they are the top-scored ones; keep_count is clamped and zero-safe.
    /// CEP:WHY: The mandated "drop bottom 90%" behavior.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn tier0_filter_respects_keep_fraction() {
        let funnel = Funnel::new();
        assert_eq!(funnel.keep_count(Tier::SurrogateFilter, 10), 1);
        assert_eq!(funnel.keep_count(Tier::SurrogateFilter, 25), 3); // ceil(2.5)
        assert_eq!(funnel.keep_count(Tier::SurrogateFilter, 1), 1);
        assert_eq!(funnel.keep_count(Tier::SurrogateFilter, 0), 0);
        assert_eq!(funnel.keep_count(Tier::Silicon, 10), 10);
        assert_eq!(funnel.keep_count(Tier::PartialIr, 10), 3); // ceil(2.5)
        assert_eq!(funnel.keep_count(Tier::CycleSim, 10), 5);

        let scores: Vec<(usize, f64)> = (0..25).map(|i| (i, (24 - i) as f64)).collect();
        let kept = funnel.filter(Tier::SurrogateFilter, &scores).unwrap();
        assert_eq!(kept.len(), 3);
        assert_eq!(kept, vec![0, 1, 2], "top-scored candidates must survive");
        // The funnel never returns an unfiltered set larger than the keep fraction.
        assert!(kept.len() as f64 <= (25.0 * TIER0_KEEP_FRACTION).ceil());
    }

    /// CEP:WHAT: Filtering an empty candidate list is an Err, never a panic.
    /// CEP:WHY: Zero-candidate handling must fail closed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn filter_rejects_empty() {
        let funnel = Funnel::new();
        assert_eq!(funnel.filter(Tier::SurrogateFilter, &[]).unwrap_err(), AutotunerError::EmptyInput);
    }

    /// CEP:WHAT: Repeated filtering with identical inputs yields identical survivor sets, and
    ///   non-finite scores are rejected.
    /// CEP:WHY: Determinism and float-hygiene contracts.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn filter_deterministic() {
        let funnel = Funnel::new();
        let scores: Vec<(usize, f64)> =
            (0..40).map(|i| (i, ((i as f64 * 7919.0) % 13.0) - 6.0)).collect();
        let a = funnel.filter(Tier::CycleSim, &scores).unwrap();
        let b = funnel.filter(Tier::CycleSim, &scores).unwrap();
        assert_eq!(a, b);
        let mut nan_scores = scores.clone();
        nan_scores[7] = (7, f64::NAN);
        assert_eq!(
            funnel.filter(Tier::CycleSim, &nan_scores).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
    }

    /// CEP:WHAT: Escalation walks SurrogateFilter -> PartialIr -> CycleSim -> Silicon -> None.
    /// CEP:WHY: The 4-tier structure is normative.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn escalation_order() {
        let mut tier = Tier::SurrogateFilter;
        let mut steps = 1;
        while let Some(next) = tier.next_tier() {
            assert!(next > tier);
            tier = next;
            steps += 1;
        }
        assert_eq!(tier, Tier::Silicon);
        assert_eq!(steps, 4);
        assert!(Tier::Silicon.next_tier().is_none());
    }

    /// CEP:WHAT: Equal scores are broken toward the lower index.
    /// CEP:WHY: Deterministic tie-breaking is a hard rule.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn ties_break_toward_lower_index() {
        let funnel = Funnel::new();
        let scores: Vec<(usize, f64)> = (0..20).map(|i| (i, 5.0)).collect();
        let kept = funnel.filter(Tier::SurrogateFilter, &scores).unwrap();
        assert_eq!(kept, vec![0, 1], "all-tied scores must keep the lowest indices");
    }
}
