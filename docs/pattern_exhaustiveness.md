# Pattern Exhaustiveness

## Abstract

The pattern checker verifies match arms of the RV8 Rust subset before lowering. It takes a scrutinee `PatternType` and the arm patterns in source order, and produces a `PatternReport` holding the unreachable arm indices, the exhaustiveness verdict, the missing constructor witnesses, and the decision tree of constructors. It is a standalone module of `rv8_frontend` invoked through `patterns::check_match`.

## Formal Definitions

A pattern is `enum Pattern { Wildcard, Binding { name: SymbolId }, LitBool(bool), LitInt(i128), OptionNone, OptionSome(Box<Pattern>) }`. A scrutinee type is `enum PatternType { Bool, Int(IntKind), Option(Box<PatternType>) }` where `IntKind` covers the ten integer widths `I8..I128, U8..U128` of the IR scalar set. The constructor space of `Bool` is `{false, true}`; of `Option<T>` it is `{None, Some}`; integer types are open, their only constructors being exact literal values.

A pattern matrix `M` is a list of rows `(Pattern, usize)` where the second component is the arm index in source order. The specialization of `M` by constructor `c` is `S(c, M)`: a row headed by a pattern matching `c` survives with its payload exposed (a `Some(p)` head yields the row `p`), a wildcard-like head (`Wildcard` or `Binding`) survives as a wildcard payload, and any other row drops. The default matrix `D(M)` keeps exactly the wildcard-like rows.

A pattern `p` is useful against `M` when some scrutinee value matches `p` and no row of `M`. A match is exhaustive when the wildcard row is not useful against the full matrix. A witness is a pattern matching at least one value that no arm covers; witnesses are constructed with `Some` and constructor leaves only, never with bindings.

## Algorithm

1. Validation (fail-closed, before analysis): arm count at most `MAX_ARMS`; scrutinee type depth at most `MAX_PATTERN_DEPTH` measured iteratively; per-arm pattern depth at most `MAX_PATTERN_DEPTH` measured iteratively; literal range check against the column kind; pattern/scrutinee conformance check (depth-bounded recursion).
2. Usefulness per arm `i` (arm order): `useful(arm_i, M[0..i], ty)`. For a wildcard-like pattern on a `Bool` or `Option` column, the test splits over the constructor signature and succeeds when any constructor path succeeds; on an `Int` column it reduces to `M[0..i]` having no wildcard-like head. For a constructor-headed pattern, the test is `S(c, M[0..i])` having no surviving row (arity 0) or the payload being useful against the specialized rows (arity 1).
3. Witness enumeration: a fully uncovered column yields the wildcard witness; a `Bool` column yields each missing literal in value order; an `Option` column yields `None` when uncovered, then recursively the `Some`-branch witnesses wrapped in `Some`. Enumeration stops after `MAX_MISSING_REPORTED + 1` witnesses and reports truncation.
4. Decision tree: closed columns switch on their signature constructors in declaration order; integer columns switch on the distinct literals in ascending value order (`BTreeMap`) with a wildcard fallback naming the first wildcard-headed arm; leaves name the first surviving row's arm, so source order decides overlaps; values covered by no case fail closed to `Fail`.

Time: one matrix specialization `O(rows)` per step; the total search is bounded by `MAX_DECISION_STEPS` steps, with at most `MAX_ARMS` rows of depth at most `MAX_PATTERN_DEPTH`. Witness enumeration is output-bounded by `MAX_MISSING_REPORTED + 1`. Space: `O(rows * depth)` for specialized matrices plus the tree.

## Invariants

1. No engine recursion exceeds the validated `MAX_PATTERN_DEPTH` of patterns and scrutinee types; the depth measurement itself is iterative, so over-deep input produces `PatternError::PatternTooDeep` and never a stack overflow.
2. Every engine step charges `MAX_DECISION_STEPS`; exhaustion of the budget yields `PatternError::BudgetExceeded`, never unbounded search.
3. Diagnostics are ordered deterministically: unreachable arms in ascending arm order, witnesses in constructor declaration order depth-first (integer literals ascending), independent of any hash iteration.
4. An ill-typed pattern (wrong constructor or literal for the column, literal outside the kind range) is rejected with `PatternError::TypeMismatch` or `PatternError::LiteralOutOfRange` before analysis.
5. The report is a pure function of `(arms, scrutinee)`: two calls produce equal `PatternReport` values.
6. A decision tree leaf names an arm whose pattern matches every value reaching that leaf; a value covered by no case and no fallback reaches `Fail`.

## Integration Points

Depends on: `rv8_core::interner` (SymbolId for binding names), `rv8_ir::son::IrType` (scrutinee scalar types via `PatternType::from_ir`).

Depended on by: none yet. The resolver, inference, and lowering passes do not call this module; the AST has no match expressions, so there is no production caller.

Signatures:
- `pub fn check_match(arms: &[Pattern], scrutinee: &PatternType) -> Result<PatternReport, PatternError>`
- `pub fn check_usefulness(arms: &[Pattern], scrutinee: &PatternType) -> Result<Vec<usize>, PatternError>`
- `pub fn check_exhaustiveness(arms: &[Pattern], scrutinee: &PatternType) -> Result<Vec<Pattern>, PatternError>`
- `pub fn build_decision_tree(arms: &[Pattern], scrutinee: &PatternType) -> Result<DecisionTree, PatternError>`

The planned caller resolves each match expression, maps its scrutinee `IrType` with `PatternType::from_ir`, desugars each arm pattern into a `Pattern`, and reports `unreachable_arms` as warnings and `missing` as one exhaustiveness error per match.

## Known Limitations

The checker is not yet invoked by the resolver or any other pass; it is tested standalone. User-defined enums, tuple patterns, struct patterns, or-patterns, range patterns, and slice patterns are not representable in `Pattern`; only booleans, integer literals, wildcards, bindings, and `Option` constructors exist. Integer columns are treated as open: a match over a finite-width integer is exhaustive only through a wildcard-like arm, so a complete literal enumeration of a narrow type is reported as non-exhaustive (a conservative verdict that never reports a false unreachable). Derived `Drop` (and `PartialEq`) on `Pattern` recurses one frame per `Some` level; accepted patterns are bounded by `MAX_PATTERN_DEPTH`, and callers holding rejected over-budget input release it with `Pattern::dispose_iterative`. Diagnostics carry no source spans; the future integration hook attaches spans from the AST. A fully uncovered column reports the wildcard witness rather than splitting into per-constructor witnesses.
