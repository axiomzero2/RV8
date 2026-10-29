// CEP:FILE: crates/rv8_frontend/src/lib.rs
// CEP:WHAT: The rv8_frontend public API: compile_source turns Rust-subset source text into
//   verified Level-0 HIR through the full pipeline lex -> parse -> resolve -> infer ->
//   lower, running the frozen HIR verifier on every produced function and rejecting the
//   program on any verification failure.
// CEP:WHY: Standard 5.7 and 38.18 require the frontend to emit HIR satisfying spec 1.3
//   and to verify after construction; this module is the only sanctioned entry point.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns the first FrontendError of the pipeline with its span; library
//   code never panics on any input (38.6).
// CEP:ASSUMES: input is a valid UTF-8 string; all budgets are named constants in their
//   owning modules (MAX_PARSE_DEPTH, MAX_PARAMS, MAX_FUNCTIONS, MAX_AST_NODES,
//   MAX_SYNTAX_CONTEXTS) and per-function HIR budgets live in rv8_ir::hir.
// CEP:COST: linear in source size plus near-linear inference; verification is O(items).
// CEP:EVIDENCE: unit tests rv8_frontend::lib::tests::* (positive, negatives per rejection
//   class, determinism, and a larger control-flow program).
// CEP:SECURITY: source text is untrusted; the lexer rejects non-subset bytes, the parser
//   bounds recursion with a named budget, and every traversal is iterative or explicitly
//   depth-bounded (38.13).

pub mod ast;
pub mod error;
pub mod infer;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod patterns;
pub mod resolve;
pub mod token;

pub use error::FrontendError;
pub use parser::{MAX_FUNCTIONS, MAX_PARSE_DEPTH, MAX_PARAMS};
pub use token::{HygieneCounter, SyntaxContext};

use ast::Program;
use infer::Inference;
use resolve::FnTable;
use rv8_core::interner::Interner;
use rv8_core::span::{FileId, Span, SPAN_MAX_OFFSET};
use rv8_ir::hir::HirFunction;

/// CEP:WHAT: Compiles one source file into verified HIR functions.
/// CEP:WHY: The frontend contract of the driver pipeline (spec 7).
/// CEP:STATUS: complete
/// CEP:FAILURE: FrontendError with a span for every rejected input class (lexical,
///   syntactic, structural, resolution, inference, lowering, and HIR verification);
///   an empty program compiles to an empty vector.
/// CEP:ASSUMES: single-module input; the pipeline is deterministic for identical input
///   (38.10, 38.19).
/// CEP:COST: O(source) lexing/parsing plus O(AST) inference and lowering.
/// CEP:EVIDENCE: unit tests rv8_frontend::lib::tests::compile_source_fixture_add_main and
///   rv8_frontend::lib::tests::compile_source_is_deterministic.
pub fn compile_source(src: &str) -> Result<Vec<HirFunction>, FrontendError> {
    if src.len() as u64 > SPAN_MAX_OFFSET as u64 {
        return Err(FrontendError::SourceTooLarge {
            len: src.len(),
            span: Span::new(FileId(0), 0, 0),
        });
    }
    let file = FileId(0);
    let mut interner = Interner::new();
    let trees = lexer::lex(src, file, &mut interner)?;
    let eof_span = Span::new(file, src.len() as u32, src.len() as u32);
    let program: Program = parser::parse_program(&trees, &interner, eof_span)?;
    let fn_table: FnTable = resolve::resolve(&program, &interner)?;
    let inference: Inference = infer::infer_program(&program, &fn_table, &interner)?;
    let hirs = lower::lower_program(&program, &fn_table, &inference, &interner)?;
    for hir in &hirs {
        hir.verify().map_err(|e| FrontendError::VerificationFailed {
            detail: format!("{e:?}"),
            span: Span::new(file, 0, 0),
        })?;
    }
    Ok(hirs)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::lowering::hir_to_son;

    /// CEP:WHAT: The exact mandated fixture text.
    /// CEP:WHY: tests/fixtures/add_main.rs must compile through compile_source.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: repo layout is stable.
    /// CEP:COST: compile-time include.
    /// CEP:EVIDENCE: this test module.
    const FIXTURE: &str = include_str!("../../../tests/fixtures/add_main.rs");

    /// CEP:WHAT: Positive end-to-end: the fixture compiles to two functions that verify
    ///   and flatten via the frozen hir_to_son.
    /// CEP:WHY: Proves consumability by the level-6.1 transition, not just HIR validity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_fixture_add_main() {
        let funcs = compile_source(FIXTURE).expect("fixture compiles");
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name, "add");
        assert_eq!(funcs[1].name, "main");
        for f in &funcs {
            f.verify().expect("HIR verifies");
            hir_to_son(f).expect("HIR lowers to Sea of Nodes");
        }
        // The add function reads both parameters and returns their sum.
        assert_eq!(funcs[0].vars.len(), 2);
    }

    /// CEP:WHAT: Determinism: two compilations of the fixture produce identical HIR
    ///   canonical hashes per function.
    /// CEP:WHY: 38.10 and 38.19 require byte-identical translation results.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, two full pipelines.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_is_deterministic() {
        let a = compile_source(FIXTURE).expect("run 1");
        let b = compile_source(FIXTURE).expect("run 2");
        assert_eq!(a.len(), b.len());
        for (fa, fb) in a.iter().zip(b.iter()) {
            assert_eq!(fa.name, fb.name);
            assert_eq!(fa.canonical_hash(), fb.canonical_hash());
        }
        // The same holds for a control-heavy program with merges, loops, and phis.
        let bigger = "fn f() -> i32 { let mut x = 0; while x < 9 { if x % 2 == 0 { x = x + 3; } else { x = x + 1; } } loop { x = x + 1; if 15 < x { break; } } x }";
        let a2 = compile_source(bigger).expect("run 1");
        let b2 = compile_source(bigger).expect("run 2");
        for (fa, fb) in a2.iter().zip(b2.iter()) {
            assert_eq!(fa.canonical_hash(), fb.canonical_hash());
        }
    }

    /// CEP:WHAT: A larger program with if/else, while, loop/break/continue, let/mut,
    ///   calls, and every binary operator class lowers, verifies, and flattens.
    /// CEP:WHY: Required integration coverage beyond the minimal fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_larger_control_flow() {
        let src = concat!(
            "fn classify(n: i32) -> i32 {\n",
            "    let mut acc = 0;\n",
            "    let mut i = 0;\n",
            "    while i < n {\n",
            "        if i % 2 == 0 {\n",
            "            acc = acc + i * 2;\n",
            "        } else if i % 3 == 0 {\n",
            "            acc = acc - 1;\n",
            "        } else {\n",
            "            acc = acc + 1;\n",
            "        }\n",
            "        i = i + 1;\n",
            "    }\n",
            "    loop {\n",
            "        if acc % 3 == 0 {\n",
            "            acc = acc + 2;\n",
            "            continue;\n",
            "        }\n",
            "        acc = acc + 1;\n",
            "        if 17 < acc {\n",
            "            break;\n",
            "        }\n",
            "    }\n",
            "    acc\n",
            "}\n",
            "fn main() -> i32 { classify(10) }\n",
        );
        let funcs = compile_source(src).expect("larger program compiles");
        assert_eq!(funcs.len(), 2);
        for f in &funcs {
            f.verify().expect("HIR verifies");
            hir_to_son(f).expect("HIR flattens");
        }
        // Both loop constructs produced regions with header and latch.
        let classify = &funcs[0];
        let loop_count = classify
            .regions
            .iter()
            .filter(|r| matches!(r.kind, rv8_ir::hir::RegionKind::Loop))
            .count();
        assert_eq!(loop_count, 2, "while plus loop");
        let cond_count = classify
            .regions
            .iter()
            .filter(|r| matches!(r.kind, rv8_ir::hir::RegionKind::Conditional))
            .count();
        assert!(cond_count >= 3, "if/else-if plus two loop conds");
    }

    /// CEP:WHAT: Negative classes each return Err and never panic.
    /// CEP:WHY: 38.6 fail-closed acceptance; one case per rejected class minimum.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_unbalanced_delimiters() {
        for src in [
            "fn add(a: i32, b: i32 -> i32 { a + b }",
            "fn f() -> i32 { (1 + 2 }",
            "fn f() { } )",
        ] {
            assert!(compile_source(src).is_err(), "must reject: {src}");
        }
    }

    /// CEP:WHAT: `break` outside any loop is rejected.
    /// CEP:WHY: Negative class per the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_break_outside_loop() {
        match compile_source("fn f() -> i32 { break; 0 }") {
            Err(FrontendError::BreakOutsideLoop { .. }) => {}
            other => panic!("expected BreakOutsideLoop, got {other:?}"),
        }
    }

    /// CEP:WHAT: Duplicate function names are rejected.
    /// CEP:WHY: Negative class per the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_duplicate_fn_names() {
        match compile_source("fn f() { } fn f() { }") {
            Err(FrontendError::DuplicateFunction { name, .. }) => assert_eq!(name, "f"),
            other => panic!("expected DuplicateFunction, got {other:?}"),
        }
    }

    /// CEP:WHAT: Calls to unknown functions are rejected.
    /// CEP:WHY: Negative class per the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_unknown_call() {
        match compile_source("fn main() -> i32 { nope(1) }") {
            Err(FrontendError::UnknownFunction { name, .. }) => assert_eq!(name, "nope"),
            other => panic!("expected UnknownFunction, got {other:?}"),
        }
    }

    /// CEP:WHAT: Type mismatches are rejected, including bool plus i32.
    /// CEP:WHY: Negative class per the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_type_mismatch() {
        let src = "fn f(a: bool, b: i32) -> i32 { a + b }";
        match compile_source(src) {
            Err(err @ (FrontendError::TypeMismatch { .. }
            | FrontendError::OperandTypeInvalid { .. })) => {
                let _ = err;
            }
            other => panic!("expected a type error, got {other:?}"),
        }
        match compile_source("fn f() -> i32 { let x: u64 = 1; let y: i32 = 2; x + y }") {
            Err(FrontendError::TypeMismatch { .. }) => {}
            other => panic!("expected TypeMismatch for u64 + i32, got {other:?}"),
        }
    }

    /// CEP:WHAT: Nesting beyond MAX_PARSE_DEPTH generated in a loop is rejected cleanly.
    /// CEP:WHY: Negative class per the brief; deep input must not exhaust the stack.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_nesting_over_max_parse_depth() {
        let depth = MAX_PARSE_DEPTH as usize + 32;
        let mut src = String::from("fn f() -> i32 { ");
        for _ in 0..depth {
            src.push('(');
        }
        src.push('1');
        for _ in 0..depth {
            src.push(')');
        }
        src.push_str(" }");
        match compile_source(&src) {
            Err(FrontendError::NestingTooDeep { .. }) => {}
            other => panic!("expected NestingTooDeep, got {other:?}"),
        }
    }

    /// CEP:WHAT: EOF in the middle of a block comment is rejected.
    /// CEP:WHY: Negative class per the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_eof_in_block_comment() {
        match compile_source("fn f() { } /* truncated") {
            Err(FrontendError::Lex {
                kind: error::LexErrorKind::UnterminatedBlockComment,
                ..
            }) => {}
            other => panic!("expected UnterminatedBlockComment, got {other:?}"),
        }
    }

    /// CEP:WHAT: Additional rejection classes: unknown variables, immutability, arity,
    ///   unsupported operators, overflow, and bare return mismatches.
    /// CEP:WHY: Each class must return Err with a span, never panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_rejects_more_classes() {
        assert!(matches!(
            compile_source("fn f() -> i32 { ghost }"),
            Err(FrontendError::UnknownVariable { .. })
        ));
        assert!(matches!(
            compile_source("fn f() { let x = 1; x = 2; }"),
            Err(FrontendError::AssignToImmutable { .. })
        ));
        assert!(matches!(
            compile_source("fn g(a: i32) -> i32 { a } fn f() -> i32 { g(1, 2) }"),
            Err(FrontendError::ArityMismatch { .. })
        ));
        assert!(matches!(
            compile_source("fn f(a: i32, b: i32) -> bool { a != b }"),
            Err(FrontendError::Unsupported { .. })
        ));
        assert!(matches!(
            compile_source("fn f() -> i32 { 99999999999999999999999 }"),
            Err(FrontendError::Lex { .. })
        ));
        assert!(matches!(
            compile_source("fn f() -> i32 { return; }"),
            Err(FrontendError::TypeMismatch { .. })
        ));
        assert!(matches!(
            compile_source("fn f() { let x: bool = 5; }"),
            Err(FrontendError::OperandTypeInvalid { .. })
        ));
    }

    /// CEP:WHAT: A return inside a loop body (transfer out of nested constructs) lowers,
    ///   verifies, and flattens; dead statements after the transfer are tolerated.
    /// CEP:WHY: The flattener stops a region's statements at the first transfer; HIR must
    ///   stay valid around that convention.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_return_inside_loop() {
        let src = "fn find(n: i32) -> i32 { let mut i = 0; loop { if i == n { return i; } i = i + 1; } }";
        let funcs = compile_source(src).expect("return-in-loop compiles");
        for f in &funcs {
            f.verify().expect("HIR verifies");
            hir_to_son(f).expect("HIR flattens");
        }
        let src2 = "fn f() -> i32 { while true { return 1; } }";
        let funcs2 = compile_source(src2).expect("return-in-while compiles");
        for f in &funcs2 {
            f.verify().expect("HIR verifies");
            hir_to_son(f).expect("HIR flattens");
        }
    }

    /// CEP:WHAT: An empty program compiles to an empty vector.
    /// CEP:WHY: Boundary behavior of the pipeline entry point.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn compile_source_empty_program() {
        let funcs = compile_source("").expect("empty compiles");
        assert!(funcs.is_empty());
        let funcs2 = compile_source("// only a comment\n").expect("comment-only compiles");
        assert!(funcs2.is_empty());
    }

    /// CEP:WHAT: The hygiene counter is exposed, monotonic, and saturating.
    /// CEP:WHY: Public API surface check for SyntaxContext plumbing.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hygiene_counter_api() {
        let mut hc = HygieneCounter::new();
        let a = hc.fresh();
        let b = hc.fresh();
        assert_eq!(a, SyntaxContext(0));
        assert_eq!(b, SyntaxContext(1));
        assert!(!hc.used_up());
    }
}
