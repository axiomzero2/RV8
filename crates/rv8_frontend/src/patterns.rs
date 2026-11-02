// CEP:FILE: crates/rv8_frontend/src/patterns.rs
// CEP:WHAT: Decision-tree exhaustiveness and usefulness checking for match arms: a pattern
//   matrix engine over the type space the frontend models (booleans, Option chains, and the
//   integer widths), reporting unreachable arms and the missing constructors of a
//   non-exhaustive match as structured, deterministically ordered values.
// CEP:WHY: Standard 5.7 and design.md section 7 require decision-tree construction, a
//   usefulness check per arm, and an exhaustiveness check over the whole match before
//   lowering; this module is that small theorem prover, standalone and fail-closed.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every malformed input returns a PatternError (depth over budget, arm count
//   over budget, literal out of range, pattern/scrutinee type mismatch, or decision-step
//   budget exceeded); library code never panics on any input.
// CEP:ASSUMES: the AST currently carries no match expressions or pattern nodes, so this
//   module defines the standalone pattern IR (Pattern, PatternType) that a future
//   match-desugaring pass will produce; user-defined enums do not exist in the subset and
//   Option<T> is the only closed multi-constructor type modeled; the checker is not yet
//   invoked by resolve/infer/lower; derived Drop on Pattern recurses one frame per Some
//   level, so callers holding over-budget rejected input release it via
//   Pattern::dispose_iterative (the checker's own traversals never recurse into it).
// CEP:COST: usefulness and witness search are bounded by MAX_DECISION_STEPS over matrices
//   of at most MAX_ARMS rows whose depth is at most MAX_PATTERN_DEPTH.
// CEP:EVIDENCE: integration tests crates/rv8_frontend/tests/pattern_exhaustiveness.rs.
// CEP:SECURITY: pattern input is untrusted; depth is measured iteratively before any
//   recursion, so nesting beyond MAX_PATTERN_DEPTH produces an error and never a stack
//   overflow.

pub mod diag;
pub mod engine;
pub mod matrix;
pub mod pat;

pub use diag::{PatternError, PatternReport, TypeShape};
pub use engine::DecisionTree;
pub use pat::{IntKind, Pattern, PatternType};

/// CEP:WHAT: Upper bound on the nesting depth of one accepted pattern.
/// CEP:WHY: Named budget (38.12); depth-bounds every recursion in the engine so deep
///   untrusted input is rejected instead of exhausting the stack.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: tests patterns_deep_input_over_budget_rejects and
///   patterns_depth_at_budget_accepts in pattern_exhaustiveness.rs.
pub const MAX_PATTERN_DEPTH: u32 = 64;

/// CEP:WHAT: Upper bound on the number of match arms accepted by one check.
/// CEP:WHY: Named budget (38.12); bounds matrix size and diagnostic work per match.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test patterns_too_many_arms_rejects in pattern_exhaustiveness.rs.
pub const MAX_ARMS: usize = 256;

/// CEP:WHAT: Upper bound on missing-pattern witnesses reported for one non-exhaustive match.
/// CEP:WHY: Named budget (38.12); witness enumeration over deep Option chains can produce
///   exponentially many witnesses, and diagnostics are truncated deterministically.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test patterns_missing_witnesses_truncated_at_budget in
///   pattern_exhaustiveness.rs.
pub const MAX_MISSING_REPORTED: usize = 4;

/// CEP:WHAT: Upper bound on engine work (matrix specializations) for one check call.
/// CEP:WHY: Named budget (38.12); the usefulness search is worst-case exponential in
///   pattern depth, so a step budget turns pathological inputs into a typed error.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::engine::tests::budget_exhausted.
pub const MAX_DECISION_STEPS: u64 = 1_048_576;

/// CEP:WHAT: Checks one match: per-arm usefulness, whole-match exhaustiveness, and decision
///   tree construction, in that order, over the arms in source order.
/// CEP:WHY: The design.md section 7 entry point; the resolver will call this once per match
///   expression after patterns exist in the AST.
/// CEP:STATUS: complete
/// CEP:FAILURE: PatternError for structural rejection (budgets, type mismatches, literal
///   ranges); a valid match always yields a PatternReport, possibly with diagnostics.
/// CEP:ASSUMES: arms are in source order and all have the scrutinee type.
/// CEP:COST: bounded by MAX_ARMS rows, MAX_PATTERN_DEPTH recursion, and MAX_DECISION_STEPS.
/// CEP:EVIDENCE: integration tests pattern_exhaustiveness.rs (determinism and per-class
///   diagnostics).
pub fn check_match(
    arms: &[Pattern],
    scrutinee: &PatternType,
) -> Result<PatternReport, PatternError> {
    let unreachable_arms = check_usefulness(arms, scrutinee)?;
    let mut engine = engine::Witness::new(MAX_MISSING_REPORTED);
    let missing = engine.witnesses(&matrix::rows_of(arms), scrutinee)?;
    let missing_truncated = engine.truncated();
    let tree = build_decision_tree(arms, scrutinee)?;
    Ok(PatternReport {
        exhaustive: missing.is_empty(),
        missing,
        missing_truncated,
        unreachable_arms,
        tree,
    })
}

/// CEP:WHAT: Usefulness check per arm: returns the indices of arms that no scrutinee value
///   can reach, in ascending arm order.
/// CEP:WHY: design.md section 7 step 2; unreachable arms are a warning class diagnostic.
/// CEP:STATUS: complete
/// CEP:FAILURE: PatternError for structural rejection; never panics.
/// CEP:ASSUMES: arms are in source order.
/// CEP:COST: one usefulness query per arm under the step budget.
/// CEP:EVIDENCE: tests patterns_unreachable_* in pattern_exhaustiveness.rs.
pub fn check_usefulness(
    arms: &[Pattern],
    scrutinee: &PatternType,
) -> Result<Vec<usize>, PatternError> {
    validate_arms(arms, scrutinee)?;
    let mut unreachable = Vec::new();
    for index in 0..arms.len() {
        let mut budget = engine::Budget::new(MAX_DECISION_STEPS);
        let useful = engine::is_useful(
            &arms[index],
            &matrix::rows_of(&arms[..index]),
            scrutinee,
            &mut budget,
        )?;
        if !useful {
            unreachable.push(index);
        }
    }
    Ok(unreachable)
}

/// CEP:WHAT: Exhaustiveness check: returns the missing-pattern witnesses for the whole
///   match, in constructor depth-first order, truncated at MAX_MISSING_REPORTED.
/// CEP:WHY: design.md section 7 step 3; the missing constructors are the error diagnostic.
/// CEP:STATUS: complete
/// CEP:FAILURE: PatternError for structural rejection; an empty witness list means the
///   match is exhaustive.
/// CEP:ASSUMES: arms are in source order.
/// CEP:COST: witness search under the step budget.
/// CEP:EVIDENCE: tests patterns_missing_* in pattern_exhaustiveness.rs.
pub fn check_exhaustiveness(
    arms: &[Pattern],
    scrutinee: &PatternType,
) -> Result<Vec<Pattern>, PatternError> {
    validate_arms(arms, scrutinee)?;
    let mut engine = engine::Witness::new(MAX_MISSING_REPORTED);
    engine.witnesses(&matrix::rows_of(arms), scrutinee)
}

/// CEP:WHAT: Constructs the decision tree of constructors for the match: switch nodes per
///   closed constructor with a wildcard fallback for open integer columns.
/// CEP:WHY: design.md section 7 step 1; the tree is the shape the lowering pass will walk.
/// CEP:STATUS: complete
/// CEP:FAILURE: PatternError for structural rejection; an uncovered value fails closed to
///   DecisionTree::Fail inside the tree rather than to a panic.
/// CEP:ASSUMES: arms are in source order and validated.
/// CEP:COST: one tree node per reachable constructor path under the step budget.
/// CEP:EVIDENCE: tests patterns_decision_tree_* in pattern_exhaustiveness.rs.
pub fn build_decision_tree(
    arms: &[Pattern],
    scrutinee: &PatternType,
) -> Result<DecisionTree, PatternError> {
    validate_arms(arms, scrutinee)?;
    let mut budget = engine::Budget::new(MAX_DECISION_STEPS);
    engine::build_tree(&matrix::rows_of(arms), scrutinee, &mut budget)
}

/// CEP:WHAT: Validates arm count, per-arm depth (iteratively), literal ranges, and
///   pattern/scrutinee conformance before any engine work.
/// CEP:WHY: Fail-closed law (38.6): structural rejection precedes analysis so the engine
///   never sees over-deep or ill-typed patterns.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns the first PatternError in arm order.
/// CEP:ASSUMES: none
/// CEP:COST: O(total pattern nodes), iterative depth walk plus depth-bounded conformance.
/// CEP:EVIDENCE: tests patterns_type_mismatch_* and patterns_deep_input_over_budget_rejects.
fn validate_arms(arms: &[Pattern], scrutinee: &PatternType) -> Result<(), PatternError> {
    if arms.len() > MAX_ARMS {
        return Err(PatternError::TooManyArms {
            found: arms.len(),
            max: MAX_ARMS,
        });
    }
    let type_depth = scrutinee.depth_iterative();
    if type_depth > MAX_PATTERN_DEPTH {
        return Err(PatternError::PatternTooDeep {
            found: type_depth,
            max: MAX_PATTERN_DEPTH,
        });
    }
    for arm in arms {
        let depth = arm.depth_iterative();
        if depth > MAX_PATTERN_DEPTH {
            return Err(PatternError::PatternTooDeep {
                found: depth,
                max: MAX_PATTERN_DEPTH,
            });
        }
        pat::check_conformance(arm, scrutinee)?;
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: The step budget is honored: an exhaustive search beyond it is a typed error.
    /// CEP:WHY: Fail-closed guarantee for pathological inputs (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn budget_exhausted() {
        // A shallow query fits a tiny budget; a witness search that recurses through a deep
        // Option chain runs out of steps and fails closed.
        let ty = PatternType::Int(IntKind::U64);
        let arms: Vec<Pattern> = (0..64).map(Pattern::lit_int).collect();
        let mut budget = engine::Budget::new(8);
        let first = matrix::rows_of(&arms[..1]);
        assert!(engine::is_useful(&arms[1], &first, &ty, &mut budget).is_ok());
        let mut deep_ty = PatternType::Bool;
        for _ in 0..40 {
            deep_ty = PatternType::option_of(deep_ty);
        }
        let mut deep_pat = Pattern::LitBool(true);
        for _ in 0..40 {
            deep_pat = Pattern::some(deep_pat);
        }
        let mut tiny = engine::Budget::new(8);
        assert!(matches!(
            engine::witnesses_all(&matrix::rows_of(&[deep_pat]), &deep_ty, &mut tiny),
            Err(PatternError::BudgetExceeded { .. })
        ));
    }
}
