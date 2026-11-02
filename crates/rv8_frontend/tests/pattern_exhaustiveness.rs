// CEP:FILE: crates/rv8_frontend/tests/pattern_exhaustiveness.rs
// CEP:WHAT: The integration batch for rv8_frontend::patterns: exhaustiveness verdicts with
//   exact missing constructors, per-arm usefulness (unreachable and subsumed arms),
//   wildcard and binding coverage, integer literal ranges, decision-tree shapes, named
//   budget rejections including a 200_000-deep pattern that proves the iterative depth
//   walk, and byte-identical diagnostics across repeated runs.
// CEP:WHY: Every claimed invariant of the pattern checker needs a standing test: the
//   determinism law (38.10), the fail-closed law (38.6), and the design.md section 7
//   contract (decision tree, usefulness, exhaustiveness) are pinned here.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (test-only).
// CEP:ASSUMES: none
// CEP:COST: test-only.
// CEP:EVIDENCE: this module.
use rv8_frontend::patterns::{
    check_match, DecisionTree, IntKind, Pattern, PatternError, PatternType,
};
use rv8_ir::son::IrType;

/// CEP:WHAT: Named fixtures for the integer decision-tree test (Law 7: named constants).
/// CEP:WHY: The expected case count and the pinned literal values appear as names, not as
///   inline magic numbers, in the case-order assertions below.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
const INT_TREE_CASES: usize = 2;
const INT_TREE_NEGATIVE: i128 = -3;
const INT_TREE_POSITIVE: i128 = 5;

/// CEP:WHAT: Helper: PatternType for one integer kind.
/// CEP:WHY: Shared fixture across the batch.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn int_ty(kind: IntKind) -> PatternType {
    PatternType::Int(kind)
}

/// CEP:WHAT: Exhaustive boolean match passes with no diagnostics.
/// CEP:WHY: Base positive for boolean specialization.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_exhaustive_bool_match_passes() {
    let report = check_match(
        &[Pattern::LitBool(false), Pattern::LitBool(true)],
        &PatternType::Bool,
    )
    .expect("exhaustive");
    assert!(report.exhaustive);
    assert!(report.missing.is_empty());
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: Exhaustive Option match (None plus Some wildcard) passes.
/// CEP:WHY: Base positive for Option specialization.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_exhaustive_option_match_passes() {
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let report = check_match(&[Pattern::some(Pattern::Wildcard), Pattern::OptionNone], &ty)
        .expect("exhaustive");
    assert!(report.exhaustive);
    assert!(report.missing.is_empty());
}

/// CEP:WHAT: A match with only false reports missing `true` exactly.
/// CEP:WHY: Missing-constructor precision for boolean columns.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_missing_true_reports_exact_constructor() {
    let report = check_match(&[Pattern::LitBool(false)], &PatternType::Bool).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::LitBool(true)]);
    assert_eq!(
        report.missing.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        vec!["true".to_string()]
    );
}

/// CEP:WHAT: A match with only Some(_) reports missing `None` exactly.
/// CEP:WHY: The task's exact missing-constructor example for Option.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_missing_none_reports_exact_constructor() {
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let report = check_match(&[Pattern::some(Pattern::Wildcard)], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::OptionNone]);
    assert_eq!(report.missing[0].to_string(), "None");
}

/// CEP:WHAT: A match with only None reports missing `Some(_)` exactly.
/// CEP:WHY: The task's exact missing-constructor example for Option.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_missing_some_wildcard_reports_exact_constructor() {
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let report = check_match(&[Pattern::OptionNone], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::some(Pattern::Wildcard)]);
    assert_eq!(report.missing[0].to_string(), "Some(_)");
}

/// CEP:WHAT: A match on Option<bool> with only Some(true) reports missing `None` then
///   `Some(false)` in constructor order.
/// CEP:WHY: Nested witness order is deterministic constructor order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_missing_nested_some_false_reports_exact_constructors() {
    let ty = PatternType::option_of(PatternType::Bool);
    let report = check_match(&[Pattern::some(Pattern::LitBool(true))], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(
        report.missing,
        vec![Pattern::OptionNone, Pattern::some(Pattern::LitBool(false))]
    );
    assert_eq!(
        report.missing.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        vec!["None".to_string(), "Some(false)".to_string()]
    );
}

/// CEP:WHAT: An arm after a boolean wildcard is unreachable.
/// CEP:WHY: Usefulness negative for boolean columns.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_unreachable_after_wildcard_bool() {
    let report = check_match(
        &[
            Pattern::LitBool(false),
            Pattern::LitBool(true),
            Pattern::Wildcard,
        ],
        &PatternType::Bool,
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert_eq!(report.unreachable_arms, vec![2]);
}

/// CEP:WHAT: A Some arm subsumed by an earlier Some wildcard arm is unreachable.
/// CEP:WHY: Usefulness negative with constructor specialization.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_unreachable_subsumed_some_arms() {
    let ty = PatternType::option_of(PatternType::Bool);
    let report = check_match(
        &[
            Pattern::some(Pattern::Wildcard),
            Pattern::some(Pattern::LitBool(true)),
        ],
        &ty,
    )
    .expect("valid");
    assert_eq!(report.unreachable_arms, vec![1]);
}

/// CEP:WHAT: A duplicate None arm after full Option coverage is unreachable.
/// CEP:WHY: Usefulness negative after exhaustive coverage.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_unreachable_after_full_option_coverage() {
    let ty = PatternType::option_of(int_ty(IntKind::U8));
    let report = check_match(
        &[
            Pattern::OptionNone,
            Pattern::some(Pattern::Wildcard),
            Pattern::OptionNone,
        ],
        &ty,
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert_eq!(report.unreachable_arms, vec![2]);
}

/// CEP:WHAT: A single wildcard arm is exhaustive and covers the whole type.
/// CEP:WHY: Wildcard-only coverage positive.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_wildcard_only_match_covers_all() {
    let report = check_match(&[Pattern::Wildcard], &PatternType::Bool).expect("valid");
    assert!(report.exhaustive);
    assert!(report.missing.is_empty());
    assert!(report.unreachable_arms.is_empty());
    let ty = PatternType::option_of(PatternType::Bool);
    let report = check_match(&[Pattern::Wildcard], &ty).expect("valid");
    assert!(report.exhaustive);
}

/// CEP:WHAT: Arms after a leading wildcard are unreachable but the match is exhaustive.
/// CEP:WHY: Wildcard-first usefulness negative.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_wildcard_first_marks_later_arms_unreachable() {
    let report = check_match(
        &[Pattern::Wildcard, Pattern::LitBool(true), Pattern::LitBool(false)],
        &PatternType::Bool,
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert_eq!(report.unreachable_arms, vec![1, 2]);
}

/// CEP:WHAT: An integer match without a wildcard reports the wildcard as missing.
/// CEP:WHY: Open integer columns are never exhausted by literals.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_integer_literal_match_needs_wildcard() {
    let report = check_match(
        &[Pattern::lit_int(0), Pattern::lit_int(1)],
        &int_ty(IntKind::U32),
    )
    .expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::Wildcard]);
    assert_eq!(report.missing[0].to_string(), "_");
}

/// CEP:WHAT: An integer match whose last arm is a wildcard is exhaustive.
/// CEP:WHY: Open-column coverage via defaulting.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_integer_wildcard_last_arm_exhaustive() {
    let report = check_match(
        &[Pattern::lit_int(0), Pattern::lit_int(1), Pattern::Wildcard],
        &int_ty(IntKind::U32),
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: A duplicate integer literal arm is unreachable.
/// CEP:WHY: Usefulness negative for literal specialization.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_integer_duplicate_literal_arm_unreachable() {
    let report = check_match(
        &[
            Pattern::lit_int(7),
            Pattern::Wildcard,
            Pattern::lit_int(7),
        ],
        &int_ty(IntKind::I64),
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert_eq!(report.unreachable_arms, vec![2]);
}

/// CEP:WHAT: Negative integer literals check against signed ranges and stay useful.
/// CEP:WHY: Signed literal handling end to end.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_integer_negative_literals_work() {
    let report = check_match(
        &[
            Pattern::lit_int(-1),
            Pattern::lit_int(0),
            Pattern::lit_int(1),
            Pattern::Wildcard,
        ],
        &int_ty(IntKind::I8),
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
    let report = check_match(&[Pattern::lit_int(-129)], &int_ty(IntKind::I8));
    assert_eq!(
        report,
        Err(PatternError::LiteralOutOfRange {
            value: -129,
            ty: IntKind::I8,
        })
    );
}

/// CEP:WHAT: A literal above the kind range is rejected with LiteralOutOfRange.
/// CEP:WHY: Fail-closed literal validation (no wrap).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_literal_out_of_range_rejects() {
    let report = check_match(&[Pattern::lit_int(256)], &int_ty(IntKind::U8));
    assert_eq!(
        report,
        Err(PatternError::LiteralOutOfRange {
            value: 256,
            ty: IntKind::U8,
        })
    );
    let report = check_match(&[Pattern::lit_int(-1)], &int_ty(IntKind::U128));
    assert_eq!(
        report,
        Err(PatternError::LiteralOutOfRange {
            value: -1,
            ty: IntKind::U128,
        })
    );
}

/// CEP:WHAT: An integer literal against a boolean scrutinee is a structured type mismatch.
/// CEP:WHY: Fail-closed conformance; ill-typed patterns are never analyzed.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_type_mismatch_litint_on_bool() {
    let report = check_match(&[Pattern::lit_int(1)], &PatternType::Bool);
    match report {
        Err(PatternError::TypeMismatch {
            expected,
            found,
            ..
        }) => {
            assert_eq!(expected, rv8_frontend::patterns::TypeShape::Bool);
            assert_eq!(found, rv8_frontend::patterns::TypeShape::Int);
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

/// CEP:WHAT: A None pattern against an integer scrutinee is a structured type mismatch.
/// CEP:WHY: Fail-closed conformance for constructor shapes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_type_mismatch_none_on_int() {
    let report = check_match(&[Pattern::OptionNone], &int_ty(IntKind::I32));
    match report {
        Err(PatternError::TypeMismatch { expected, found, .. }) => {
            assert_eq!(expected, rv8_frontend::patterns::TypeShape::Int);
            assert_eq!(found, rv8_frontend::patterns::TypeShape::Option);
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

/// CEP:WHAT: A binding arm subsumed by an earlier Some wildcard is unreachable, and a
///   binding inside Some covers the inner type like a wildcard.
/// CEP:WHY: Bindings are wildcard-like for coverage.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_binding_covers_like_wildcard() {
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let report = check_match(
        &[
            Pattern::some(Pattern::Wildcard),
            Pattern::some(Pattern::binding(1)),
        ],
        &ty,
    )
    .expect("valid");
    assert_eq!(report.unreachable_arms, vec![1]);
    let report = check_match(&[Pattern::some(Pattern::binding(1)), Pattern::OptionNone], &ty)
        .expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: A pattern at exactly MAX_PATTERN_DEPTH is accepted.
/// CEP:WHY: Upper boundary of the depth budget.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_depth_at_budget_accepts() {
    let max = rv8_frontend::patterns::MAX_PATTERN_DEPTH;
    let mut ty = PatternType::Bool;
    for _ in 0..(max - 1) {
        ty = PatternType::option_of(ty);
    }
    let mut pat = Pattern::Wildcard;
    for _ in 0..(max - 1) {
        pat = Pattern::some(pat);
    }
    let report = check_match(&[pat], &ty).expect("depth at budget accepted");
    assert!(!report.exhaustive);
    assert_eq!(report.missing[0], Pattern::OptionNone);
}

/// CEP:WHAT: A 200_000-deep pattern is rejected with PatternTooDeep and without a stack
///   overflow, proving the depth walk is iterative.
/// CEP:WHY: The fail-closed traversal law; a recursive measurement would overflow.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_deep_input_over_budget_rejects() {
    let mut pat = Pattern::Wildcard;
    for _ in 0..200_000 {
        pat = Pattern::some(pat);
    }
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let report = check_match(std::slice::from_ref(&pat), &ty);
    // The rejected input is released through the iterative disposal path; a recursive
    // depth walk (measurement or release) would overflow the stack at this depth.
    match report {
        Err(PatternError::PatternTooDeep { found, max }) => {
            assert_eq!(found, 200_001);
            assert_eq!(max, rv8_frontend::patterns::MAX_PATTERN_DEPTH);
        }
        other => panic!("expected PatternTooDeep, got {other:?}"),
    }
    pat.dispose_iterative();
    assert_eq!(pat, Pattern::Wildcard);
}

/// CEP:WHAT: A scrutinee type nested beyond MAX_PATTERN_DEPTH is rejected structurally.
/// CEP:WHY: Engine recursion is bounded by type depth as well as pattern depth.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_deep_type_over_budget_rejects() {
    let mut ty = PatternType::Bool;
    for _ in 0..100 {
        ty = PatternType::option_of(ty);
    }
    let report = check_match(&[Pattern::Wildcard], &ty);
    assert!(matches!(report, Err(PatternError::PatternTooDeep { .. })));
}

/// CEP:WHAT: More arms than MAX_ARMS is a structured rejection.
/// CEP:WHY: Named arm budget (38.12).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_too_many_arms_rejects() {
    let max = rv8_frontend::patterns::MAX_ARMS;
    let arms = vec![Pattern::Wildcard; max + 1];
    let report = check_match(&arms, &PatternType::Bool);
    match report {
        Err(PatternError::TooManyArms { found, max }) => {
            assert_eq!(found, rv8_frontend::patterns::MAX_ARMS + 1);
            assert_eq!(max, rv8_frontend::patterns::MAX_ARMS);
        }
        other => panic!("expected TooManyArms, got {other:?}"),
    }
}

/// CEP:WHAT: Two runs over the same input produce identical reports, diagnostic text, and
///   tree shapes.
/// CEP:WHY: Determinism law (38.10, 38.19).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_diagnostics_are_deterministic() {
    let ty = PatternType::option_of(int_ty(IntKind::I32));
    let arms = vec![
        Pattern::some(Pattern::lit_int(1)),
        Pattern::OptionNone,
        Pattern::some(Pattern::lit_int(2)),
    ];
    let first = check_match(&arms, &ty).expect("valid");
    let second = check_match(&arms, &ty).expect("valid");
    assert_eq!(first, second);
    let text_a: Vec<String> = first.missing.iter().map(|p| p.to_string()).collect();
    let text_b: Vec<String> = second.missing.iter().map(|p| p.to_string()).collect();
    assert_eq!(text_a, text_b);
    assert_eq!(first.missing, vec![Pattern::some(Pattern::Wildcard)]);
    assert_eq!(format!("{:?}", first.tree), format!("{:?}", second.tree));
}

/// CEP:WHAT: The boolean decision tree switches on both constructors to the earliest
///   matching arms.
/// CEP:WHY: design.md section 7 step 1; tree shape is the lowering contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_decision_tree_bool_switch() {
    let report = check_match(
        &[Pattern::LitBool(false), Pattern::LitBool(true), Pattern::Wildcard],
        &PatternType::Bool,
    )
    .expect("valid");
    match &report.tree {
        DecisionTree::Switch { cases, fallback } => {
            assert_eq!(cases.len(), 2);
            assert_eq!(cases[0], (rv8_frontend::patterns::matrix::Constructor::BoolFalse, DecisionTree::Leaf { arm: 0 }));
            assert_eq!(cases[1], (rv8_frontend::patterns::matrix::Constructor::BoolTrue, DecisionTree::Leaf { arm: 1 }));
            assert!(fallback.is_none());
        }
        other => panic!("expected Switch, got {other:?}"),
    }
}

/// CEP:WHAT: The integer decision tree lists literal cases in ascending value order with a
///   wildcard fallback to the wildcard arm.
/// CEP:WHY: Open-column tree shape and deterministic case order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_decision_tree_integer_fallback() {
    let report = check_match(
        &[
            Pattern::lit_int(INT_TREE_POSITIVE),
            Pattern::Wildcard,
            Pattern::lit_int(INT_TREE_NEGATIVE),
        ],
        &int_ty(IntKind::I32),
    )
    .expect("valid");
    match &report.tree {
        DecisionTree::Switch { cases, fallback } => {
            assert_eq!(cases.len(), INT_TREE_CASES);
            // The negative value matches the wildcard arm 1 before the literal arm 2; the
            // positive value matches the literal arm 0. Source order decides both.
            assert_eq!(
                cases[0],
                (
                    rv8_frontend::patterns::matrix::Constructor::IntLit(INT_TREE_NEGATIVE),
                    DecisionTree::Leaf { arm: 1 }
                )
            );
            assert_eq!(
                cases[1],
                (
                    rv8_frontend::patterns::matrix::Constructor::IntLit(INT_TREE_POSITIVE),
                    DecisionTree::Leaf { arm: 0 }
                )
            );
            assert_eq!(**fallback.as_ref().expect("fallback"), DecisionTree::Leaf { arm: 1 });
        }
        other => panic!("expected Switch, got {other:?}"),
    }
}

/// CEP:WHAT: An empty match reports the wildcard witness and is not exhaustive.
/// CEP:WHY: Base case of the witness search.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_empty_arms_report_wildcard_witness() {
    let report = check_match(&[], &PatternType::Bool).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::Wildcard]);
    let ty = PatternType::option_of(PatternType::Bool);
    let report = check_match(&[], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::Wildcard]);
}

/// CEP:WHAT: Witness enumeration truncates deterministically at MAX_MISSING_REPORTED and
///   records the truncation flag.
/// CEP:WHY: The named reporting budget (38.12).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_missing_witnesses_truncated_at_budget() {
    let mut ty = PatternType::Bool;
    for _ in 0..4 {
        ty = PatternType::option_of(ty);
    }
    let mut pat = Pattern::LitBool(true);
    for _ in 0..4 {
        pat = Pattern::some(pat);
    }
    let report = check_match(&[pat], &ty).expect("valid");
    assert!(!report.exhaustive);
    let limit = rv8_frontend::patterns::MAX_MISSING_REPORTED;
    assert_eq!(report.missing.len(), limit);
    assert!(report.missing_truncated);
    // The untruncated witness set is: None, Some(None), Some(Some(None)),
    // Some(Some(Some(None))), Some(Some(Some(Some(false)))); the report keeps the first
    // four in constructor order.
    assert_eq!(report.missing[0], Pattern::OptionNone);
    assert_eq!(report.missing[1], Pattern::some(Pattern::OptionNone));
    assert_eq!(
        report.missing[3],
        Pattern::some(Pattern::some(Pattern::some(Pattern::OptionNone)))
    );
}

/// CEP:WHAT: PatternType::from_ir maps the scalar widths and rejects non-pattern types.
/// CEP:WHY: The integration hook receives IrType scrutinees from inference.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_from_ir_maps_scalar_widths() {
    assert_eq!(PatternType::from_ir(IrType::Bool), Some(PatternType::Bool));
    assert_eq!(
        PatternType::from_ir(IrType::I64),
        Some(int_ty(IntKind::I64))
    );
    assert_eq!(
        PatternType::from_ir(IrType::U128),
        Some(int_ty(IntKind::U128))
    );
    assert_eq!(PatternType::from_ir(IrType::F32), None);
    assert_eq!(PatternType::from_ir(IrType::Ptr), None);
    assert_eq!(PatternType::from_ir(IrType::Unit), None);
}

/// CEP:WHAT: Display renders patterns and types in one canonical spelling.
/// CEP:WHY: Deterministic diagnostics need stable text forms.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn patterns_display_roundtrip() {
    assert_eq!(Pattern::some(Pattern::lit_int(-2)).to_string(), "Some(-2)");
    assert_eq!(Pattern::binding(3).to_string(), "x3");
    assert_eq!(Pattern::LitBool(true).to_string(), "true");
    assert_eq!(
        PatternType::option_of(int_ty(IntKind::I64)).to_string(),
        "Option<i64>"
    );
    assert_eq!(int_ty(IntKind::U8).to_string(), "u8");
}
