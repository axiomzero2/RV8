// CEP:FILE: crates/rv8_frontend/src/lower.rs
// CEP:WHAT: AST-to-HIR lowering using only the frozen rv8_ir::hir construction API: the
//   function body becomes a region tree; if/else become conditional regions with exactly
//   two arms; while/loop become loop regions with exactly one header and one latch;
//   break/continue are validated against the enclosing loop region; bindings are hoisted
//   as zero-initialized root declarations so every construct obeys the flattener's
//   binding-availability contract; and a tail return is synthesized.
// CEP:WHY: The frontend output must satisfy spec 1.3 and be consumable by
//   rv8_ir::lowering::hir_to_son; hoisting exists because the flattener builds loop
//   header phis over all variables written in a loop subtree and join merges over all
//   variables written in an arm, so every such variable must be bound before the construct.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns FrontendError (BreakOutsideLoop, ContinueOutsideLoop, and Internal
//   fail-closed guards) with spans; never panics on any verified input.
// CEP:ASSUMES: resolve and infer ran first (names, calls, and types are all determined);
//   statement nesting is bounded by parser::MAX_PARSE_DEPTH; expression lowering is
//   iterative so operator chains cannot drive recursion (38.13).
// CEP:COST: O(AST size); each statement and expression node is emitted exactly once.
// CEP:EVIDENCE: unit tests rv8_frontend::lower::tests::* and the end-to-end fixture test.
// CEP:NOTE: Parameters thread through the IR natively: HirFunction::set_params registers
//   the ABI argument order, and hir_to_son binds each to an Opcode::Parameter entry node
//   (downstream: ComputeOp::Param, then ABI pre-coloring). Locals are hoisted as
//   zero-initialized root declarations; every other construct lowers with full source
//   fidelity.

use crate::ast::{
    collect_bindings, BindingInfo, Block, ElseClause, Expr, ExprKind, Function, Program, Stmt,
};
use crate::error::FrontendError;
use crate::infer::Inference;
use crate::parser::MAX_PARSE_DEPTH;
use crate::resolve::FnTable;
use rv8_core::interner::{Interner, SymbolId};
use rv8_core::span::Span;
use rv8_ir::hir::{
    ExprId, ExprKind as HirExprKind, HirFunction, RegionId, RegionKind, StmtKind, VarId,
};
use rv8_ir::son::IrType;

/// CEP:WHAT: Lowers a whole program into one HirFunction per declaration, in order.
/// CEP:WHY: compile_source's output contract; order is declaration order (deterministic).
/// CEP:STATUS: complete
/// CEP:FAILURE: the first per-function lowering error with its span.
/// CEP:ASSUMES: inference results align with collect_bindings order (checked per function).
/// CEP:COST: O(total AST nodes).
/// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_fixture_add_main.
pub fn lower_program(
    program: &Program,
    fn_table: &FnTable,
    inference: &Inference,
    interner: &Interner,
) -> Result<Vec<HirFunction>, FrontendError> {
    let mut out = Vec::with_capacity(program.functions.len());
    for (index, func) in program.functions.iter().enumerate() {
        out.push(lower_function(func, index, fn_table, inference, interner)?);
    }
    Ok(out)
}

/// CEP:WHAT: One function's lowering state: the HIR under construction, the binding-to-
///   VarId map, the lexical scope stack, and the enclosing-loop stack.
/// CEP:WHY: One owner of mutable state keeps region wiring total and borrow-clean.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(nodes emitted).
/// CEP:EVIDENCE: unit tests rv8_frontend::lower::tests::*.
struct FnLower<'a> {
    interner: &'a Interner,
    inference: &'a Inference,
    fn_table: &'a FnTable,
    var_types: &'a [IrType],
    bindings: Vec<BindingInfo>,
    var_ids: Vec<VarId>,
    next_binding: usize,
    hir: HirFunction,
    scope: Vec<(SymbolId, VarId, bool)>,
    loops: Vec<RegionId>,
}

/// CEP:WHAT: Lowers one function into a verified-shape HirFunction.
/// CEP:WHY: Pipeline step; per-function isolation keeps state small.
/// CEP:STATUS: complete
/// CEP:FAILURE: Inference alignment failures surface as Internal; statement failures
///   propagate with spans.
/// CEP:ASSUMES: fn_table.sig at the declaration index exists and the matching
///   inference.fn_var_types entry matches
///   collect_bindings order.
/// CEP:COST: O(function AST).
/// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_fixture_region_shapes.
fn lower_function(
    func: &Function,
    index: usize,
    fn_table: &FnTable,
    inference: &Inference,
    interner: &Interner,
) -> Result<HirFunction, FrontendError> {
    let sig = fn_table
        .sig(index)
        .ok_or(FrontendError::Internal { context: "lower sig", span: func.name_span })?;
    let var_types = inference
        .fn_var_types
        .get(index)
        .ok_or(FrontendError::Internal { context: "lower var types", span: func.name_span })?;
    let bindings = collect_bindings(func)?;
    if bindings.len() != var_types.len() {
        return Err(FrontendError::Internal { context: "lower binding alignment", span: func.name_span });
    }
    let name = interner.resolve(func.name).unwrap_or("").to_string();
    let mut state = FnLower {
        interner,
        inference,
        fn_table,
        var_types,
        bindings: bindings.clone(),
        var_ids: Vec::with_capacity(bindings.len()),
        next_binding: 0,
        hir: HirFunction::new(&name),
        scope: Vec::new(),
        loops: Vec::new(),
    };
    let root = state.hir.add_region(RegionKind::Block, None, func.body.span);
    // Hoist every local binding as a zero-initialized root declaration, in binding order.
    // Params (the first binding slots) are declared as root vars and registered through
    // HirFunction::set_params: flattening binds them to Opcode::Parameter entry nodes, so
    // incoming argument values flow from the ABI (no zero-init).
    for (k, binding) in state.bindings.iter().enumerate() {
        let ty = *state
            .var_types
            .get(k)
            .ok_or(FrontendError::Internal { context: "lower hoist type", span: binding.span })?;
        let vid = state.hir.add_var(state.interner.resolve(binding.name).unwrap_or(""), ty, root);
        if k >= func.params.len() {
            let zero = state.hir.add_expr(HirExprKind::Const(0), ty, binding.span);
            state.hir.add_stmt(root, StmtKind::Let { var: vid, init: Some(zero) }, binding.span);
        }
        state.var_ids.push(vid);
    }
    // Params occupy the first binding slots; seed the scope with them and register the
    // ABI argument order on the HIR (verify enforces the contract).
    let param_count = func.params.len();
    if param_count > 0 {
        let param_vars: Vec<VarId> = state.var_ids[..param_count].to_vec();
        state
            .hir
            .set_params(param_vars)
            .map_err(|_| FrontendError::Internal { context: "set_params", span: func.name_span })?;
    }
    for (k, param) in func.params.iter().enumerate() {
        let vid = *state
            .var_ids
            .get(k)
            .ok_or(FrontendError::Internal { context: "lower param var", span: param.span })?;
        state.scope.push((param.name, vid, param.mutable));
    }
    state.next_binding = param_count;
    let tail = state.lower_block_content(&func.body, root, true, 1)?;
    // Synthesize the fall-through return. The flattener stops emitting a region's
    // statements at the first transfer, so this is dead on paths that already returned.
    let end_span = Span::new(func.body.span.file, func.body.span.hi, func.body.span.hi);
    if sig.ret == IrType::Unit {
        state.hir.add_stmt(tail, StmtKind::Return(None), end_span);
    } else {
        let zero = state.hir.add_expr(HirExprKind::Const(0), sig.ret, end_span);
        state.hir.add_stmt(tail, StmtKind::Return(Some(zero)), end_span);
    }
    Ok(state.hir)
}

impl FnLower<'_> {
    /// CEP:WHAT: Looks up a variable's HIR id and mutability by reverse scope scan.
    /// CEP:WHY: Lowering must bind reads to the innermost declaration.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal (inference guarantees the name resolved).
    /// CEP:ASSUMES: infer ran first.
    /// CEP:COST: O(scope depth).
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_fixture_region_shapes.
    fn lookup(&self, name: SymbolId, span: Span) -> Result<VarId, FrontendError> {
        for (n, vid, _) in self.scope.iter().rev() {
            if *n == name {
                return Ok(*vid);
            }
        }
        Err(FrontendError::Internal { context: "lower name lookup", span })
    }

    /// CEP:WHAT: Consumes the next binding in shared pre-order for a let statement.
    /// CEP:WHY: The hoisted VarIds were created in exactly this order.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal on order divergence (impossible for parser output).
    /// CEP:ASSUMES: statement walks follow source pre-order.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_fixture_region_shapes.
    fn take_binding(&mut self, name: SymbolId, span: Span) -> Result<VarId, FrontendError> {
        let expected = self
            .bindings
            .get(self.next_binding)
            .ok_or(FrontendError::Internal { context: "lower binding cursor", span })?;
        if expected.name != name {
            return Err(FrontendError::Internal { context: "lower binding order", span });
        }
        let vid = *self
            .var_ids
            .get(self.next_binding)
            .ok_or(FrontendError::Internal { context: "lower binding id", span })?;
        self.next_binding += 1;
        Ok(vid)
    }

    /// CEP:WHAT: The resolved IrType of one expression, by its ExprKey.
    /// CEP:WHY: Every HirExpr carries a concrete type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal when inference lacks the key (impossible after infer).
    /// CEP:ASSUMES: infer ran first.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_fixture_region_shapes.
    fn ty_of(&self, expr: &Expr) -> Result<IrType, FrontendError> {
        self.inference
            .expr_types
            .get(expr.key.0 as usize)
            .copied()
            .ok_or(FrontendError::Internal { context: "lower expr type", span: expr.span })
    }

    /// CEP:WHAT: Lowers one expression tree with an iterative post-order walk.
    /// CEP:WHY: Operand ids must precede parent ids in the HIR vector, and operator
    ///   chains of arbitrary length must not recurse (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal fail-closed guards; never panics.
    /// CEP:ASSUMES: names resolved and types inferred.
    /// CEP:COST: O(nodes); each pushed and popped once.
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_fixture_region_shapes.
    fn lower_expr(&mut self, root: &Expr) -> Result<ExprId, FrontendError> {
        enum Frame<'e> {
            Enter(&'e Expr),
            Combine(&'e Expr),
        }
        let mut work: Vec<Frame> = vec![Frame::Enter(root)];
        let mut vals: Vec<ExprId> = Vec::new();
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
                    let ty = self.ty_of(expr)?;
                    let id = match &expr.kind {
                        ExprKind::IntLit(v) => {
                            self.hir.add_expr(HirExprKind::Const(*v), ty, expr.span)
                        }
                        ExprKind::BoolLit(b) => {
                            self.hir.add_expr(HirExprKind::Const(i64::from(*b)), ty, expr.span)
                        }
                        ExprKind::Var(sym) => {
                            let vid = self.lookup(*sym, expr.span)?;
                            self.hir.add_expr(HirExprKind::Var(vid), ty, expr.span)
                        }
                        ExprKind::Unary(op, _) => {
                            let operand = vals.pop().ok_or(FrontendError::Internal {
                                context: "lower unary arity",
                                span: expr.span,
                            })?;
                            self.hir.add_expr(HirExprKind::Unary(op.to_hir(), operand), ty, expr.span)
                        }
                        ExprKind::Binary(op, _, _) => {
                            let rhs = vals.pop().ok_or(FrontendError::Internal {
                                context: "lower binary arity",
                                span: expr.span,
                            })?;
                            let lhs = vals.pop().ok_or(FrontendError::Internal {
                                context: "lower binary arity",
                                span: expr.span,
                            })?;
                            self.hir.add_expr(HirExprKind::Binary(op.to_hir(), lhs, rhs), ty, expr.span)
                        }
                        ExprKind::Call { callee, args } => {
                            let argc = args.len();
                            let mut argvs: Vec<ExprId> = Vec::with_capacity(argc);
                            for _ in 0..argc {
                                argvs.push(vals.pop().ok_or(FrontendError::Internal {
                                    context: "lower call arity",
                                    span: expr.span,
                                })?);
                            }
                            argvs.reverse();
                            let index = self.fn_table.find(*callee).ok_or(
                                FrontendError::Internal { context: "lower callee", span: expr.span },
                            )?;
                            let callee_index = index as u32;
                            self.hir.add_expr(
                                HirExprKind::Call { callee: callee_index, args: argvs },
                                ty,
                                expr.span,
                            )
                        }
                        ExprKind::Paren(_) => {
                            let child = vals.pop().ok_or(FrontendError::Internal {
                                context: "lower paren arity",
                                span: expr.span,
                            })?;
                            vals.push(child);
                            continue;
                        }
                    };
                    vals.push(id);
                }
            }
        }
        vals.pop().ok_or(FrontendError::Internal { context: "lower expr root", span: root.span })
    }

    /// CEP:WHAT: Lowers a block's content into an existing region, switching to sibling
    ///   tail regions after each construct so execution order matches source order.
    /// CEP:WHY: A region's statements run before its child regions (HIR convention), so
    ///   statements after an if/while/loop must live in a fresh sibling tail.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: statement lowering failures with spans; Internal depth guard.
    /// CEP:ASSUMES: recursion depth bounded by MAX_PARSE_DEPTH (checked).
    /// CEP:COST: O(block size).
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_loop_region_shape.
    fn lower_block_content(
        &mut self,
        block: &Block,
        region: RegionId,
        is_fn_body: bool,
        depth: u32,
    ) -> Result<RegionId, FrontendError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(FrontendError::Internal {
                context: "lower block depth",
                span: block.span,
            });
        }
        let scope_mark = self.scope.len();
        let mut tail = region;
        for stmt in &block.stmts {
            let is_construct = matches!(stmt, Stmt::If { .. } | Stmt::While { .. } | Stmt::Loop { .. });
            self.lower_stmt(stmt, tail, depth)?;
            if is_construct {
                tail = self.hir.add_region(RegionKind::Block, Some(region), stmt_span_of(stmt));
            }
        }
        if let Some(t) = &block.trailing {
            let value = self.lower_expr(t)?;
            if is_fn_body {
                self.hir.add_stmt(tail, StmtKind::Return(Some(value)), t.span);
            } else {
                self.hir.add_stmt(tail, StmtKind::Expr(value), t.span);
            }
        }
        self.scope.truncate(scope_mark);
        Ok(tail)
    }

    /// CEP:WHAT: Lowers one statement into the given region.
    /// CEP:WHY: The statement-to-HIR mapping; constructs recurse into regions.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BreakOutsideLoop/ContinueOutsideLoop with spans; Internal guards.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(statement size).
    /// CEP:EVIDENCE: unit tests rv8_frontend::lower::tests::*.
    fn lower_stmt(&mut self, stmt: &Stmt, cur: RegionId, depth: u32) -> Result<(), FrontendError> {
        match stmt {
            Stmt::Let { name, init, span, .. } => {
                let vid = self.take_binding(*name, *span)?;
                let mutable = self
                    .bindings
                    .get(self.next_binding - 1)
                    .map(|b| b.mutable)
                    .unwrap_or(false);
                self.scope.push((*name, vid, mutable));
                if let Some(init) = init {
                    let value = self.lower_expr(init)?;
                    self.hir.add_stmt(cur, StmtKind::Assign { var: vid, value }, *span);
                }
            }
            Stmt::Assign { name, value, span } => {
                let vid = self.lookup(*name, *span)?;
                let value_id = self.lower_expr(value)?;
                self.hir.add_stmt(cur, StmtKind::Assign { var: vid, value: value_id }, *span);
            }
            Stmt::Expr(expr) => {
                let id = self.lower_expr(expr)?;
                self.hir.add_stmt(cur, StmtKind::Expr(id), expr.span);
            }
            Stmt::If { .. } | Stmt::While { .. } | Stmt::Loop { .. } => {
                self.lower_construct(stmt, cur, depth)?;
            }
            Stmt::Break { span } => {
                let target = *self
                    .loops
                    .last()
                    .ok_or(FrontendError::BreakOutsideLoop { span: *span })?;
                self.hir.add_stmt(cur, StmtKind::Break(target), *span);
            }
            Stmt::Continue { span } => {
                let target = *self
                    .loops
                    .last()
                    .ok_or(FrontendError::ContinueOutsideLoop { span: *span })?;
                self.hir.add_stmt(cur, StmtKind::Continue(target), *span);
            }
            Stmt::Return { value, span } => {
                let value_id = match value {
                    Some(e) => Some(self.lower_expr(e)?),
                    None => None,
                };
                self.hir.add_stmt(cur, StmtKind::Return(value_id), *span);
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Lowers a structured construct: if/else as a Conditional with exactly two
    ///   arm regions, while/loop as a Loop with exactly one header, one body, and one
    ///   latch (children in that order).
    /// CEP:WHY: The frozen flattener requires loop regions to carry no statements of
    ///   their own, conditionals to have exactly two children, and loops to have header
    ///   and latch as direct children; the cond is evaluated at the end of the header,
    ///   which makes the region tree a faithful while.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal guards; never emits unverifiable HIR.
    /// CEP:ASSUMES: constructs appear only as children of Block regions.
    /// CEP:COST: O(construct size).
    /// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_loop_region_shape.
    fn lower_construct(
        &mut self,
        stmt: &Stmt,
        cur: RegionId,
        depth: u32,
    ) -> Result<(), FrontendError> {
        match stmt {
            Stmt::If { cond, then_block, else_clause, span } => {
                let cif = self.hir.add_region(RegionKind::Conditional, Some(cur), *span);
                let cond_id = self.lower_expr(cond)?;
                if let Some(r) = self.hir.regions.get_mut(cif.0 as usize) {
                    r.cond = Some(cond_id);
                } else {
                    return Err(FrontendError::Internal { context: "lower cond", span: *span });
                }
                let then_r = self.hir.add_region(RegionKind::Block, Some(cif), then_block.span);
                self.lower_block_content(then_block, then_r, false, depth + 1)?;
                let else_r = self.hir.add_region(
                    RegionKind::Block,
                    Some(cif),
                    else_clause
                        .as_ref()
                        .map(else_clause_span)
                        .unwrap_or(*span),
                );
                match else_clause {
                    Some(ElseClause::Block(b)) => {
                        self.lower_block_content(b, else_r, false, depth + 1)?;
                    }
                    Some(ElseClause::If(inner)) => {
                        // An else-if is exactly an if statement whose entire else body
                        // is the nested construct; no tail is needed inside the else.
                        self.lower_construct(inner, else_r, depth + 1)?;
                    }
                    None => {}
                }
            }
            Stmt::While { cond, body, span } => {
                let loop_r = self.hir.add_region(RegionKind::Loop, Some(cur), *span);
                let cond_id = self.lower_expr(cond)?;
                let header = self.hir.add_region(RegionKind::Block, Some(loop_r), *span);
                let body_r = self.hir.add_region(RegionKind::Block, Some(loop_r), body.span);
                let latch = self.hir.add_region(RegionKind::Block, Some(loop_r), *span);
                {
                    let r = self
                        .hir
                        .regions
                        .get_mut(loop_r.0 as usize)
                        .ok_or(FrontendError::Internal { context: "lower loop", span: *span })?;
                    r.cond = Some(cond_id);
                    r.header = Some(header);
                    r.latch = Some(latch);
                }
                self.loops.push(loop_r);
                self.lower_block_content(body, body_r, false, depth + 1)?;
                self.loops.pop();
            }
            Stmt::Loop { body, span } => {
                let loop_r = self.hir.add_region(RegionKind::Loop, Some(cur), *span);
                let cond_id = self.hir.add_expr(HirExprKind::Const(1), IrType::Bool, *span);
                let header = self.hir.add_region(RegionKind::Block, Some(loop_r), *span);
                let body_r = self.hir.add_region(RegionKind::Block, Some(loop_r), body.span);
                let latch = self.hir.add_region(RegionKind::Block, Some(loop_r), *span);
                {
                    let r = self
                        .hir
                        .regions
                        .get_mut(loop_r.0 as usize)
                        .ok_or(FrontendError::Internal { context: "lower loop", span: *span })?;
                    r.cond = Some(cond_id);
                    r.header = Some(header);
                    r.latch = Some(latch);
                }
                self.loops.push(loop_r);
                self.lower_block_content(body, body_r, false, depth + 1)?;
                self.loops.pop();
            }
            _ => {
                return Err(FrontendError::Internal { context: "lower construct kind", span: stmt_span_of(stmt) });
            }
        }
        Ok(())
    }
}

/// CEP:WHAT: Span accessor for a statement (used for tail-region spans).
/// CEP:WHY: Tail regions carry the construct's provenance.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_loop_region_shape.
fn stmt_span_of(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::Loop { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Return { span, .. } => *span,
        Stmt::Expr(e) => e.span,
    }
}

/// CEP:WHAT: Span accessor for an else clause.
/// CEP:WHY: Else regions carry their clause's provenance.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lower::tests::lower_if_else_region_shape.
fn else_clause_span(clause: &ElseClause) -> Span {
    match clause {
        ElseClause::Block(b) => b.span,
        ElseClause::If(s) => stmt_span_of(s),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::infer::infer_program;
    use crate::lexer::lex;
    use crate::parser::parse_program;
    use rv8_core::span::{FileId, Span};
    use rv8_ir::hir::HirError;
    use rv8_ir::lowering::hir_to_son;

    /// CEP:WHAT: Runs the full front pipeline up to lowering for tests.
    /// CEP:WHY: Keeps lower tests short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn lower_ok(src: &str) -> Vec<HirFunction> {
        let mut ix = Interner::new();
        let trees = lex(src, FileId(0), &mut ix).expect("lex ok");
        let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
        let program = parse_program(&trees, &ix, eof).expect("parse ok");
        let table = crate::resolve::resolve(&program, &ix).expect("resolve ok");
        let inference = infer_program(&program, &table, &ix).expect("infer ok");
        lower_program(&program, &table, &inference, &ix).expect("lower ok")
    }

    /// CEP:WHAT: Lowers the fixture and checks region/statement shapes end to end.
    /// CEP:WHY: Verifies the exact mandated pipeline output shape.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_fixture_region_shapes() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() -> i32 { add(6, 7) }";
        let funcs = lower_ok(src);
        assert_eq!(funcs.len(), 2);
        for f in &funcs {
            f.verify().expect("fixture HIR verifies");
            hir_to_son(f).expect("fixture HIR flattens");
        }
        let add = &funcs[0];
        assert_eq!(add.vars.len(), 2, "params a and b");
        assert_eq!(add.name, "add");
        assert_eq!(add.params.len(), 2, "ABI argument order registered");
        // Root: no hoisted lets for params; trailing return + synthesized return.
        let root = add.root.expect("root");
        let region = add.region(root).expect("region");
        assert_eq!(region.stmts.len(), 2, "return a+b, return 0");
    }

    /// CEP:WHAT: Flattens the fixture and checks Parameter entry nodes exist per param.
    /// CEP:WHY: Params must resolve to Opcode::Parameter(i) so ABI pre-coloring applies;
    ///   a regression here silently computes on zero instead of the incoming arguments.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_fixture_params_become_parameter_nodes() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let funcs = lower_ok(src);
        let son = hir_to_son(&funcs[0]).expect("flatten");
        let params: Vec<u16> = son
            .nodes
            .iter()
            .filter_map(|n| match n.opcode {
                rv8_ir::son::Opcode::Parameter(k) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(params, vec![0, 1], "one Parameter node per ABI slot");
    }

    /// CEP:WHAT: A while loop lowers to exactly one header and one latch.
    /// CEP:WHY: HIR invariant 1.3.2, exercised through the real lowering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_loop_region_shape() {
        let src = "fn f() -> i32 { let mut i = 0; while i < 10 { i = i + 1; } i }";
        let funcs = lower_ok(src);
        let f = &funcs[0];
        f.verify().expect("loop HIR verifies");
        hir_to_son(f).expect("loop HIR flattens");
        let loops: Vec<usize> = f
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.kind, RegionKind::Loop))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(loops.len(), 1);
        let r = &f.regions[loops[0]];
        let header = r.header.expect("header");
        let latch = r.latch.expect("latch");
        assert_ne!(header, latch);
        assert!(r.children.contains(&header));
        assert!(r.children.contains(&latch));
        assert!(r.stmts.is_empty(), "loop regions carry no statements");
        assert!(matches!(
            f.verify(),
            Ok(())
        ));
        let _ = HirError::Oversized { items: 0 }; // keep HirError import meaningful
    }

    /// CEP:WHAT: An if/else lowers to a conditional with exactly two arm regions.
    /// CEP:WHY: The flattener's conditional shape contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_if_else_region_shape() {
        let src = "fn f() -> i32 { let mut x = 0; if true { x = 1; } else { x = 2; } x }";
        let funcs = lower_ok(src);
        let f = &funcs[0];
        f.verify().expect("if HIR verifies");
        hir_to_son(f).expect("if HIR flattens");
        let conds: Vec<usize> = f
            .regions
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.kind, RegionKind::Conditional))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(conds.len(), 1);
        assert_eq!(f.regions[conds[0]].children.len(), 2, "then and else arms");
        assert!(f.regions[conds[0]].cond.is_some());
    }

    /// CEP:WHAT: break/continue outside a loop are rejected at the statement span.
    /// CEP:WHY: HIR invariant 1.3.4 is enforced at the source level, before HIR exists.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_rejects_transfers_outside_loop() {
        for (src, expected) in [
            ("fn f() { break; }", 0u8),
            ("fn f() { continue; }", 1u8),
        ] {
            let mut ix = Interner::new();
            let trees = lex(src, FileId(0), &mut ix).expect("lex ok");
            let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
            let program = parse_program(&trees, &ix, eof).expect("parse ok");
            let table = crate::resolve::resolve(&program, &ix).expect("resolve ok");
            let inference = infer_program(&program, &table, &ix).expect("infer ok");
            let result = lower_program(&program, &table, &inference, &ix);
            if expected == 0 {
                assert!(matches!(result, Err(FrontendError::BreakOutsideLoop { .. })));
            } else {
                assert!(matches!(result, Err(FrontendError::ContinueOutsideLoop { .. })));
            }
        }
    }
}
