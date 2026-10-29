// CEP:FILE: crates/rv8_frontend/src/infer.rs
// CEP:WHAT: Union-Find type inference for the subset: an InferenceTable { parent, rank,
//   value } with path compression and union by rank; integer literals default-unify to
//   i32; explicit annotations (parameters, let annotations, signatures) anchor roots;
//   type mismatches are spanned errors. Produces per-expression and per-binding IrTypes.
// CEP:WHY: Standard 5.7 pins union-find inference; the frozen HIR requires a concrete
//   IrType on every expression node, so all types must be determined before lowering.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns FrontendError (TypeMismatch, OperandTypeInvalid, UnresolvedType,
//   UnknownVariable, AssignToImmutable) with spans; never panics on any Program.
// CEP:ASSUMES: resolve() ran first, so call targets and arities are valid; statement
//   nesting is bounded by parser::MAX_PARSE_DEPTH; expression walks are iterative, so
//   operator chain length cannot drive recursion (38.13).
// CEP:COST: near-linear: every union does O(alpha(N)) amortized finds; each expression is
//   visited a constant number of times.
// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::*.

use crate::ast::{
    collect_bindings, Block, ElseClause, Expr, ExprKind, Program, Stmt, UnOp,
};
use crate::error::FrontendError;
use crate::parser::MAX_PARSE_DEPTH;
use crate::resolve::FnTable;
use rv8_core::interner::{Interner, SymbolId};
use rv8_core::span::Span;
use rv8_ir::son::IrType;

/// CEP:WHAT: Error of the raw union-find layer, before spans are attached.
/// CEP:WHY: The table is span-agnostic; callers map failures onto spanned
///   FrontendError variants with context.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnifyError {
    Mismatch { left: IrType, right: IrType },
    Internal,
}

/// CEP:WHAT: The union-find table: parent links, union-by-rank bytes, and the concrete
///   type anchored at each root.
/// CEP:WHY: Standard 5.7 requires union-find inference; value-at-root makes every find
///   answer the whole equivalence class's type.
/// CEP:STATUS: complete
/// CEP:FAILURE: accessors fail closed (None / UnifyError::Internal) on out-of-range node
///   ids instead of indexing panics.
/// CEP:ASSUMES: nodes are only created by fresh().
/// CEP:COST: fresh O(1); find O(alpha(N)) with path compression; union O(alpha(N)) with
///   union by rank.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
#[derive(Default, Debug)]
pub struct InferenceTable {
    parent: Vec<u32>,
    rank: Vec<u8>,
    value: Vec<Option<IrType>>,
}

impl InferenceTable {
    /// CEP:WHAT: Creates an empty table.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
    pub fn new() -> InferenceTable {
        InferenceTable {
            parent: Vec::new(),
            rank: Vec::new(),
            value: Vec::new(),
        }
    }

    /// CEP:WHAT: Allocates one fresh equivalence-class node.
    /// CEP:WHY: Every expression and binding gets its own class before unification.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
    pub fn fresh(&mut self) -> u32 {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.value.push(None);
        id
    }

    /// CEP:WHAT: Finds the root of a node's class with iterative path compression.
    /// CEP:WHY: Flat trees keep later finds near-constant without recursion (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None for out-of-range ids (fail closed, never an index panic).
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
    pub fn find(&mut self, x: u32) -> Option<u32> {
        if (x as usize) >= self.parent.len() {
            return None;
        }
        let mut root = x;
        loop {
            let next = match self.parent.get(root as usize) {
                Some(&p) => p,
                None => return None,
            };
            if next == root {
                break;
            }
            root = next;
        }
        let mut cur = x;
        while cur != root {
            let next = match self.parent.get(cur as usize) {
                Some(&p) => p,
                None => break,
            };
            if next == root {
                break;
            }
            if let Some(slot) = self.parent.get_mut(cur as usize) {
                *slot = root;
            }
            cur = next;
        }
        Some(root)
    }

    /// CEP:WHAT: Merges two classes by rank, merging anchored values; conflicting anchors
    ///   are a Mismatch.
    /// CEP:WHY: Union by rank bounds tree height; the value merge is where type conflicts
    ///   surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Mismatch on two different concrete anchors; Internal on invalid ids.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
    pub fn union(&mut self, a: u32, b: u32) -> Result<(), UnifyError> {
        let ra = self.find(a).ok_or(UnifyError::Internal)?;
        let rb = self.find(b).ok_or(UnifyError::Internal)?;
        if ra == rb {
            return Ok(());
        }
        let rank_a = *self.rank.get(ra as usize).ok_or(UnifyError::Internal)?;
        let rank_b = *self.rank.get(rb as usize).ok_or(UnifyError::Internal)?;
        let (new_root, old_root) = if rank_a >= rank_b { (ra, rb) } else { (rb, ra) };
        let value_new = self.value.get(new_root as usize).copied().flatten();
        let value_old = self.value.get(old_root as usize).copied().flatten();
        let merged = match (value_new, value_old) {
            (Some(x), Some(y)) => {
                if x != y {
                    return Err(UnifyError::Mismatch { left: x, right: y });
                }
                Some(x)
            }
            (Some(x), None) => Some(x),
            (None, Some(y)) => Some(y),
            (None, None) => None,
        };
        if let Some(slot) = self.parent.get_mut(old_root as usize) {
            *slot = new_root;
        }
        if rank_a == rank_b {
            if let Some(slot) = self.rank.get_mut(new_root as usize) {
                *slot = slot.saturating_add(1);
            }
        }
        if let Some(slot) = self.value.get_mut(new_root as usize) {
            *slot = merged;
        }
        if let Some(slot) = self.value.get_mut(old_root as usize) {
            *slot = None;
        }
        Ok(())
    }

    /// CEP:WHAT: Anchors a class to one concrete type.
    /// CEP:WHY: Annotations (params, lets, signatures) are the roots that make every
    ///   literal default resolvable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Mismatch when the class already has a different concrete type.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_annotations_anchor_roots.
    pub fn constrain(&mut self, x: u32, t: IrType) -> Result<(), UnifyError> {
        let rx = self.find(x).ok_or(UnifyError::Internal)?;
        match self.value.get(rx as usize).copied().flatten() {
            Some(existing) if existing != t => Err(UnifyError::Mismatch { left: t, right: existing }),
            _ => {
                if let Some(slot) = self.value.get_mut(rx as usize) {
                    *slot = Some(t);
                }
                Ok(())
            }
        }
    }

    /// CEP:WHAT: The concrete type of a class, if anchored.
    /// CEP:WHY: Finalization reads anchors; unresolved classes drive the i32 fallback.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when the class has no anchor yet.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::union_find_find_union_rank.
    pub fn type_of(&mut self, x: u32) -> Option<IrType> {
        let rx = self.find(x)?;
        self.value.get(rx as usize).copied().flatten()
    }
}

/// CEP:WHAT: Inference results: one IrType per expression key and per function binding.
/// CEP:WHY: Lowering reads these instead of re-deriving types.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: produced only by infer_program.
/// CEP:COST: flat vectors.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_types_fixture_expressions.
#[derive(Clone, Debug)]
pub struct Inference {
    pub expr_types: Vec<IrType>,
    pub fn_var_types: Vec<Vec<IrType>>,
}

/// CEP:WHAT: Post-finalization operand requirements.
/// CEP:WHY: Requirements like "integer operand" are checks over resolved types, not
///   unification constraints.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_bool_arithmetic.
#[derive(Clone, Copy, Debug)]
enum TypeCheck {
    Integral,
    IsBool,
}

/// CEP:WHAT: One function's inference state: union-find table, binding nodes, scope
///   stack, and deferred finalization records.
/// CEP:WHY: Per-function isolation keeps tables small and errors local.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: bindings were enumerated by ast::collect_bindings.
/// CEP:COST: O(exprs + bindings).
/// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::*.
struct FnInfer<'a> {
    sig: &'a crate::resolve::FnSig,
    fn_table: &'a FnTable,
    interner: &'a Interner,
    table: InferenceTable,
    bindings: Vec<crate::ast::BindingInfo>,
    var_nodes: Vec<u32>,
    next_binding: usize,
    scope: Vec<(SymbolId, u32, bool)>,
    key_tv: Vec<(u32, u32, Span)>,
    lit_defaults: Vec<(u32, Span)>,
    checks: Vec<(TypeCheck, u32, Span)>,
}

/// CEP:WHAT: Reports whether an IrType is one of the subset's integer widths.
/// CEP:WHY: Arithmetic operand checks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_bool_arithmetic.
fn ir_is_integer(t: IrType) -> bool {
    matches!(t, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64)
}

impl<'a> FnInfer<'a> {
    /// CEP:WHAT: Maps a UnifyError onto a spanned FrontendError with context.
    /// CEP:WHY: The table is span-agnostic; call sites attach meaning.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_mismatched_params.
    fn unify_err(&self, err: UnifyError, context: &'static str, span: Span) -> FrontendError {
        match err {
            UnifyError::Mismatch { left, right } => FrontendError::TypeMismatch {
                context,
                expected: left,
                found: right,
                span,
            },
            UnifyError::Internal => FrontendError::Internal { context: "unify", span },
        }
    }

    /// CEP:WHAT: Anchors a node to a type with a spanned error context.
    /// CEP:WHY: Shared constrain path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: TypeMismatch via unify_err.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_non_bool_condition.
    fn constrain_span(
        &mut self,
        node: u32,
        ty: IrType,
        context: &'static str,
        span: Span,
    ) -> Result<(), FrontendError> {
        self.table
            .constrain(node, ty)
            .map_err(|e| self.unify_err(e, context, span))
    }

    /// CEP:WHAT: Unifies two nodes with a spanned error context.
    /// CEP:WHY: Shared union path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: TypeMismatch via unify_err.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(alpha(N)).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_mismatched_params.
    fn union_span(
        &mut self,
        a: u32,
        b: u32,
        context: &'static str,
        span: Span,
    ) -> Result<(), FrontendError> {
        self.table
            .union(a, b)
            .map_err(|e| self.unify_err(e, context, span))
    }

    /// CEP:WHAT: Resolves a name to its binding node and mutability by reverse scope scan.
    /// CEP:WHY: Lexical scoping with shadowing; the innermost binding wins.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnknownVariable for names never bound at this position (including
    ///   use-before-declaration).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(scope depth).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_rejects_use_before_declare.
    fn lookup(&self, name: SymbolId, span: Span) -> Result<(u32, bool), FrontendError> {
        for (n, node, mutable) in self.scope.iter().rev() {
            if *n == name {
                return Ok((*node, *mutable));
            }
        }
        let text = self.interner.resolve(name).unwrap_or("").to_string();
        Err(FrontendError::UnknownVariable { name: text, span })
    }

    /// CEP:WHAT: Takes the next binding in shared pre-order and returns its node.
    /// CEP:WHY: infer and lower must agree on binding identity; a mismatch is an internal
    ///   invariant violation (fail closed).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal when the walk order diverges from collect_bindings.
    /// CEP:ASSUMES: statement walks follow source pre-order.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_types_fixture_expressions.
    fn take_binding(&mut self, name: SymbolId, span: Span) -> Result<(u32, bool), FrontendError> {
        let expected = self
            .bindings
            .get(self.next_binding)
            .ok_or(FrontendError::Internal { context: "binding cursor", span })?;
        if expected.name != name {
            return Err(FrontendError::Internal { context: "binding order", span });
        }
        let node = *self
            .var_nodes
            .get(self.next_binding)
            .ok_or(FrontendError::Internal { context: "binding node", span })?;
        self.next_binding += 1;
        Ok((node, expected.mutable))
    }

    /// CEP:WHAT: Types one expression tree with an iterative post-order walk.
    /// CEP:WHY: Left-associated chains of arbitrary length must not recurse (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnknownVariable, TypeMismatch (immediate anchors), and Internal
    ///   failures, all with spans.
    /// CEP:ASSUMES: resolve() validated calls.
    /// CEP:COST: O(nodes); each pushed and popped once.
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_types_fixture_expressions.
    fn type_expr(&mut self, root: &Expr) -> Result<u32, FrontendError> {
        enum Frame<'e> {
            Enter(&'e Expr),
            Combine(&'e Expr),
        }
        let mut work: Vec<Frame> = vec![Frame::Enter(root)];
        let mut vals: Vec<u32> = Vec::new();
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Enter(expr) => {
                    work.push(Frame::Combine(expr));
                    match &expr.kind {
                        ExprKind::Binary(_, a, b) => {
                            work.push(Frame::Enter(b));
                            work.push(Frame::Enter(a));
                        }
                        ExprKind::Unary(_, a) => work.push(Frame::Enter(a)),
                        ExprKind::Call { args, .. } => {
                            for arg in args.iter().rev() {
                                work.push(Frame::Enter(arg));
                            }
                        }
                        ExprKind::Paren(p) => work.push(Frame::Enter(p)),
                        ExprKind::IntLit(_) | ExprKind::BoolLit(_) | ExprKind::Var(_) => {}
                    }
                }
                Frame::Combine(expr) => {
                    let node = match &expr.kind {
                        ExprKind::IntLit(_) => {
                            let n = self.table.fresh();
                            self.lit_defaults.push((n, expr.span));
                            n
                        }
                        ExprKind::BoolLit(b) => {
                            let n = self.table.fresh();
                            self.constrain_span(n, IrType::Bool, "boolean literal", expr.span)?;
                            let _ = b;
                            n
                        }
                        ExprKind::Var(sym) => {
                            let (target, _) = self.lookup(*sym, expr.span)?;
                            let n = self.table.fresh();
                            self.union_span(n, target, "variable read", expr.span)?;
                            n
                        }
                        ExprKind::Unary(op, _) => {
                            let operand = vals.pop().ok_or(FrontendError::Internal {
                                context: "infer unary arity",
                                span: expr.span,
                            })?;
                            let n = self.table.fresh();
                            match op {
                                UnOp::Neg => {
                                    self.union_span(n, operand, "negation result", expr.span)?;
                                    self.checks.push((TypeCheck::Integral, operand, expr.span));
                                }
                                UnOp::Not => {
                                    self.checks.push((TypeCheck::IsBool, operand, expr.span));
                                    self.constrain_span(n, IrType::Bool, "logical not", expr.span)?;
                                }
                            }
                            n
                        }
                        ExprKind::Binary(op, _, _) => {
                            let rhs = vals.pop().ok_or(FrontendError::Internal {
                                context: "infer binary arity",
                                span: expr.span,
                            })?;
                            let lhs = vals.pop().ok_or(FrontendError::Internal {
                                context: "infer binary arity",
                                span: expr.span,
                            })?;
                            let n = self.table.fresh();
                            if op.result_is_bool() {
                                self.union_span(lhs, rhs, "comparison operands", expr.span)?;
                                self.constrain_span(n, IrType::Bool, "comparison result", expr.span)?;
                            } else {
                                self.union_span(lhs, rhs, "binary operands", expr.span)?;
                                self.union_span(n, lhs, "arithmetic result", expr.span)?;
                                self.checks.push((TypeCheck::Integral, lhs, expr.span));
                            }
                            n
                        }
                        ExprKind::Call { callee, args } => {
                            let arity = args.len();
                            let mut argvs: Vec<u32> = Vec::with_capacity(arity);
                            for _ in 0..arity {
                                argvs.push(vals.pop().ok_or(FrontendError::Internal {
                                    context: "infer call arity",
                                    span: expr.span,
                                })?);
                            }
                            argvs.reverse();
                            let index = self.fn_table.find(*callee).ok_or(
                                FrontendError::Internal { context: "infer callee", span: expr.span },
                            )?;
                            let sig = self
                                .fn_table
                                .sig(index)
                                .ok_or(FrontendError::Internal {
                                    context: "infer callee sig",
                                    span: expr.span,
                                })?;
                            if sig.params.len() != argvs.len() {
                                return Err(FrontendError::Internal {
                                    context: "infer call param count",
                                    span: expr.span,
                                });
                            }
                            for (av, pt) in argvs.iter().zip(sig.params.iter()) {
                                self.constrain_span(*av, *pt, "call argument", expr.span)?;
                            }
                            let n = self.table.fresh();
                            self.constrain_span(n, sig.ret, "call result", expr.span)?;
                            n
                        }
                        ExprKind::Paren(_) => {
                            let child = vals.pop().ok_or(FrontendError::Internal {
                                context: "infer paren arity",
                                span: expr.span,
                            })?;
                            self.key_tv.push((expr.key.0, child, expr.span));
                            vals.push(child);
                            continue;
                        }
                    };
                    self.key_tv.push((expr.key.0, node, expr.span));
                    vals.push(node);
                }
            }
        }
        vals.pop().ok_or(FrontendError::Internal { context: "infer expr root", span: root.span })
    }

    /// CEP:WHAT: Types a block with its own lexical scope.
    /// CEP:WHY: Blocks introduce scopes; the fn body constrains its trailing expression
    ///   to the return type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: propagates statement failures.
    /// CEP:ASSUMES: nesting bounded by parser::MAX_PARSE_DEPTH (checked per level).
    /// CEP:COST: O(block size).
    /// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::infer_shadowing_and_scopes.
    fn walk_block(&mut self, block: &Block, is_fn_body: bool, depth: u32) -> Result<(), FrontendError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(FrontendError::Internal { context: "infer block depth", span: block.span });
        }
        let mark = self.scope.len();
        for stmt in &block.stmts {
            self.walk_stmt(stmt, depth + 1)?;
        }
        if let Some(t) = &block.trailing {
            let tv = self.type_expr(t)?;
            if is_fn_body {
                self.constrain_span(tv, self.sig.ret, "function result", t.span)?;
            }
        }
        self.scope.truncate(mark);
        Ok(())
    }

    /// CEP:WHAT: Types one statement.
    /// CEP:WHY: The core inference rules: annotations anchor, init unifies, conditions
    ///   are bool, calls check signatures, returns check the signature; else-if chain
    ///   recursion is bounded by the depth budget.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: all semantic rejection classes with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(statement size).
    /// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::*.
    fn walk_stmt(&mut self, stmt: &Stmt, depth: u32) -> Result<(), FrontendError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(FrontendError::Internal {
                context: "infer stmt depth",
                span: match stmt {
                    Stmt::Let { span, .. }
                    | Stmt::Assign { span, .. }
                    | Stmt::If { span, .. }
                    | Stmt::While { span, .. }
                    | Stmt::Loop { span, .. }
                    | Stmt::Break { span }
                    | Stmt::Continue { span }
                    | Stmt::Return { span, .. } => *span,
                    Stmt::Expr(e) => e.span,
                },
            });
        }
        match stmt {
            Stmt::Let { name, ty, init, span, .. } => {
                let (node, mutable) = self.take_binding(*name, *span)?;
                if let Some(t) = ty {
                    self.constrain_span(node, t.to_ir(), "let annotation", *span)?;
                }
                if let Some(e) = init {
                    let tv = self.type_expr(e)?;
                    self.union_span(node, tv, "let initializer", *span)?;
                }
                self.scope.push((*name, node, mutable));
            }
            Stmt::Assign { name, value, span } => {
                let (node, mutable) = self.lookup(*name, *span)?;
                if !mutable {
                    let text = self.interner.resolve(*name).unwrap_or("").to_string();
                    return Err(FrontendError::AssignToImmutable { name: text, span: *span });
                }
                let tv = self.type_expr(value)?;
                self.union_span(node, tv, "assignment", *span)?;
            }
            Stmt::Expr(expr) => {
                self.type_expr(expr)?;
            }
            Stmt::If { cond, then_block, else_clause, span } => {
                let cv = self.type_expr(cond)?;
                self.constrain_span(cv, IrType::Bool, "if condition", *span)?;
                self.walk_block(then_block, false, depth)?;
                if let Some(clause) = else_clause {
                    let mark = self.scope.len();
                    match clause {
                        ElseClause::Block(b) => self.walk_block(b, false, depth)?,
                        ElseClause::If(inner) => self.walk_stmt(inner, depth + 1)?,
                    }
                    self.scope.truncate(mark);
                }
            }
            Stmt::While { cond, body, span } => {
                let cv = self.type_expr(cond)?;
                self.constrain_span(cv, IrType::Bool, "while condition", *span)?;
                self.walk_block(body, false, depth)?;
            }
            Stmt::Loop { body, .. } => {
                self.walk_block(body, false, depth)?;
            }
            Stmt::Break { .. } | Stmt::Continue { .. } => {}
            Stmt::Return { value, span } => match value {
                Some(e) => {
                    let tv = self.type_expr(e)?;
                    self.constrain_span(tv, self.sig.ret, "return value", *span)?;
                }
                None => {
                    if self.sig.ret != IrType::Unit {
                        return Err(FrontendError::TypeMismatch {
                            context: "return value",
                            expected: self.sig.ret,
                            found: IrType::Unit,
                            span: *span,
                        });
                    }
                }
            },
        }
        Ok(())
    }

}

/// CEP:WHAT: Infers concrete IrTypes for the whole program.
/// CEP:WHY: The HIR requires a type on every node; this is the only sanctioned producer.
/// CEP:STATUS: complete
/// CEP:FAILURE: the first semantic error with its span; never panics.
/// CEP:ASSUMES: resolve() ran first.
/// CEP:COST: O(total expressions and bindings) with near-linear union-find.
/// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::* and the end-to-end fixture test.
pub fn infer_program(
    program: &Program,
    fn_table: &FnTable,
    interner: &Interner,
) -> Result<Inference, FrontendError> {
    let mut all_expr_slots: Vec<(u32, IrType)> = Vec::new();
    let mut fn_var_types: Vec<Vec<IrType>> = Vec::with_capacity(program.functions.len());
    for (index, func) in program.functions.iter().enumerate() {
        let sig = fn_table
            .sig(index)
            .ok_or(FrontendError::Internal { context: "infer sig", span: func.name_span })?;
        let bindings = collect_bindings(func)?;
        let mut state = FnInfer {
            sig,
            fn_table,
            interner,
            table: InferenceTable::new(),
            var_nodes: Vec::with_capacity(bindings.len()),
            bindings,
            next_binding: 0,
            scope: Vec::new(),
            key_tv: Vec::new(),
            lit_defaults: Vec::new(),
            checks: Vec::new(),
        };
        for (i, binding) in state.bindings.iter().enumerate() {
            let node = state.table.fresh();
            if let Some(&param_ty) = sig.params.get(i) {
                state
                    .table
                    .constrain(node, param_ty)
                    .map_err(|e| state.unify_err(e, "parameter type", binding.span))?;
            }
            state.var_nodes.push(node);
        }
        // Parameters are in lexical scope for the entire body.
        for (i, param) in func.params.iter().enumerate() {
            let node = *state.var_nodes.get(i).ok_or(FrontendError::Internal {
                context: "infer param node",
                span: param.span,
            })?;
            state.scope.push((param.name, node, param.mutable));
        }
        state.next_binding = func.params.len();
        state.walk_block(&func.body, true, 1)?;

        // Integer literal rule, named once: an unconstrained literal class becomes i32;
        // a literal class anchored to a non-integer type (e.g. `let x: bool = 5;`) is a
        // rejected class, because no integer literal can be a Bool.
        const INTEGER_FALLBACK: IrType = IrType::I32;
        let defaults = std::mem::take(&mut state.lit_defaults);
        for (node, span) in defaults {
            match state.table.type_of(node) {
                None => {
                    state.constrain_span(node, INTEGER_FALLBACK, "integer literal", span)?;
                }
                Some(t) if !ir_is_integer(t) => {
                    return Err(FrontendError::OperandTypeInvalid {
                        requirement: "an integer type",
                        found: t,
                        span,
                    });
                }
                Some(_) => {}
            }
        }
        // Unannotated, never-unified bindings fall back to i32 like Rust's integer
        // variable fallback.
        let binding_infos = state.bindings.clone();
        let var_nodes = state.var_nodes.clone();
        let mut var_types: Vec<IrType> = Vec::with_capacity(binding_infos.len());
        for (i, binding) in binding_infos.iter().enumerate() {
            let node = *var_nodes.get(i).ok_or(FrontendError::Internal {
                context: "infer binding node",
                span: binding.span,
            })?;
            if state.table.type_of(node).is_none() && binding.ty.is_none() {
                state.constrain_span(node, INTEGER_FALLBACK, "unannotated binding", binding.span)?;
            }
            match state.table.type_of(node) {
                Some(t) => var_types.push(t),
                None => return Err(FrontendError::UnresolvedType { span: binding.span }),
            }
        }
        fn_var_types.push(var_types);

        for (key, node, span) in std::mem::take(&mut state.key_tv) {
            let ty = match state.table.type_of(node) {
                Some(t) => t,
                None => return Err(FrontendError::UnresolvedType { span }),
            };
            all_expr_slots.push((key, ty));
        }
        let checks = std::mem::take(&mut state.checks);
        for (check, node, span) in checks {
            let found = match state.table.type_of(node) {
                Some(t) => t,
                None => return Err(FrontendError::UnresolvedType { span }),
            };
            match check {
                TypeCheck::Integral => {
                    if !ir_is_integer(found) {
                        return Err(FrontendError::OperandTypeInvalid {
                            requirement: "an integer type",
                            found,
                            span,
                        });
                    }
                }
                TypeCheck::IsBool => {
                    if found != IrType::Bool {
                        return Err(FrontendError::OperandTypeInvalid {
                            requirement: "bool",
                            found,
                            span,
                        });
                    }
                }
            }
        }
    }
    // Slot per-key types into a dense vector addressed by ExprKey. Keys are dense over
    // the program (parser-assigned), so the highest key bounds the table.
    let mut max_key: i64 = -1;
    for (key, _) in &all_expr_slots {
        if (*key as i64) > max_key {
            max_key = *key as i64;
        }
    }
    let total = (max_key + 1).max(0) as usize;
    let mut typed: Vec<Option<IrType>> = vec![None; total];
    for (key, ty) in all_expr_slots.drain(..) {
        if let Some(slot) = typed.get_mut(key as usize) {
            *slot = Some(ty);
        }
    }
    let mut final_types: Vec<IrType> = Vec::with_capacity(total);
    for slot in typed.into_iter() {
        match slot {
            Some(t) => final_types.push(t),
            None => {
                return Err(FrontendError::Internal {
                    context: "expr key coverage",
                    span: Span::new(rv8_core::span::FileId(0), 0, 0),
                });
            }
        }
    }
    Ok(Inference { expr_types: final_types, fn_var_types })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse_program;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Runs lex+parse+resolve for inference tests (test-only panics).
    /// CEP:WHY: Keeps inference tests short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn stage(src: &str) -> (Program, FnTable, Interner) {
        let mut ix = Interner::new();
        let trees = lex(src, FileId(0), &mut ix).expect("lex ok");
        let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
        let program = parse_program(&trees, &ix, eof).expect("parse ok");
        let table = crate::resolve::resolve(&program, &ix).expect("resolve ok");
        (program, table, ix)
    }

    /// CEP:WHAT: Union-find find/union/rank/value-merge behavior.
    /// CEP:WHY: The 5.7 mechanism contract: path compression flattens, ranks grow, and
    ///   conflicting anchors are mismatches.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn union_find_find_union_rank() {
        let mut t = InferenceTable::new();
        let a = t.fresh();
        let b = t.fresh();
        let c = t.fresh();
        t.constrain(a, IrType::I64).expect("anchor a");
        t.union(b, c).expect("union b c");
        t.union(a, b).expect("union a bc");
        let ra = t.find(a).expect("root a");
        assert_eq!(t.find(b), Some(ra));
        assert_eq!(t.find(c), Some(ra));
        assert_eq!(t.type_of(c), Some(IrType::I64));
        let d = t.fresh();
        t.constrain(d, IrType::Bool).expect("anchor d");
        match t.union(c, d) {
            Err(UnifyError::Mismatch { left, right }) => {
                assert_ne!(left, right);
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
        assert_eq!(t.find(9999), None, "out-of-range find fails closed");
    }

    /// CEP:WHAT: Fixture expression types: literals and arithmetic are i32, calls return
    ///   the signature type.
    /// CEP:WHY: The mandated fixture must type exactly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn inference_types_fixture_expressions() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() -> i32 { add(6, 7) }";
        let (program, table, ix) = stage(src);
        let inference = infer_program(&program, &table, &ix).expect("infer ok");
        assert_eq!(inference.fn_var_types.len(), 2);
        assert_eq!(
            inference.fn_var_types[0],
            vec![IrType::I32, IrType::I32],
            "params a and b"
        );
        assert!(!inference.expr_types.is_empty());
        assert!(inference
            .expr_types
            .iter()
            .all(|t| matches!(t, IrType::I32)));
    }

    /// CEP:WHAT: Annotations anchor roots and absorb unanchored literals.
    /// CEP:WHY: `let x: u64 = 5;` must type x as U64, not the i32 default.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn inference_annotations_anchor_roots() {
        let src = "fn f() -> u64 { let x: u64 = 5; x }";
        let (program, table, ix) = stage(src);
        let inference = infer_program(&program, &table, &ix).expect("infer ok");
        assert!(inference
            .expr_types
            .iter()
            .any(|t| matches!(t, IrType::U64)));
        assert_eq!(inference.fn_var_types[0], vec![IrType::U64]);
    }

    /// CEP:WHAT: Mismatched call argument types are rejected with spans.
    /// CEP:WHY: Negative class: `fn(a: bool, b: i32) -> i32 { a + b }` and bool args.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_mismatched_params() {
        let src = "fn g(x: bool) -> i32 { 1 } fn f(a: bool, b: i32) -> i32 { a + b }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::OperandTypeInvalid { .. })
            | Err(FrontendError::TypeMismatch { .. }) => {}
            other => panic!("expected a type error, got {other:?}"),
        }
        let src2 = "fn g(x: bool) -> i32 { 1 } fn f() -> i32 { g(3) }";
        let (program2, table2, ix2) = stage(src2);
        match infer_program(&program2, &table2, &ix2) {
            Err(FrontendError::OperandTypeInvalid { found, .. }) => {
                assert_eq!(found, IrType::Bool);
            }
            other => panic!("expected OperandTypeInvalid for bool arg, got {other:?}"),
        }
    }

    /// CEP:WHAT: Non-bool conditions are rejected.
    /// CEP:WHY: Negative class: `if 1 { }` — an integer literal can never be a condition.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_non_bool_condition() {
        let src = "fn f() { if 1 { } }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::OperandTypeInvalid { found, .. }) => {
                assert_eq!(found, IrType::Bool);
            }
            other => panic!("expected OperandTypeInvalid, got {other:?}"),
        }
    }

    /// CEP:WHAT: Bool operands in arithmetic are rejected by the deferred check.
    /// CEP:WHY: Negative class: `true + false`.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_bool_arithmetic() {
        let src = "fn f() -> bool { true + false }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::OperandTypeInvalid { requirement, found, .. }) => {
                assert!(requirement.contains("integer"));
                assert_eq!(found, IrType::Bool);
            }
            other => panic!("expected OperandTypeInvalid, got {other:?}"),
        }
    }

    /// CEP:WHAT: Use before declaration and unknown names are rejected.
    /// CEP:WHY: Negative class: unknown variable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_use_before_declare() {
        let src = "fn f() -> i32 { y = 1; let y = 0; y }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::UnknownVariable { name, .. }) => assert_eq!(name, "y"),
            other => panic!("expected UnknownVariable, got {other:?}"),
        }
    }

    /// CEP:WHAT: Assignment to an immutable binding is rejected.
    /// CEP:WHY: Negative class: mutability enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_assign_to_immutable() {
        let src = "fn f() { let x = 1; x = 2; }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::AssignToImmutable { name, .. }) => assert_eq!(name, "x"),
            other => panic!("expected AssignToImmutable, got {other:?}"),
        }
        let src2 = "fn f(mut x: i32) { x = 2; }";
        let (program2, table2, ix2) = stage(src2);
        assert!(infer_program(&program2, &table2, &ix2).is_ok());
    }

    /// CEP:WHAT: Bare `return;` in a value-returning function is rejected.
    /// CEP:WHY: Negative class: return-type mismatch.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_rejects_bare_return_with_ret_type() {
        let src = "fn f() -> i32 { return; }";
        let (program, table, ix) = stage(src);
        match infer_program(&program, &table, &ix) {
            Err(FrontendError::TypeMismatch { expected, found, .. }) => {
                assert_eq!(expected, IrType::I32);
                assert_eq!(found, IrType::Unit);
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
    }

    /// CEP:WHAT: Shadowing resolves to the innermost binding; scopes close correctly.
    /// CEP:WHY: Lexical scoping rule of the subset.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn infer_shadowing_and_scopes() {
        let src = "fn f() -> i64 { let x: i64 = 1; if true { let mut x: i32 = 2; x = 3; } x }";
        let (program, table, ix) = stage(src);
        let inference = infer_program(&program, &table, &ix).expect("infer ok");
        assert_eq!(
            inference.fn_var_types[0],
            vec![IrType::I64, IrType::I32],
            "outer x stays i64; inner x is i32"
        );
    }
}
