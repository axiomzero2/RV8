// CEP:FILE: crates/rv8_frontend/src/patterns/diag.rs
// CEP:WHAT: The structured diagnostics of the pattern checker: PatternError for structural
//   rejection, TypeShape for coarse type identity in mismatch errors, and PatternReport as
//   the complete result of check_match (unreachable arms, missing witnesses, tree).
// CEP:WHY: Standard 38.6 and 38.15 require fail-closed structured errors and deterministic
//   diagnostics; the checker reports values, never panics and never free text.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; this module defines the failure contract.
// CEP:ASSUMES: spans are attached by the future integration hook, which knows the source
//   positions of the arms; this module stays span-free by design.
// CEP:COST: allocation only on cold diagnostic paths.
// CEP:EVIDENCE: integration tests pattern_exhaustiveness.rs.

use super::engine::DecisionTree;
use super::pat::Pattern;
use std::fmt;

/// CEP:WHAT: The coarse shape of a pattern or scrutinee type, carried by type-mismatch
///   errors instead of owned types.
/// CEP:WHY: Errors stay cheap and exactly matchable in tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: tests patterns_type_mismatch_litint_on_bool, patterns_type_mismatch_none_on_int.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TypeShape {
    Bool,
    Int,
    Option,
}

impl fmt::Display for TypeShape {
    /// CEP:WHAT: Renders the shape name.
    /// CEP:WHY: One canonical text form per diagnostic value.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: test patterns_display_roundtrip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            TypeShape::Bool => "bool",
            TypeShape::Int => "integer",
            TypeShape::Option => "Option",
        };
        write!(f, "{name}")
    }
}

/// CEP:WHAT: The single error type of the pattern checker: depth budget, arm budget,
///   literal range, pattern/scrutinee type mismatch, or decision-step budget.
/// CEP:WHY: Library validation paths return Result (38.6); every rejection class is a
///   structured variant, one negative test per class.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on the cold error path.
/// CEP:EVIDENCE: one rejection test per variant in pattern_exhaustiveness.rs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PatternError {
    PatternTooDeep { found: u32, max: u32 },
    TooManyArms { found: usize, max: usize },
    LiteralOutOfRange { value: i128, ty: super::pat::IntKind },
    TypeMismatch { context: &'static str, expected: TypeShape, found: TypeShape },
    BudgetExceeded { max: u64 },
}

impl fmt::Display for PatternError {
    /// CEP:WHAT: Renders the error in one stable, human-readable line.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: test patterns_diagnostics_are_deterministic.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatternError::PatternTooDeep { found, max } => {
                write!(f, "pattern depth {found} exceeds the budget of {max}")
            }
            PatternError::TooManyArms { found, max } => {
                write!(f, "{found} match arms exceed the budget of {max}")
            }
            PatternError::LiteralOutOfRange { value, ty } => {
                write!(f, "literal {value} does not fit in {ty}")
            }
            PatternError::TypeMismatch { context, expected, found } => {
                write!(
                    f,
                    "type mismatch in {context}: expected {expected}, found {found}"
                )
            }
            PatternError::BudgetExceeded { max } => {
                write!(f, "pattern analysis exceeded its step budget of {max}")
            }
        }
    }
}

impl std::error::Error for PatternError {}

/// CEP:WHAT: The complete result of check_match: unreachable arm indices in ascending order,
///   the exhaustiveness verdict, missing witnesses in constructor order, the truncation flag,
///   and the built decision tree.
/// CEP:WHY: One value carries every diagnostic so the future resolver hook reports them in
///   one deterministic pass.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: arm indices refer to the caller's arm slice in source order.
/// CEP:COST: owned vectors; sized by diagnostics and tree nodes.
/// CEP:EVIDENCE: test patterns_diagnostics_are_deterministic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PatternReport {
    pub unreachable_arms: Vec<usize>,
    pub exhaustive: bool,
    pub missing: Vec<Pattern>,
    pub missing_truncated: bool,
    pub tree: DecisionTree,
}
