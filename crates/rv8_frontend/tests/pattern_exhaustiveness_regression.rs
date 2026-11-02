// CEP:FILE: crates/rv8_frontend/tests/pattern_exhaustiveness_regression.rs
// CEP:WHAT: Twenty-five regression tests for the pattern exhaustiveness engine
//   (task 2-b1): exhaustiveness verdicts with exact missing constructors in constructor
//   order, per-arm usefulness including duplicate and subsumed arms, wildcard and
//   binding-only coverage, behavior at and beyond MAX_PATTERN_DEPTH including a
//   300_000-deep input that would stack-overflow a naive recursive implementation,
//   byte-identical diagnostics on repeated input, boolean/Option/integer decision-tree
//   specializations, and the closed multi-constructor space walked variant by variant.
// CEP:WHY: design.md section 7 and standard 5.7 require the decision tree, the usefulness
//   check, and the exhaustiveness check with missing constructors; every invariant this
//   module claims is pinned by one standing test, and the determinism and fail-closed
//   laws (38.10, 38.6, 38.12) are exercised against the public API surface.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: panics via assert/expect (test-only).
// CEP:ASSUMES: the pattern IR carries no or-patterns, tuples, structs, or user-defined
//   enums; Option is the only closed multi-constructor type, so the many-variant enum
//   scenario is pinned through a nested Option chain (regression_pat_25) instead of
//   inventing AST features that do not exist.
// CEP:COST: test-only.
// CEP:EVIDENCE: this module.
use rv8_frontend::patterns::matrix::Constructor;
use rv8_frontend::patterns::{
    check_exhaustiveness, check_match, check_usefulness, DecisionTree, IntKind, Pattern,
    PatternError, PatternType, MAX_MISSING_REPORTED, MAX_PATTERN_DEPTH,
};

/// CEP:WHAT: Named depth fixtures for the budget tests (Law 7: named constants).
/// CEP:WHY: Depth budgets and the naive-recursion-overflow depth appear as names, never
///   as inline magic numbers, in the tests below.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
const AT_BUDGET_LEVELS: u32 = MAX_PATTERN_DEPTH - 1;
const OVER_BUDGET_LEVELS: u32 = MAX_PATTERN_DEPTH;
const OVER_BUDGET_TYPE_LEVELS: u32 = MAX_PATTERN_DEPTH;
const NAIVE_OVERFLOW_LEVELS: u32 = 300_000;
const DETERMINISM_LEVELS: u32 = 8;
const MANY_VARIANT_LEVELS: u32 = 3;
const DUPLICATE_LITERAL: i128 = 4;

/// CEP:WHAT: Helper: wraps a pattern in `levels` Some constructors, iteratively.
/// CEP:WHY: Deep fixtures must be built without recursion, mirroring the engine's law.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only; one Box per level.
/// CEP:EVIDENCE: this module.
fn some_n(inner: Pattern, levels: u32) -> Pattern {
    let mut out = inner;
    for _ in 0..levels {
        out = Pattern::some(out);
    }
    out
}

/// CEP:WHAT: Helper: wraps a type in `levels` Option constructors, iteratively.
/// CEP:WHY: Deep scrutinee fixtures must be built without recursion.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only; one Box per level.
/// CEP:EVIDENCE: this module.
fn option_n(inner: PatternType, levels: u32) -> PatternType {
    let mut out = inner;
    for _ in 0..levels {
        out = PatternType::option_of(out);
    }
    out
}

/// CEP:WHAT: Regression 01_exhaustive_bool: a boolean match with both arms passes clean.
/// CEP:WHY: Base positive for the boolean specialization with zero diagnostics.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_01_exhaustive_bool_passes() {
    let report = check_match(
        &[Pattern::LitBool(false), Pattern::LitBool(true)],
        &PatternType::Bool,
    )
    .expect("valid");
    assert!(report.exhaustive);
    assert!(report.missing.is_empty());
    assert!(report.unreachable_arms.is_empty());
    assert!(!report.missing_truncated);
}

/// CEP:WHAT: Regression 02_exhaustive_option: None plus Some(_) covers Option exactly.
/// CEP:WHY: Base positive for the Option specialization in source order Some-first.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_02_exhaustive_option_passes() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I8));
    let report =
        check_match(&[Pattern::some(Pattern::Wildcard), Pattern::OptionNone], &ty)
            .expect("valid");
    assert!(report.exhaustive);
    assert!(report.missing.is_empty());
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: Regression 03_int_wildcard_last: listed literals plus a final wildcard are
///   exhaustive over an open integer column and every arm is useful.
/// CEP:WHY: Open-column coverage is decided by the wildcard-like row.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_03_exhaustive_int_wildcard_last() {
    let ty = PatternType::Int(IntKind::U8);
    let arms = [Pattern::lit_int(1), Pattern::lit_int(2), Pattern::Wildcard];
    let report = check_match(&arms, &ty).expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
    assert!(check_usefulness(&arms, &ty).expect("valid").is_empty());
}

/// CEP:WHAT: Regression 04_exhaustive_nested_option: None, Some(None), Some(Some(_))
///   covers Option<Option<Bool>> exactly.
/// CEP:WHY: Constructor decomposition must reach two Option levels.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_04_exhaustive_nested_option() {
    let ty = PatternType::option_of(PatternType::option_of(PatternType::Bool));
    let arms = [
        Pattern::OptionNone,
        Pattern::some(Pattern::OptionNone),
        Pattern::some(Pattern::some(Pattern::Wildcard)),
    ];
    let report = check_match(&arms, &ty).expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: Regression 05_missing_true: a false-only match reports exactly [true].
/// CEP:WHY: Missing-constructor precision for the boolean column.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_05_missing_true_exact() {
    let report = check_match(&[Pattern::LitBool(false)], &PatternType::Bool).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::LitBool(true)]);
    assert_eq!(report.missing[0].to_string(), "true");
}

/// CEP:WHAT: Regression 06_missing_none: a Some-only match reports exactly [None].
/// CEP:WHY: The task's ["None"] missing-constructor example, pinned structurally.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_06_missing_none_exact() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I8));
    let report =
        check_match(&[Pattern::some(Pattern::Wildcard)], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::OptionNone]);
    assert_eq!(report.missing[0].to_string(), "None");
}

/// CEP:WHAT: Regression 07_missing_some_wildcard: a None-only match reports exactly
///   [Some(_)].
/// CEP:WHY: The task's ["Some(_)"] missing-constructor example, pinned structurally.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_07_missing_some_wildcard_exact() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I8));
    let report = check_match(&[Pattern::OptionNone], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(report.missing, vec![Pattern::some(Pattern::Wildcard)]);
    assert_eq!(report.missing[0].to_string(), "Some(_)");
}

/// CEP:WHAT: Regression 08_missing_order: a Some(true)-only match over Option<Bool>
///   reports [None, Some(false)] in signature order (None before Some).
/// CEP:WHY: Determinism law: missing witnesses follow constructor declaration order,
///   never a hash-map order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_08_missing_constructor_order() {
    let ty = PatternType::option_of(PatternType::Bool);
    let report =
        check_match(&[Pattern::some(Pattern::LitBool(true))], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(
        report.missing,
        vec![Pattern::OptionNone, Pattern::some(Pattern::LitBool(false))]
    );
    assert!(!report.missing_truncated);
}

/// CEP:WHAT: Regression 09_missing_nested: Some(None) alone over Option<Option<i8>>
///   reports [None, Some(Some(_))].
/// CEP:WHY: The witness search descends past a covered constructor and reports the
///   minimal wildcard-headed witness for the uncovered subtree.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_09_missing_nested_two_level() {
    let ty = PatternType::option_of(PatternType::option_of(PatternType::Int(IntKind::I8)));
    let report = check_match(&[Pattern::some(Pattern::OptionNone)], &ty).expect("valid");
    assert!(!report.exhaustive);
    assert_eq!(
        report.missing,
        vec![Pattern::OptionNone, Pattern::some(Pattern::some(Pattern::Wildcard))]
    );
    assert_eq!(report.missing[1].to_string(), "Some(Some(_))");
}

/// CEP:WHAT: Regression 10_duplicate_arms: duplicated boolean, None, and literal arms
///   are each diagnosed unreachable at their source index.
/// CEP:WHY: The usefulness check must flag exact duplicates for every column shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_10_duplicate_arms_unreachable() {
    let duplicated = check_usefulness(
        &[Pattern::LitBool(false), Pattern::LitBool(false)],
        &PatternType::Bool,
    )
    .expect("valid");
    assert_eq!(duplicated, vec![1]);
    let none_dup = check_usefulness(
        &[Pattern::OptionNone, Pattern::OptionNone],
        &PatternType::option_of(PatternType::Bool),
    )
    .expect("valid");
    assert_eq!(none_dup, vec![1]);
    let int_dup = check_usefulness(
        &[Pattern::lit_int(DUPLICATE_LITERAL), Pattern::lit_int(DUPLICATE_LITERAL)],
        &PatternType::Int(IntKind::I8),
    )
    .expect("valid");
    assert_eq!(int_dup, vec![1]);
}

/// CEP:WHAT: Regression 11_wildcard_first: arms after a wildcard (boolean or integer)
///   are unreachable, at their exact source indices.
/// CEP:WHY: A leading wildcard-like row subsumes every later arm.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_11_wildcard_first_unreachable() {
    let bool_case = check_usefulness(
        &[Pattern::Wildcard, Pattern::LitBool(true)],
        &PatternType::Bool,
    )
    .expect("valid");
    assert_eq!(bool_case, vec![1]);
    let int_case = check_usefulness(
        &[Pattern::lit_int(1), Pattern::Wildcard, Pattern::lit_int(2)],
        &PatternType::Int(IntKind::I32),
    )
    .expect("valid");
    assert_eq!(int_case, vec![2]);
}

/// CEP:WHAT: Regression 12_wildcard_only: one wildcard is exhaustive over bool, a
///   nested Option, and an open integer type, with no unreachable arms, and
///   check_exhaustiveness independently reports an empty witness set.
/// CEP:WHY: Wildcards cover the entire type space by definition.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_12_wildcard_only_covers_all() {
    for ty in [
        PatternType::Bool,
        PatternType::option_of(PatternType::option_of(PatternType::Bool)),
        PatternType::Int(IntKind::I64),
    ] {
        let report = check_match(&[Pattern::Wildcard], &ty).expect("valid");
        assert!(report.exhaustive);
        assert!(report.unreachable_arms.is_empty());
        assert!(check_exhaustiveness(&[Pattern::Wildcard], &ty)
            .expect("valid")
            .is_empty());
    }
}

/// CEP:WHAT: Regression 13_binding_only_last_arm: a final binding arm covers the open
///   integer space and a lone binding arm covers the whole type.
/// CEP:WHY: Bindings are wildcard-like: they cover every value of their column.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_13_binding_only_last_arm() {
    let ty = PatternType::Int(IntKind::U8);
    let arms = [Pattern::lit_int(1), Pattern::lit_int(2), Pattern::binding(0)];
    let report = check_match(&arms, &ty).expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
    let lone = check_match(&[Pattern::binding(3)], &PatternType::Int(IntKind::I64))
        .expect("valid");
    assert!(lone.exhaustive);
}

/// CEP:WHAT: Regression 14_binding_inside_option: None plus Some(binding) is exhaustive
///   over Option<i32>.
/// CEP:WHY: A binding inside a constructor payload covers the payload column.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_14_binding_inside_option() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I32));
    let arms = [Pattern::OptionNone, Pattern::some(Pattern::binding(5))];
    let report = check_match(&arms, &ty).expect("valid");
    assert!(report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
}

/// CEP:WHAT: Regression 15_subsumed_some_arm: Some(_) before Some(1) makes the literal
///   arm unreachable; a repeated None after full Option coverage is unreachable too.
/// CEP:WHY: Subsumption, not only equality, must drive the usefulness verdict.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_15_subsumed_some_arm_unreachable() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I32));
    let subsumed = check_usefulness(
        &[Pattern::some(Pattern::Wildcard), Pattern::some(Pattern::lit_int(1))],
        &ty,
    )
    .expect("valid");
    assert_eq!(subsumed, vec![1]);
    let repeated = check_usefulness(
        &[Pattern::OptionNone, Pattern::some(Pattern::Wildcard), Pattern::OptionNone],
        &ty,
    )
    .expect("valid");
    assert_eq!(repeated, vec![2]);
}

/// CEP:WHAT: Regression 16_usefulness_negative: well-ordered arm lists produce no
///   unreachable diagnostics (no false positives across bool, Option, and int columns).
/// CEP:WHY: The usefulness check must not flag arms that values can reach.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_16_usefulness_negative_clean() {
    assert!(
        check_usefulness(&[Pattern::LitBool(false), Pattern::LitBool(true)], &PatternType::Bool)
            .expect("valid")
            .is_empty()
    );
    let ty = PatternType::option_of(PatternType::Bool);
    assert!(
        check_usefulness(&[Pattern::OptionNone, Pattern::some(Pattern::Wildcard)], &ty)
            .expect("valid")
            .is_empty()
    );
    let int_ty = PatternType::Int(IntKind::I16);
    assert!(
        check_usefulness(&[Pattern::lit_int(1), Pattern::binding(2)], &int_ty)
            .expect("valid")
            .is_empty()
    );
}

/// CEP:WHAT: Regression 17_bool_tree: the boolean decision tree switches on the
///   signature order (false then true) while each leaf names the first source arm.
/// CEP:WHY: Boolean specialization: case order comes from the type, overlap from source.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_17_bool_switch_specialization() {
    let report = check_match(
        &[Pattern::LitBool(true), Pattern::LitBool(false)],
        &PatternType::Bool,
    )
    .expect("valid");
    match &report.tree {
        DecisionTree::Switch { cases, fallback } => {
            assert_eq!(
                *cases,
                vec![
                    (Constructor::BoolFalse, DecisionTree::Leaf { arm: 1 }),
                    (Constructor::BoolTrue, DecisionTree::Leaf { arm: 0 }),
                ]
            );
            assert!(fallback.is_none());
        }
        other => panic!("expected Switch, got {other:?}"),
    }
}

/// CEP:WHAT: Regression 18_option_tree: the Option tree carries a None leaf plus a Some
///   subtree that itself switches over the payload type.
/// CEP:WHY: Option specialization: the tree decomposes constructors recursively.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_18_option_switch_specialization() {
    let ty = PatternType::option_of(PatternType::Bool);
    let report = check_match(&[Pattern::OptionNone, Pattern::some(Pattern::Wildcard)], &ty)
        .expect("valid");
    match &report.tree {
        DecisionTree::Switch { cases, fallback } => {
            assert_eq!(cases[0], (Constructor::OptionNone, DecisionTree::Leaf { arm: 0 }));
            assert_eq!(cases[1].0, Constructor::OptionSome);
            assert_eq!(
                cases[1].1,
                DecisionTree::Switch {
                    cases: vec![
                        (Constructor::BoolFalse, DecisionTree::Leaf { arm: 1 }),
                        (Constructor::BoolTrue, DecisionTree::Leaf { arm: 1 }),
                    ],
                    fallback: None,
                }
            );
            assert!(fallback.is_none());
        }
        other => panic!("expected Switch, got {other:?}"),
    }
}

/// CEP:WHAT: Regression 19_int_tree: the integer tree lists literal cases in ascending
///   value order with the wildcard arm as fallback leaf.
/// CEP:WHY: Open-column specialization: BTreeMap key order fixes case order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_19_int_tree_ascending_and_fallback() {
    let ty = PatternType::Int(IntKind::I32);
    let arms = [Pattern::lit_int(7), Pattern::lit_int(-2), Pattern::Wildcard];
    let report = check_match(&arms, &ty).expect("valid");
    match &report.tree {
        DecisionTree::Switch { cases, fallback } => {
            assert!(cases.len() == 2, "two literal cases");
            assert_eq!(cases[0].0, Constructor::IntLit(-2));
            assert_eq!(cases[0].1, DecisionTree::Leaf { arm: 1 });
            assert_eq!(cases[1].0, Constructor::IntLit(7));
            assert_eq!(cases[1].1, DecisionTree::Leaf { arm: 0 });
            assert_eq!(
                **fallback.as_ref().expect("fallback"),
                DecisionTree::Leaf { arm: 2 }
            );
        }
        other => panic!("expected Switch, got {other:?}"),
    }
}

/// CEP:WHAT: Regression 20_depth_at_budget: a 64-arm, 64-deep nested Option match is
///   accepted, its single gap is reported exactly as Some^63(false), and covering that
///   gap makes the match exhaustive.
/// CEP:WHY: The MAX_PATTERN_DEPTH boundary must analyze correctly, not just reject.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_20_nested_depth_at_max_accepted() {
    let ty = option_n(PatternType::Bool, AT_BUDGET_LEVELS);
    let mut arms: Vec<Pattern> = (0..AT_BUDGET_LEVELS)
        .map(|level| some_n(Pattern::OptionNone, level))
        .collect();
    arms.push(some_n(Pattern::LitBool(true), AT_BUDGET_LEVELS));
    assert!(arms.len() as u32 == MAX_PATTERN_DEPTH, "arms fill the budget");
    let report = check_match(&arms, &ty).expect("valid");
    assert!(!report.exhaustive);
    assert!(report.unreachable_arms.is_empty());
    assert!(!report.missing_truncated);
    assert_eq!(report.missing, vec![some_n(Pattern::LitBool(false), AT_BUDGET_LEVELS)]);
    arms.push(some_n(Pattern::LitBool(false), AT_BUDGET_LEVELS));
    let closed = check_match(&arms, &ty).expect("valid");
    assert!(closed.exhaustive);
    assert!(closed.missing.is_empty());
}

/// CEP:WHAT: Regression 21_depth_over_budget: a pattern one level past MAX_PATTERN_DEPTH
///   returns the structured PatternTooDeep error with exact found/max values.
/// CEP:WHY: Fail-closed law: over-deep input is a typed error, never a wrap or a panic.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_21_depth_over_budget_structured_error() {
    let ty = option_n(PatternType::Bool, AT_BUDGET_LEVELS);
    let deep = some_n(Pattern::LitBool(true), OVER_BUDGET_LEVELS);
    let err = check_match(&[deep], &ty).expect_err("over-budget pattern is rejected");
    assert!(
        matches!(
            &err,
            PatternError::PatternTooDeep { found, max: MAX_PATTERN_DEPTH }
                if *found == MAX_PATTERN_DEPTH + 1
        ),
        "structured error, got {err:?}"
    );
}

/// CEP:WHAT: Regression 22_type_depth_over_budget: a scrutinee type nested one level
///   past the budget is rejected even for wildcard arms.
/// CEP:WHY: Type depth bounds engine recursion the same way pattern depth does.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_22_type_depth_over_budget_structured_error() {
    let ty = option_n(PatternType::Bool, OVER_BUDGET_TYPE_LEVELS);
    let err = check_match(&[Pattern::Wildcard], &ty).expect_err("over-budget type rejected");
    assert!(matches!(
        err,
        PatternError::PatternTooDeep { max: MAX_PATTERN_DEPTH, .. }
    ));
}

/// CEP:WHAT: Regression 23_deep_input_no_stack_overflow: a 300_000-deep Some chain is
///   measured iteratively and rejected with a structured error; the chain is then
///   released through dispose_iterative without a recursive drop.
/// CEP:WHY: A naive recursive depth walk or derived drop would overflow the stack at
///   this depth; the engine and the documented release path must both stay flat.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_23_deep_input_rejected_without_stack_overflow() {
    let ty = PatternType::option_of(PatternType::Int(IntKind::I32));
    let mut deep = some_n(Pattern::Wildcard, NAIVE_OVERFLOW_LEVELS);
    let err = check_match(std::slice::from_ref(&deep), &ty)
        .expect_err("over-budget pattern is rejected");
    assert!(
        matches!(
            &err,
            PatternError::PatternTooDeep { found, max: MAX_PATTERN_DEPTH }
                if *found > MAX_PATTERN_DEPTH
        ),
        "structured error, got {err:?}"
    );
    deep.dispose_iterative();
    assert!(matches!(deep, Pattern::Wildcard));
    assert!(deep.depth_iterative() == 1, "chain fully released");
}

/// CEP:WHAT: Regression 24_determinism: two identical check_match calls return
///   byte-identical reports (including the truncated witness set and the decision
///   tree), and two identical invalid inputs return identical errors.
/// CEP:WHY: Determinism law 38.10: same input, same diagnostics, every run.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_24_diagnostics_deterministic_on_repeat() {
    let ty = option_n(PatternType::Bool, DETERMINISM_LEVELS);
    let arms = [some_n(Pattern::LitBool(true), DETERMINISM_LEVELS)];
    let first = check_match(&arms, &ty).expect("valid");
    let second = check_match(&arms, &ty).expect("valid");
    assert_eq!(first, second);
    assert!(first.missing_truncated, "witnesses exceed the report budget");
    assert!(first.missing.len() == MAX_MISSING_REPORTED, "truncated to budget");
    let err_a = check_match(&[Pattern::OptionNone], &PatternType::Bool).expect_err("invalid");
    let err_b = check_match(&[Pattern::OptionNone], &PatternType::Bool).expect_err("invalid");
    assert_eq!(err_a, err_b);
    assert_eq!(err_a.to_string(), err_b.to_string());
}

/// CEP:WHAT: Regression 25_many_variant_closed_type: the four shapes of
///   Option<Option<Option<i8>>> are reported missing one at a time, each as the minimal
///   wildcard-headed witness, until the fully armed match is exhaustive.
/// CEP:WHY: The closed multi-constructor space (the enum case of design.md section 7)
///   must decompose variant by variant; with no user-defined enums in the subset, the
///   nested Option chain is the honest stand-in for many variants.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_pat_25_many_variant_closed_type_decomposition() {
    let ty = option_n(PatternType::Int(IntKind::I8), MANY_VARIANT_LEVELS);
    let gap = |covered: &[Pattern]| {
        let report = check_match(covered, &ty).expect("valid");
        assert!(!report.exhaustive);
        report.missing
    };
    assert_eq!(gap(&[Pattern::OptionNone]), vec![Pattern::some(Pattern::Wildcard)]);
    assert_eq!(
        gap(&[Pattern::OptionNone, Pattern::some(Pattern::OptionNone)]),
        vec![Pattern::some(Pattern::some(Pattern::Wildcard))]
    );
    assert_eq!(
        gap(&[
            Pattern::OptionNone,
            Pattern::some(Pattern::OptionNone),
            Pattern::some(Pattern::some(Pattern::OptionNone)),
        ]),
        vec![Pattern::some(Pattern::some(Pattern::some(Pattern::Wildcard)))]
    );
    let closed = check_match(
        &[
            Pattern::OptionNone,
            Pattern::some(Pattern::OptionNone),
            Pattern::some(Pattern::some(Pattern::OptionNone)),
            Pattern::some(Pattern::some(Pattern::some(Pattern::Wildcard))),
        ],
        &ty,
    )
    .expect("valid");
    assert!(closed.exhaustive);
    assert!(closed.unreachable_arms.is_empty());
}
