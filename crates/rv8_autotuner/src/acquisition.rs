// CEP:FILE: crates/rv8_autotuner/src/acquisition.rs
// CEP:WHAT: The multi-fidelity acquisition function: expected-information-per-unit-cost proxy
//   computed from surrogate outputs (delta mean, delta variance, value) and a fidelity cost.
// CEP:WHY: Standard 5.3 mandates MF-PES acquisition divided by fidelity cost; this function is
//   the single decision rule that ranks candidate genomes for funnel escalation.
// CEP:CLASS: CEP-2
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-130
// CEP:WHY-PARTIAL: honest approximation statement - full multi-fidelity predictive entropy search
//   (MF-PES) estimates the expected information gain about the global optimum under a
//   moment-matched posterior, typically by Monte-Carlo integration over an entropic posterior
//   approximation; this implementation uses the normative proxy score
//   value * (1 + CONFIDENCE_BONUS_SCALE * delta_var) / fidelity_cost: high predicted value
//   weighted by an epistemic-uncertainty confidence bonus, divided by the cost of the fidelity
//   tier. It captures the explore/exploit/cost tradeoff directionally but is not a computed
//   information gain; replacing the numerator with a true E[IG] estimate is future work.
// CEP:FAILURE: returns AutotunerError for any non-finite input, a non-positive fidelity cost,
//   a negative delta variance, or a numerator that overflows to non-finite; never panics.
// CEP:ASSUMES: value is signed (higher is better); delta_var >= 0; fidelity_cost > 0 and comes
//   from Tier::cost().
// CEP:COST: O(1); four comparisons, three multiplies.
// CEP:EVIDENCE: unit tests acquisition::tests::{rejects_zero_cost, rejects_nan_inputs,
//   rejects_negative_variance, monotone_in_value_variance_cost, finite_for_typical_inputs}.

use crate::error::AutotunerError;

/// CEP:WHAT: Weight of the epistemic variance in the confidence bonus.
/// CEP:WHY: Named hyperparameter; converts the delta variance into a multiplicative exploration
///   bonus. 0.5 means one unit of variance adds 50% expected-gain mass.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: acquisition::tests::monotone_in_value_variance_cost.
pub const CONFIDENCE_BONUS_SCALE: f64 = 0.5;

/// CEP:WHAT: Multi-fidelity acquisition score for one candidate at one fidelity tier.
/// CEP:WHY: Candidates promising high value under high uncertainty at low fidelity cost are the
///   ones worth escalating; the score ranks them in one comparable unit.
/// CEP:STATUS: partial (MF-PES proxy, not computed information gain - see CEP:WHY-PARTIAL on the
///   file header)
/// CEP:FAILURE: InvalidCost for fidelity_cost <= 0 or non-finite; NonFiniteInput for non-finite
///   delta_mean, delta_var, or value; NegativeVariance for delta_var < 0; NonFiniteInput if the
///   final score overflows (extreme-but-finite inputs are rejected, never saturated).
/// CEP:ASSUMES: delta_mean is accepted for signature stability and API symmetry with the
///   surrogate output; the normative proxy formula does not currently use it (documented
///   limitation, see CEP:WHY-PARTIAL).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: acquisition::tests::*.
pub fn acquire(
    delta_mean: f64,
    delta_var: f64,
    value: f64,
    fidelity_cost: f64,
) -> Result<f64, AutotunerError> {
    if !delta_mean.is_finite() || !delta_var.is_finite() || !value.is_finite() {
        return Err(AutotunerError::NonFiniteInput);
    }
    if !fidelity_cost.is_finite() || fidelity_cost <= 0.0 {
        return Err(AutotunerError::InvalidCost);
    }
    if delta_var < 0.0 {
        return Err(AutotunerError::NegativeVariance);
    }
    let bonus = 1.0 + CONFIDENCE_BONUS_SCALE * delta_var;
    let score = value * bonus / fidelity_cost;
    if !score.is_finite() {
        // Extreme-but-finite inputs may overflow the product; fail closed instead of
        // saturating to infinity.
        return Err(AutotunerError::NonFiniteInput);
    }
    Ok(score)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Zero, negative, and non-finite fidelity costs are rejected.
    /// CEP:WHY: Division-by-cost must never divide by zero.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_zero_cost() {
        assert_eq!(acquire(0.0, 0.0, 1.0, 0.0).unwrap_err(), AutotunerError::InvalidCost);
        assert_eq!(acquire(0.0, 0.0, 1.0, -1.0).unwrap_err(), AutotunerError::InvalidCost);
        assert_eq!(
            acquire(0.0, 0.0, 1.0, f64::INFINITY).unwrap_err(),
            AutotunerError::InvalidCost
        );
        assert!(acquire(0.0, 0.0, 1.0, 2.0).is_ok());
    }

    /// CEP:WHAT: NaN and infinite surrogate outputs are rejected.
    /// CEP:WHY: Non-finite inputs must never reach the score.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_nan_inputs() {
        assert_eq!(
            acquire(f64::NAN, 0.0, 1.0, 1.0).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
        assert_eq!(
            acquire(0.0, f64::INFINITY, 1.0, 1.0).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
        assert_eq!(
            acquire(0.0, 0.0, f64::NAN, 1.0).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
    }

    /// CEP:WHAT: A negative variance is rejected; a variance overflowing the score is rejected.
    /// CEP:WHY: Impossible statistics and overflow both fail closed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_negative_variance() {
        assert_eq!(acquire(0.0, -1e-9, 1.0, 1.0).unwrap_err(), AutotunerError::NegativeVariance);
        // value * (1 + 0.5*var) overflows for var near f64::MAX.
        assert_eq!(
            acquire(0.0, f64::MAX * 0.5, f64::MAX, 1e-300).unwrap_err(),
            AutotunerError::NonFiniteInput
        );
    }

    /// CEP:WHAT: The score is monotone in value and variance and anti-monotone in cost.
    /// CEP:WHY: Directional correctness of the explore/exploit/cost tradeoff.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn monotone_in_value_variance_cost() {
        let base = acquire(0.0, 0.1, 1.0, 1.0).unwrap();
        assert!(acquire(0.0, 0.1, 2.0, 1.0).unwrap() > base, "higher value ranks higher");
        assert!(acquire(0.0, 0.2, 1.0, 1.0).unwrap() > base, "higher variance ranks higher");
        assert!(acquire(0.0, 0.1, 1.0, 2.0).unwrap() < base, "higher cost ranks lower");
        // Exact formula check: value * (1 + 0.5*var) / cost.
        let exact = acquire(0.0, 0.4, 3.0, 2.0).unwrap();
        assert!((exact - 3.0 * 1.2 / 2.0).abs() < 1e-12);
    }

    /// CEP:WHAT: Typical surrogate outputs produce a finite score with no error.
    /// CEP:WHY: Happy-path contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn finite_for_typical_inputs() {
        for var in [0.0, 1e-6, 0.5, 10.0] {
            let s = acquire(0.25, var, 1.5, 16.0).unwrap();
            assert!(s.is_finite() && s > 0.0);
        }
        // Negative value yields a negative score (still finite and orderable).
        assert!(acquire(0.0, 0.0, -2.0, 1.0).unwrap() < 0.0);
    }
}
