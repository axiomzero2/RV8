// CEP:FILE: crates/rv8_frontend/src/resolve.rs
// CEP:WHAT: The name-resolution pre-pass: builds the single-module function table in
//   declaration order, rejects duplicate function names, and validates that every call
//   target names a declared function with a matching arity.
// CEP:WHY: Call targets in the frozen HIR are plain function indices; resolution must
//   happen before inference so signatures can anchor argument types (standard 5.7).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns FrontendError (DuplicateFunction, UnknownFunction, ArityMismatch)
//   with spans; never panics on any Program.
// CEP:ASSUMES: one module; names are interned SymbolIds of this session.
// CEP:COST: O(F + total expressions); table lookup is a linear scan over declared names.
// CEP:EVIDENCE: unit tests rv8_frontend::resolve::tests::*.

use crate::ast::{self, Expr, ExprKind, Program, Stmt};
use crate::error::FrontendError;
use rv8_core::interner::{Interner, SymbolId};
use rv8_ir::son::IrType;

/// CEP:WHAT: One resolved function signature: interned name, parameter types, return type,
///   and declaration span.
/// CEP:WHY: Inference anchors call arguments and returns; lowering maps call targets to
///   the declaration index.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: types are concrete (the subset has no type parameters).
/// CEP:COST: POD plus one small vector.
/// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_builds_table_in_order.
#[derive(Clone, Debug)]
pub struct FnSig {
    pub name: SymbolId,
    pub params: Vec<IrType>,
    pub ret: IrType,
    pub span: rv8_core::span::Span,
}

/// CEP:WHAT: The function table of the single module, in declaration order.
/// CEP:WHY: Vec order is the deterministic fn index embedded in HIR Call nodes; linear
///   scans replace hash maps so no iteration order can influence output (38.10).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: built only by resolve().
/// CEP:COST: find is O(F) per lookup, F bounded by parser::MAX_FUNCTIONS.
/// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_builds_table_in_order.
#[derive(Clone, Debug)]
pub struct FnTable {
    sigs: Vec<FnSig>,
}

impl FnTable {
    /// CEP:WHAT: Index and signature of the function with the given name.
    /// CEP:WHY: Call-site resolution.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when the name was never declared.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(F).
    /// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_rejects_unknown_call.
    pub fn find(&self, name: SymbolId) -> Option<usize> {
        self.sigs.iter().position(|sig| sig.name == name)
    }

    /// CEP:WHAT: Borrowed signature by declaration index.
    /// CEP:WHY: Inference and lowering read parameter and return types by index.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None for out-of-range indices (impossible for resolve-produced ones).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_builds_table_in_order.
    pub fn sig(&self, index: usize) -> Option<&FnSig> {
        self.sigs.get(index)
    }

    /// CEP:WHAT: All signatures in declaration order.
    /// CEP:WHY: Deterministic iteration over the module surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) borrow.
    /// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_builds_table_in_order.
    pub fn sigs(&self) -> &[FnSig] {
        &self.sigs
    }
}

/// CEP:WHAT: Resolves the module: duplicate check, signature table build, and validation
///   of every call target and arity.
/// CEP:WHY: One pre-pass keeps later passes free of name lookups beyond signatures.
/// CEP:STATUS: complete
/// CEP:FAILURE: DuplicateFunction on the second declaration's span; UnknownFunction and
///   ArityMismatch on the call site's span.
/// CEP:ASSUMES: program came from this crate's parser.
/// CEP:COST: O(F) for the table plus O(total expressions) for call validation.
/// CEP:EVIDENCE: unit tests rv8_frontend::resolve::tests::*.
pub fn resolve(program: &Program, interner: &Interner) -> Result<FnTable, FrontendError> {
    let mut sigs: Vec<FnSig> = Vec::with_capacity(program.functions.len());
    for func in &program.functions {
        if sigs.iter().any(|sig| sig.name == func.name) {
            let name = interner.resolve(func.name).unwrap_or("").to_string();
            return Err(FrontendError::DuplicateFunction { name, span: func.name_span });
        }
        let params: Vec<IrType> = func.params.iter().map(|p| p.ty.to_ir()).collect();
        let ret = func.ret.map(|t| t.to_ir()).unwrap_or(IrType::Unit);
        sigs.push(FnSig { name: func.name, params, ret, span: func.name_span });
    }
    let table = FnTable { sigs };
    for func in &program.functions {
        validate_calls(func, &table, interner)?;
    }
    Ok(table)
}

/// CEP:WHAT: Validates every call expression of one function: target exists and arity
///   matches the signature.
/// CEP:WHY: Fail-closed before inference; spans point at the call site. for_each_stmt
///   already reaches statements nested in construct bodies, so each statement's own
///   expressions are the only thing to check here.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnknownFunction or ArityMismatch with the call's span.
/// CEP:ASSUMES: none
/// CEP:COST: O(statements + expressions); traversal is explicit-stack iterative.
/// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_rejects_unknown_call.
fn validate_calls(
    func: &ast::Function,
    table: &FnTable,
    interner: &Interner,
) -> Result<(), FrontendError> {
    let mut err: Option<FrontendError> = None;
    ast::for_each_stmt(func, &mut |stmt| {
        if err.is_some() {
            return;
        }
        match stmt {
            Stmt::Let { init: Some(expr), .. }
            | Stmt::Assign { value: expr, .. }
            | Stmt::Expr(expr)
            | Stmt::If { cond: expr, .. }
            | Stmt::While { cond: expr, .. }
            | Stmt::Return { value: Some(expr), .. } => {
                check_expr(expr, table, interner, &mut err);
            }
            Stmt::Let { init: None, .. }
            | Stmt::Loop { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Return { value: None, .. } => {}
        }
    });
    if err.is_none() {
        if let Some(t) = &func.body.trailing {
            check_expr(t, table, interner, &mut err);
        }
    }
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// CEP:WHAT: Checks one expression tree for resolvable call targets and arities.
/// CEP:WHY: Shared by statement and trailing-expression validation.
/// CEP:STATUS: complete
/// CEP:FAILURE: records the first UnknownFunction/ArityMismatch into the shared slot.
/// CEP:ASSUMES: none
/// CEP:COST: O(expressions), iterative via for_each_expr.
/// CEP:EVIDENCE: unit test rv8_frontend::resolve::tests::resolve_rejects_bad_arity.
fn check_expr(
    root: &Expr,
    table: &FnTable,
    interner: &Interner,
    err: &mut Option<FrontendError>,
) {
    if err.is_some() {
        return;
    }
    ast::for_each_expr(root, &mut |expr| {
        if err.is_some() {
            return;
        }
        if let ExprKind::Call { callee, args } = &expr.kind {
            match table.find(*callee) {
                None => {
                    let name = interner.resolve(*callee).unwrap_or("").to_string();
                    *err = Some(FrontendError::UnknownFunction { name, span: expr.span });
                }
                Some(index) => {
                    let sig = match table.sig(index) {
                        Some(s) => s,
                        None => {
                            *err = Some(FrontendError::Internal {
                                context: "resolve sig index",
                                span: expr.span,
                            });
                            return;
                        }
                    };
                    if sig.params.len() != args.len() {
                        let name = interner.resolve(*callee).unwrap_or("").to_string();
                        *err = Some(FrontendError::ArityMismatch {
                            name,
                            expected: sig.params.len(),
                            found: args.len(),
                            span: expr.span,
                        });
                    }
                }
            }
        }
    });
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse_program;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Full front half pipeline for tests (test-only panics).
    /// CEP:WHY: Keeps resolve tests short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn stage(src: &str) -> (Program, Interner) {
        let mut ix = Interner::new();
        let trees = lex(src, FileId(0), &mut ix).expect("lex ok");
        let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
        let program = parse_program(&trees, &ix, eof).expect("parse ok");
        (program, ix)
    }

    /// CEP:WHAT: The table is built in declaration order with concrete signatures.
    /// CEP:WHY: The declaration index is the HIR call target; it must be stable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn resolve_builds_table_in_order() {
        let (program, ix) = stage("fn a() -> i32 { 0 } fn b(x: i64) -> i64 { x }");
        let table = resolve(&program, &ix).expect("resolve ok");
        assert_eq!(table.sigs().len(), 2);
        assert_eq!(ix.resolve(table.sig(0).expect("sig0").name), Some("a"));
        assert_eq!(table.sig(0).expect("sig0").ret, IrType::I32);
        assert_eq!(
            table.sig(1).expect("sig1").params,
            vec![IrType::I64]
        );
        assert_eq!(table.find(table.sig(1).expect("sig1").name), Some(1));
    }

    /// CEP:WHAT: A second declaration of the same name is rejected.
    /// CEP:WHY: Negative class: duplicate fn names.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn resolve_rejects_duplicate_names() {
        let (program, ix) = stage("fn f() { } fn f() { }");
        match resolve(&program, &ix) {
            Err(FrontendError::DuplicateFunction { name, .. }) => assert_eq!(name, "f"),
            other => panic!("expected DuplicateFunction, got {other:?}"),
        }
    }

    /// CEP:WHAT: Calls to undeclared functions are rejected at the call span.
    /// CEP:WHY: Negative class: unknown call target, including inside nested bodies.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn resolve_rejects_unknown_call() {
        let (program, ix) = stage("fn f() { if true { while false { g(); } } }");
        match resolve(&program, &ix) {
            Err(FrontendError::UnknownFunction { name, .. }) => assert_eq!(name, "g"),
            other => panic!("expected UnknownFunction, got {other:?}"),
        }
    }

    /// CEP:WHAT: Wrong argument counts are rejected even when the target exists.
    /// CEP:WHY: Negative class: arity mismatch.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn resolve_rejects_bad_arity() {
        let (program, ix) = stage("fn g(x: i32) -> i32 { x } fn f() -> i32 { g(1, 2) }");
        match resolve(&program, &ix) {
            Err(FrontendError::ArityMismatch { name, expected, found, .. }) => {
                assert_eq!(name, "g");
                assert_eq!(expected, 1);
                assert_eq!(found, 2);
            }
            other => panic!("expected ArityMismatch, got {other:?}"),
        }
    }
}
