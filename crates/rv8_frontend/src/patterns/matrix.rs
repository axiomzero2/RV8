// CEP:FILE: crates/rv8_frontend/src/patterns/matrix.rs
// CEP:WHAT: The pattern matrix of the usefulness engine: single-column rows tagged with arm
//   provenance, constructor specialization S(c, M), the default matrix D(M) for open integer
//   columns, and the deterministic constructor signature per scrutinee type.
// CEP:WHY: design.md section 7 specifies constructor splitting over the pattern matrix;
//   specialization is the primitive both the usefulness and witness algorithms share, and
//   arm tags keep every derived decision in source order.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; every operation is total and returns values, never panics.
// CEP:ASSUMES: single-column matrices (no tuple or struct patterns exist in the subset);
//   rows preserve arm order so all derived orders are deterministic.
// CEP:COST: specialize is O(rows); every engine step is linear in matrix size.
// CEP:EVIDENCE: unit tests in this module; integration tests pattern_exhaustiveness.rs.

use super::pat::{Pattern, PatternType};

/// CEP:WHAT: One matrix row: the pattern of the current column plus the arm index it came
///   from, in source order.
/// CEP:WHY: Decision-tree leaves and diagnostics must name the arm; the tag carries the
///   provenance through specializations.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: one pair per row.
/// CEP:EVIDENCE: unit test specialize_drops_conflicting_rows.
pub type Row = (Pattern, usize);

/// CEP:WHAT: One constructor of the pattern language: the two boolean values, the two
///   Option variants, or one integer literal value.
/// CEP:WHY: Specialization and decision-tree switches are keyed on constructors.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: integer columns are open, so their only constructors are exact literals.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests in this module.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Constructor {
    BoolFalse,
    BoolTrue,
    OptionNone,
    OptionSome,
    IntLit(i128),
}

impl Constructor {
    /// CEP:WHAT: Reports the arity of the constructor payload: none for leaves, one for the
    ///   Some payload.
    /// CEP:WHY: Specialization of a surviving row descends into exactly this many cells.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test specialize_drops_conflicting_rows.
    pub fn arity(self) -> usize {
        match self {
            Constructor::OptionSome => 1,
            _ => 0,
        }
    }

    /// CEP:WHAT: Reports whether this constructor can match a value of the scrutinee type.
    /// CEP:WHY: The tree builder enumerates constructors per type shape; mismatches are
    ///   rejected upstream by conformance checking.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test signature_is_deterministic_per_type.
    pub fn fits(self, ty: &PatternType) -> bool {
        match self {
            Constructor::BoolFalse | Constructor::BoolTrue => matches!(ty, PatternType::Bool),
            Constructor::OptionNone | Constructor::OptionSome => {
                matches!(ty, PatternType::Option(_))
            }
            Constructor::IntLit(_) => matches!(ty, PatternType::Int(_)),
        }
    }
}

/// CEP:WHAT: Wraps each arm pattern as one tagged matrix row, preserving arm order.
/// CEP:WHY: The engine's entry matrices are the arms of one match in source order.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the caller validated arm count and conformance.
/// CEP:COST: O(arms).
/// CEP:EVIDENCE: integration tests pattern_exhaustiveness.rs.
pub fn rows_of(arms: &[Pattern]) -> Vec<Row> {
    arms.iter().enumerate().map(|(index, pat)| (pat.clone(), index)).collect()
}

/// CEP:WHAT: Specializes the matrix by one constructor: rows headed by a matching pattern
///   survive (descending into the payload), wildcard-headed rows survive as wildcards,
///   conflicting rows drop.
/// CEP:WHY: This is Maranget's S(c, M), the core splitting step of usefulness and witness
///   computation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the constructor fits the column type (checked by the callers).
/// CEP:COST: O(rows).
/// CEP:EVIDENCE: unit test specialize_drops_conflicting_rows.
pub fn specialize(ctor: Constructor, rows: &[Row]) -> Vec<Row> {
    let mut out = Vec::new();
    for (pattern, arm) in rows {
        let payload: Option<Pattern> = match (ctor, pattern) {
            (Constructor::BoolFalse, Pattern::LitBool(false)) => Some(Pattern::Wildcard),
            (Constructor::BoolTrue, Pattern::LitBool(true)) => Some(Pattern::Wildcard),
            (Constructor::OptionNone, Pattern::OptionNone) => Some(Pattern::Wildcard),
            (Constructor::IntLit(value), Pattern::LitInt(head)) if value == *head => {
                Some(Pattern::Wildcard)
            }
            (Constructor::OptionSome, Pattern::OptionSome(inner)) => Some((**inner).clone()),
            (_, head) if head.is_wildcard_like() => Some(Pattern::Wildcard),
            _ => None,
        };
        if let Some(payload) = payload {
            out.push((payload, *arm));
        }
    }
    out
}

/// CEP:WHAT: Computes the default matrix D(M): the rows whose head cannot exclude any value
///   (wildcard-like heads only), keeping their tags.
/// CEP:WHY: For open integer columns, usefulness of a wildcard pattern reduces to the
///   default matrix because unlisted literal values always exist.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(rows).
/// CEP:EVIDENCE: unit test default_matrix_keeps_wildcards.
pub fn default_matrix(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .filter(|(pattern, _)| pattern.is_wildcard_like())
        .cloned()
        .collect()
}

/// CEP:WHAT: Reports whether any row is headed by a wildcard-like pattern.
/// CEP:WHY: Open integer columns are covered exactly when a wildcard-headed row exists.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(rows).
/// CEP:EVIDENCE: test patterns_integer_literal_match_needs_wildcard.
pub fn has_wildcard_head(rows: &[Row]) -> bool {
    rows.iter().any(|(pattern, _)| pattern.is_wildcard_like())
}

/// CEP:WHAT: Enumerates the constructor signature of a scrutinee type in declaration order:
///   false then true for booleans, None then Some for options, and no constructors for the
///   open integer types.
/// CEP:WHY: Determinism law: constructor order comes from the type, never from a hash map;
///   this order fixes the missing-witness order.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1) plus a small allocation.
/// CEP:EVIDENCE: unit test signature_is_deterministic_per_type.
pub fn signature(ty: &PatternType) -> Vec<Constructor> {
    match ty {
        PatternType::Bool => vec![Constructor::BoolFalse, Constructor::BoolTrue],
        PatternType::Option(_) => vec![Constructor::OptionNone, Constructor::OptionSome],
        PatternType::Int(_) => Vec::new(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Specialization keeps matching and wildcard rows with their tags, drops
    ///   conflicting rows.
    /// CEP:WHY: Correctness of S(c, M) underpins the whole engine.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn specialize_drops_conflicting_rows() {
        let rows: Vec<Row> = vec![
            (Pattern::LitBool(false), 0),
            (Pattern::Wildcard, 1),
            (Pattern::LitBool(true), 2),
        ];
        let kept = specialize(Constructor::BoolFalse, &rows);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].1, 0);
        assert_eq!(specialize(Constructor::BoolTrue, &rows).len(), 2);

        let some_rows: Vec<Row> = vec![
            (Pattern::some(Pattern::lit_int(3)), 0),
            (Pattern::OptionNone, 1),
        ];
        let inner = specialize(Constructor::OptionSome, &some_rows);
        assert_eq!(inner, vec![(Pattern::lit_int(3), 0)]);
        assert_eq!(specialize(Constructor::OptionNone, &some_rows).len(), 1);
    }

    /// CEP:WHAT: Default matrix keeps exactly the wildcard-like rows with tags.
    /// CEP:WHY: Open-column usefulness depends on this filter.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn default_matrix_keeps_wildcards() {
        let rows: Vec<Row> = vec![
            (Pattern::lit_int(1), 0),
            (Pattern::binding(9), 1),
            (Pattern::lit_int(2), 2),
        ];
        let default = default_matrix(&rows);
        assert_eq!(default, vec![(Pattern::binding(9), 1)]);
        assert!(has_wildcard_head(&rows));
        assert!(!has_wildcard_head(&[(Pattern::lit_int(5), 0)]));
    }

    /// CEP:WHAT: Signature order is fixed per type and independent of input order.
    /// CEP:WHY: Determinism law for diagnostics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn signature_is_deterministic_per_type() {
        assert_eq!(
            signature(&PatternType::Bool),
            vec![Constructor::BoolFalse, Constructor::BoolTrue]
        );
        assert_eq!(
            signature(&PatternType::option_of(PatternType::Bool)),
            vec![Constructor::OptionNone, Constructor::OptionSome]
        );
        assert!(signature(&PatternType::Int(super::super::pat::IntKind::U32)).is_empty());
        assert!(Constructor::BoolFalse.fits(&PatternType::Bool));
        assert!(!Constructor::OptionSome.fits(&PatternType::Bool));
        assert_eq!(Constructor::OptionSome.arity(), 1);
        assert_eq!(Constructor::BoolTrue.arity(), 0);
    }
}
