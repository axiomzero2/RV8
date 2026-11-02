// CEP:FILE: crates/rv8_frontend/src/patterns/engine.rs
// CEP:WHAT: The usefulness and witness engine: a step-budgeted usefulness test over the
//   pattern matrix (Maranget-style specialization with boolean/Option shortcuts and
//   open-integer defaulting), witness enumeration for missing constructors, and decision
//   tree construction with deterministic case order.
// CEP:WHY: design.md section 7 requires the decision tree, the per-arm usefulness check,
//   and the exhaustiveness witnesses; the engine recursion is bounded by MAX_PATTERN_DEPTH
//   (validated upstream on both patterns and scrutinee types) and by MAX_DECISION_STEPS.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every entry returns Result; BudgetExceeded reports pathological search size
//   instead of running unbounded; never panics.
// CEP:ASSUMES: callers validated arm count, arm depth, scrutinee type depth, literal
//   ranges, and pattern/scrutinee conformance (patterns::validate_arms).
// CEP:COST: each step is linear in matrix rows; total work bounded by MAX_DECISION_STEPS.
// CEP:EVIDENCE: unit tests in this module; integration tests pattern_exhaustiveness.rs.

use super::diag::PatternError;
use super::matrix::{self, Constructor, Row};
use super::pat::{Pattern, PatternType};
use super::MAX_DECISION_STEPS;
use std::collections::BTreeMap;

/// CEP:WHAT: A deterministic step budget: every engine recursion charges one step and the
///   search aborts with a typed error when the budget is spent.
/// CEP:WHY: Named budget (38.12) for a worst-case exponential search space.
/// CEP:STATUS: complete
/// CEP:FAILURE: charge returns BudgetExceeded once the limit is reached.
/// CEP:ASSUMES: none
/// CEP:COST: O(1) per charge.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::tests::budget_exhausted.
pub struct Budget {
    limit: u64,
    remaining: u64,
}

impl Budget {
    /// CEP:WHAT: Creates a budget with the given limit.
    /// CEP:WHY: Callers pass the named constant MAX_DECISION_STEPS.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::patterns::tests::budget_exhausted.
    pub fn new(limit: u64) -> Budget {
        Budget {
            limit,
            remaining: limit,
        }
    }

    /// CEP:WHAT: Charges one engine step.
    /// CEP:WHY: Turns pathological searches into a deterministic error.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded when the budget is spent.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::patterns::tests::budget_exhausted.
    fn charge(&mut self) -> Result<(), PatternError> {
        if self.remaining == 0 {
            return Err(PatternError::BudgetExceeded { max: self.limit });
        }
        self.remaining -= 1;
        Ok(())
    }
}

/// CEP:WHAT: The decision tree of constructors for one match: switch nodes keyed by
///   constructor (cases in signature order, integer literals in ascending value order), an
///   optional wildcard fallback for open integer columns, leaves naming the winning arm,
///   and Fail for values no row covers.
/// CEP:WHY: design.md section 7 step 1; the lowering pass will walk this tree.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: case evaluation order is cases then fallback; a missing case with no
///   fallback is Fail (fail-closed inside the tree).
/// CEP:COST: one node per reachable constructor path.
/// CEP:EVIDENCE: tests patterns_decision_tree_bool_switch and
///   patterns_decision_tree_integer_fallback.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DecisionTree {
    Fail,
    Leaf { arm: usize },
    Switch {
        cases: Vec<(Constructor, DecisionTree)>,
        fallback: Option<Box<DecisionTree>>,
    },
}

/// CEP:WHAT: Usefulness test: reports whether any scrutinee value matches `pat` and no row
///   of the matrix. Boolean and Option columns split by constructor; integer columns are
///   open, so a wildcard pattern is useful exactly when no wildcard-headed row exists.
/// CEP:WHY: design.md section 7 step 2, one query per arm.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded; never panics. Recursion depth is bounded by the validated
///   MAX_PATTERN_DEPTH of patterns and scrutinee types.
/// CEP:ASSUMES: `pat` conforms to `ty` and every row head conforms to `ty`.
/// CEP:COST: one step per recursion; each step is O(rows).
/// CEP:EVIDENCE: integration tests patterns_usefulness_* and patterns_unreachable_*.
pub fn is_useful(
    pat: &Pattern,
    rows: &[Row],
    ty: &PatternType,
    budget: &mut Budget,
) -> Result<bool, PatternError> {
    budget.charge()?;
    match pat {
        Pattern::Wildcard | Pattern::Binding { .. } => match ty {
            PatternType::Int(_) => Ok(!matrix::has_wildcard_head(rows)),
            PatternType::Bool => {
                if is_useful(&Pattern::LitBool(false), rows, ty, budget)? {
                    return Ok(true);
                }
                is_useful(&Pattern::LitBool(true), rows, ty, budget)
            }
            PatternType::Option(_) => {
                if is_useful(&Pattern::OptionNone, rows, ty, budget)? {
                    return Ok(true);
                }
                is_useful(&Pattern::some(Pattern::Wildcard), rows, ty, budget)
            }
        },
        Pattern::LitBool(value) => {
            let ctor = match value {
                false => Constructor::BoolFalse,
                true => Constructor::BoolTrue,
            };
            Ok(matrix::specialize(ctor, rows).is_empty())
        }
        Pattern::LitInt(value) => Ok(matrix::specialize(Constructor::IntLit(*value), rows).is_empty()),
        Pattern::OptionNone => Ok(matrix::specialize(Constructor::OptionNone, rows).is_empty()),
        Pattern::OptionSome(inner) => {
            let kept = matrix::specialize(Constructor::OptionSome, rows);
            match ty {
                PatternType::Option(item) => is_useful(inner, &kept, item, budget),
                _ => Ok(kept.is_empty()),
            }
        }
    }
}

/// CEP:WHAT: Witness enumerator for missing constructors: collects witnesses in constructor
///   depth-first order up to limit plus one (to detect truncation) and reports whether any
///   witness beyond the limit exists.
/// CEP:WHY: design.md section 7 step 3; missing patterns are the exhaustiveness diagnostic.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded; never panics.
/// CEP:ASSUMES: rows conform to the scrutinee type.
/// CEP:COST: bounded by MAX_DECISION_STEPS and the limit plus one witnesses.
/// CEP:EVIDENCE: tests patterns_missing_* in pattern_exhaustiveness.rs.
pub struct Witness {
    limit: usize,
    over: bool,
}

impl Witness {
    /// CEP:WHAT: Creates an enumerator with the named reporting limit.
    /// CEP:WHY: The limit is MAX_MISSING_REPORTED from the module root.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_missing_witnesses_truncated_at_budget.
    pub fn new(limit: usize) -> Witness {
        Witness { limit, over: false }
    }

    /// CEP:WHAT: Reports whether more witnesses than the limit exist.
    /// CEP:WHY: The truncation flag keeps diagnostics honest about incompleteness.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_missing_witnesses_truncated_at_budget.
    pub fn truncated(&self) -> bool {
        self.over
    }

    /// CEP:WHAT: Enumerates the missing witnesses for one matrix over one type.
    /// CEP:WHY: The exhaustiveness query of check_match and check_exhaustiveness.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded.
    /// CEP:ASSUMES: none
    /// CEP:COST: bounded by the step budget.
    /// CEP:EVIDENCE: tests patterns_missing_*.
    pub fn witnesses(
        &mut self,
        rows: &[Row],
        ty: &PatternType,
    ) -> Result<Vec<Pattern>, PatternError> {
        let mut budget = Budget::new(MAX_DECISION_STEPS);
        let mut out = Vec::new();
        self.collect(rows, ty, &mut out, &mut budget)?;
        if out.len() > self.limit {
            out.truncate(self.limit);
        }
        Ok(out)
    }

    /// CEP:WHAT: Recursive witness search: a fully uncovered column reports the wildcard
    ///   witness; closed columns split by constructor in signature order; open integer
    ///   columns report the wildcard witness unless a wildcard-headed row covers them.
    /// CEP:WHY: The witness order is the deterministic constructor order of the type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded.
    /// CEP:ASSUMES: recursion depth bounded by the validated type depth.
    /// CEP:COST: one step per visit.
    /// CEP:EVIDENCE: tests patterns_missing_none, patterns_missing_some_wildcard,
    ///   patterns_missing_nested_some_false.
    fn collect(
        &mut self,
        rows: &[Row],
        ty: &PatternType,
        out: &mut Vec<Pattern>,
        budget: &mut Budget,
    ) -> Result<(), PatternError> {
        budget.charge()?;
        if out.len() > self.limit {
            return Ok(());
        }
        if rows.is_empty() {
            self.push(out, Pattern::Wildcard);
            return Ok(());
        }
        match ty {
            PatternType::Int(_) => {
                if !matrix::has_wildcard_head(rows) {
                    self.push(out, Pattern::Wildcard);
                }
            }
            PatternType::Bool => {
                if matrix::specialize(Constructor::BoolFalse, rows).is_empty() {
                    self.push(out, Pattern::LitBool(false));
                }
                if matrix::specialize(Constructor::BoolTrue, rows).is_empty() {
                    self.push(out, Pattern::LitBool(true));
                }
            }
            PatternType::Option(inner) => {
                if matrix::specialize(Constructor::OptionNone, rows).is_empty() {
                    self.push(out, Pattern::OptionNone);
                }
                let kept = matrix::specialize(Constructor::OptionSome, rows);
                let mut child = Vec::new();
                self.collect(&kept, inner, &mut child, budget)?;
                for witness in child {
                    self.push(out, Pattern::some(witness));
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Pushes one witness, recording truncation when the limit is reached.
    /// CEP:WHY: The enumerator must detect witnesses beyond the limit without enumerating
    ///   unboundedly many.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_missing_witnesses_truncated_at_budget.
    fn push(&mut self, out: &mut Vec<Pattern>, witness: Pattern) {
        if out.len() == self.limit {
            self.over = true;
        }
        if out.len() > self.limit {
            return;
        }
        out.push(witness);
    }
}

/// CEP:WHAT: Unbounded witness enumeration for one matrix (used by tests and by callers
///   that want the complete witness set).
/// CEP:WHY: Separates the full enumeration from the reporting-limited diagnostic path.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded; witness count can grow exponentially with type depth, so
///   the step budget still bounds the work.
/// CEP:ASSUMES: none
/// CEP:COST: bounded by MAX_DECISION_STEPS.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::tests::budget_exhausted.
pub fn witnesses_all(
    rows: &[Row],
    ty: &PatternType,
    budget: &mut Budget,
) -> Result<Vec<Pattern>, PatternError> {
    let mut enumerator = Witness {
        limit: usize::MAX,
        over: false,
    };
    let mut out = Vec::new();
    enumerator.collect(rows, ty, &mut out, budget)?;
    Ok(out)
}

/// CEP:WHAT: Builds the decision tree for one matrix over one type: closed columns switch
///   on their signature constructors in order, integer columns switch on the distinct
///   literals in ascending value order with a wildcard fallback, and leaves name the first
///   matching row's arm so source order decides overlaps.
/// CEP:WHY: design.md section 7 step 1.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded; Fail nodes mark values no row covers.
/// CEP:ASSUMES: rows conform to the type; recursion depth bounded by the validated type
///   depth.
/// CEP:COST: one node per reachable constructor path; literal dedup is O(rows log rows).
/// CEP:EVIDENCE: tests patterns_decision_tree_bool_switch and
///   patterns_decision_tree_integer_fallback.
pub fn build_tree(
    rows: &[Row],
    ty: &PatternType,
    budget: &mut Budget,
) -> Result<DecisionTree, PatternError> {
    budget.charge()?;
    if rows.is_empty() {
        return Ok(DecisionTree::Fail);
    }
    match ty {
        PatternType::Int(_) => {
            let mut literals: BTreeMap<i128, ()> = BTreeMap::new();
            for (pattern, _) in rows {
                if let Pattern::LitInt(value) = pattern {
                    literals.insert(*value, ());
                }
            }
            let mut cases = Vec::new();
            for value in literals.keys() {
                let survivors = matrix::specialize(Constructor::IntLit(*value), rows);
                if let Some((_, arm)) = survivors.first() {
                    cases.push((Constructor::IntLit(*value), DecisionTree::Leaf { arm: *arm }));
                }
            }
            let fallback = matrix::default_matrix(rows);
            if cases.is_empty() {
                return match fallback.first() {
                    Some((_, arm)) => Ok(DecisionTree::Leaf { arm: *arm }),
                    None => Ok(DecisionTree::Fail),
                };
            }
            let fallback = fallback
                .first()
                .map(|(_, arm)| Box::new(DecisionTree::Leaf { arm: *arm }));
            Ok(DecisionTree::Switch { cases, fallback })
        }
        PatternType::Bool => {
            let mut cases = Vec::new();
            for ctor in [Constructor::BoolFalse, Constructor::BoolTrue] {
                let survivors = matrix::specialize(ctor, rows);
                if let Some((_, arm)) = survivors.first() {
                    cases.push((ctor, DecisionTree::Leaf { arm: *arm }));
                }
            }
            Ok(DecisionTree::Switch {
                cases,
                fallback: None,
            })
        }
        PatternType::Option(inner) => {
            let mut cases = Vec::new();
            for ctor in [Constructor::OptionNone, Constructor::OptionSome] {
                let survivors = matrix::specialize(ctor, rows);
                if survivors.is_empty() {
                    continue;
                }
                let subtree = match ctor {
                    Constructor::OptionNone => DecisionTree::Leaf { arm: survivors[0].1 },
                    _ => build_tree(&survivors, inner, budget)?,
                };
                cases.push((ctor, subtree));
            }
            Ok(DecisionTree::Switch {
                cases,
                fallback: None,
            })
        }
    }
}
