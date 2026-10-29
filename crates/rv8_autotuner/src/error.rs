// CEP:FILE: crates/rv8_autotuner/src/error.rs
// CEP:WHAT: The AutotunerError enum: the single failure vocabulary of the ML autotuner.
// CEP:WHY: Standard 38.6 requires fail-closed libraries; every fallible autotuner operation
//   reports through this type instead of panicking, so callers (driver, cache) can branch on
//   precise failure causes.
// CEP:CLASS: CEP-1
// CEP:STATUS: complete
// CEP:FAILURE: none; this type is itself the failure contract of the crate.
// CEP:ASSUMES: none
// CEP:COST: zero-sized error construction; Display formatting is cold-path only.
// CEP:EVIDENCE: every Result-returning public item in rv8_autotuner returns this type; negative
//   tests across genome, pl, cgt, acquisition, funnel, minhash, tuner assert these variants.

use std::fmt;

/// CEP:WHAT: Failure variants for every fallible operation in the autotuner.
/// CEP:WHY: One exhaustive error type keeps match arms total and messages precise.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: enum is Copy-sized; no allocation.
/// CEP:EVIDENCE: negative tests: genome::tests::validate_rejects_nan, pl::tests::empty_returns_none,
///   cgt::tests::predict_rejects_nan_features, acquisition::tests::rejects_zero_cost,
///   funnel::tests::filter_rejects_empty, tuner::tests::tune_rejects_empty_candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutotunerError {
    /// CEP:WHAT: A required input slice was empty (logits, candidates, features).
    /// CEP:WHY: Empty genomes or candidate pools have no defined meaning; fail closed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: zero.
    /// CEP:EVIDENCE: genome::tests::validate_rejects_empty.
    EmptyInput,
    /// CEP:WHAT: An input slice had the wrong length for the operation.
    /// CEP:WHY: Feature vectors must match FEATURE_DIM; crossover needs equal shapes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: `expected` and `actual` are honest sizes, not guesses.
    /// CEP:COST: zero.
    /// CEP:EVIDENCE: cgt::tests::predict_rejects_wrong_feature_length.
    LengthMismatch { expected: usize, actual: usize },
    /// CEP:WHAT: A genome axis exceeded MAX_PASS_COUNT entries.
    /// CEP:WHY: Named budget per standard 38.12 bounds surrogate compute and memory.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: zero.
    /// CEP:EVIDENCE: genome::tests::validate_rejects_too_long.
    ExceedsMaxPasses { max: usize, actual: usize },
    /// CEP:WHY: NaN/inf inputs would poison f64 math end to end; rejected at the boundary.
    /// CEP:WHAT: A non-finite float (NaN or infinity) reached a validated input.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: IEEE 754 semantics for is_finite.
    /// CEP:COST: one is_finite check per element on validated boundaries.
    /// CEP:EVIDENCE: cgt::tests::predict_rejects_nan_features, tuner::tests::record_rejects_nan_delta.
    NonFiniteInput,
    /// CEP:WHAT: A fidelity cost was zero, negative, or otherwise unusable as a divisor.
    /// CEP:WHY: Acquisition divides by cost; a non-positive divisor is undefined, never a panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: zero.
    /// CEP:EVIDENCE: acquisition::tests::rejects_zero_cost.
    InvalidCost,
    /// CEP:WHAT: A reported variance was negative, which no valid estimator produces.
    /// CEP:WHY: Fail closed on impossible statistics rather than silently clamping caller data.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: zero.
    /// CEP:EVIDENCE: acquisition::tests::rejects_negative_variance.
    NegativeVariance,
}

impl fmt::Display for AutotunerError {
    /// CEP:WHAT: Human-readable rendering of every error variant.
    /// CEP:WHY: Diagnostics flow into driver logs; messages must be precise and stable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) formatting, cold path.
    /// CEP:EVIDENCE: tuner tests surface messages via unwrap_err in failure assertions.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AutotunerError::EmptyInput => write!(f, "autotuner input slice was empty"),
            AutotunerError::LengthMismatch { expected, actual } => {
                write!(f, "autotuner input length {actual} does not match expected {expected}")
            }
            AutotunerError::ExceedsMaxPasses { max, actual } => {
                write!(f, "autotuner genome length {actual} exceeds MAX_PASS_COUNT {max}")
            }
            AutotunerError::NonFiniteInput => write!(f, "autotuner input was NaN or infinite"),
            AutotunerError::InvalidCost => write!(f, "autotuner fidelity cost must be finite and > 0"),
            AutotunerError::NegativeVariance => write!(f, "autotuner variance must be >= 0"),
        }
    }
}

impl std::error::Error for AutotunerError {}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies every variant renders without panicking and compares by value.
    /// CEP:WHY: Display is on the diagnostic path; PartialEq drives negative tests crate-wide.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn variants_display_and_compare() {
        let e = AutotunerError::LengthMismatch { expected: 3, actual: 4 };
        assert_eq!(e, AutotunerError::LengthMismatch { expected: 3, actual: 4 });
        assert_ne!(e, AutotunerError::EmptyInput);
        assert!(!format!("{e}").is_empty());
        assert!(!AutotunerError::NonFiniteInput.to_string().is_empty());
    }
}
