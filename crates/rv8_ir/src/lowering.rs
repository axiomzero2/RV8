// CEP:FILE: crates/rv8_ir/src/lowering.rs
// CEP:WHAT: Level transition 6.1 (HIR to Sea-of-Nodes flattening) per
//   docs/specs/multi_level_ir.md: statements become nodes, control flow edges connect
//   sequential statements, loop headers and latches become back-edges, and phi nodes are
//   inserted at control flow merge points; plus the shared LowerError contract.
// CEP:WHY: The frontend produces HIR (spec 7) and every global optimization runs on the
//   Sea of Nodes; a deterministic, recursion-free flattening with structured SSA
//   construction keeps results reproducible (38.10, 38.19) on untrusted input (38.13).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns LowerError for unbound variables, unsupported region shapes, or
//   HIR verification failure; never panics.
// CEP:ASSUMES: input HIR satisfies spec 1.3 (re-verified here before flattening); loop
//   regions carry no statements of their own (header condition, body children, and latch
//   carry all code).
// CEP:COST: O(N) in HIR items with one stack-machine pass; merge construction is O(sum of
//   per-region written-variable deltas).
// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
// CEP:SECURITY: fully iterative; every traversal is an explicit stack, so untrusted
//   region nesting depth cannot drive recursion (38.13).

use crate::hir::{
    BinOp, ExprId, ExprKind, HirError, HirFunction, RegionId, RegionKind, StmtId, StmtKind, UnOp,
    VarId,
};
use crate::son::{IrType, NodeId, Opcode, SeaOfNodes};
use rv8_core::span::Span;

/// CEP:WHAT: Blank span for nodes synthesized by lowering (region heads, merges, phis).
/// CEP:WHY: Provenance for synthetic nodes is the zero span; real statements carry their
/// own spans through lower_expr.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: FileId(0) is the placeholder file.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
fn synth_span() -> Span {
    Span::new(rv8_core::span::FileId(0), 0, 0)
}

/// CEP:WHAT: Errors of the level-transition lowerings (6.1, 6.2, 6.3).
/// CEP:WHY: Fail-closed contracts without panics; variants name the offending ids.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::flatten_rejects_*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LowerError {
    Hir(HirError),
    UnboundVariable { var: VarId },
    UnsupportedRegionShape { region: RegionId },
    UnsupportedOp { op: &'static str },
    Het(crate::hetir::HetError),
}

impl From<HirError> for LowerError {
    /// CEP:WHAT: Lifts a HirError into LowerError.
    /// CEP:WHY: One error type for the whole lowering surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_rejects_invalid_hir.
    fn from(e: HirError) -> LowerError {
        LowerError::Hir(e)
    }
}

/// CEP:WHAT: One binding of a variable to a value node on the scoped value stack.
/// CEP:WHY: SSA construction replaces variable stores with stack rebinds.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: later entries shadow earlier ones.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_if_else_makes_phi.
type Binding = (VarId, NodeId);

/// CEP:WHAT: Incoming edge of a merge: the control node whose execution leads here, its
/// block, and the variable-value deltas written on that path.
/// CEP:WHY: Multi-exit merges (breaks, continues, loop exits) need all three facts to
/// place phis correctly.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: deltas are deduped keeping the last write per variable.
/// CEP:COST: POD plus delta vector.
/// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_loop_makes_header_phi.
#[derive(Clone)]
struct MergeIncoming {
    ctrl: NodeId,
    deltas: Vec<Binding>,
}

/// CEP:WHAT: One frame phase of the flattening stack machine; every region or structured
/// construct owns exactly one frame for its lifetime.
/// CEP:WHY: Explicit frames replace recursion; the phase drives each machine step.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD plus small vectors.
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
#[derive(Clone)]
enum Phase {
    /// Block or Scope region: statements first, then child regions.
    RegionStmts,
    /// Walking child regions of a Block or Scope; index of the next child.
    RegionChildren { next: usize },
    /// Conditional construct: open the condition block, run its statements, emit the
    /// branch, then descend into the then arm.
    CondEnter,
    /// Then arm finished; facts parked, else arm next.
    CondAfterThen {
        else_region: RegionId,
        branch: NodeId,
        val_mark: usize,
        delta_mark: usize,
    },
    /// Both arms finished; build the join and complete the construct.
    CondAfterElse {
        then_incoming: Option<MergeIncoming>,
        val_mark: usize,
        delta_mark: usize,
    },
    /// Loop construct: open the header, emit phis, then run the header region frame.
    LoopEnter,
    /// Header frame finished: evaluate the condition, emit the branch, enter the body.
    LoopHeaderWait {
        latch: RegionId,
        header_region: NodeId,
        header_phis: Vec<Binding>,
        body_children: Vec<RegionId>,
        latch_incoming: Vec<MergeIncoming>,
        exit_incoming: Vec<MergeIncoming>,
        val_mark: usize,
        delta_mark: usize,
    },
    /// Walking loop body children.
    LoopBody {
        latch: RegionId,
        header_region: NodeId,
        branch: NodeId,
        header_phis: Vec<Binding>,
        body_children: Vec<RegionId>,
        next_body: usize,
        latch_incoming: Vec<MergeIncoming>,
        exit_incoming: Vec<MergeIncoming>,
        val_mark: usize,
        delta_mark: usize,
        latch_mark: usize,
    },
    /// Latch frame finished or skipped: close the loop.
    LoopLatchWait {
        header_region: NodeId,
        branch: NodeId,
        branch_delta: Vec<Binding>,
        header_phis: Vec<Binding>,
        latch_incoming: Vec<MergeIncoming>,
        exit_incoming: Vec<MergeIncoming>,
        val_mark: usize,
        delta_mark: usize,
        latch_mark: usize,
    },
}

/// CEP:WHAT: One stack frame: region or construct under processing, phase, and the
/// terminated flag.
/// CEP:WHY: Frames carry all state a recursive call would have kept in locals.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD plus phase payload.
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
struct Frame {
    region: Option<RegionId>,
    phase: Phase,
    terminated: bool,
}

/// CEP:WHAT: The flattening machine: shared stacks plus the output graph.
/// CEP:WHY: One owner of mutable state keeps phase transitions total and borrow-clean.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(N).
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
struct Flattener<'a> {
    hir: &'a HirFunction,
    out: SeaOfNodes,
    vals: Vec<Binding>,
    deltas: Vec<Binding>,
    stack: Vec<Frame>,
    cur_ctrl: Option<NodeId>,
    cur_block: u32,
    /// CEP:WHAT: Next synthesized join-block id, allocated beyond region blocks.
    /// CEP:WHY: A join must own its block: a region block can hold two control points
    /// (its own flow and the join), which no single LIR block can represent.
    next_synth: u32,
}

impl<'a> Flattener<'a> {
    /// CEP:WHAT: Emits one node into the current block with a span.
    /// CEP:WHY: Single emission path keeps block association and provenance uniform.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_straight_line.
    fn emit(&mut self, opcode: Opcode, ty: IrType, inputs: Vec<NodeId>, span: Span) -> NodeId {
        let id = self.out.add_node(opcode, ty, inputs);
        if let Some(n) = self.out.nodes.get_mut(id.0 as usize) {
            n.block = Some(crate::son::BlockId(self.cur_block));
            n.span = span;
        }
        id
    }

    /// CEP:WHAT: Resolves the current value of a variable by reverse scan.
    /// CEP:WHY: Shadowing means the last binding wins.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnboundVariable when no binding exists.
    /// CEP:ASSUMES: HIR scope checking passed, so the binding exists; lowering reports
    /// rather than guessing.
    /// CEP:COST: O(bindings in scope).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_rejects_unbound_variable.
    fn resolve(&self, var: VarId) -> Result<NodeId, LowerError> {
        self.vals
            .iter()
            .rev()
            .find(|(v, _)| *v == var)
            .map(|(_, n)| *n)
            .ok_or(LowerError::UnboundVariable { var })
    }

    /// CEP:WHAT: Binds a variable to a value on both the value stack and the delta log.
    /// CEP:WHY: Merge snapshots come from the delta log; reads come from the value stack.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_if_else_makes_phi.
    fn bind(&mut self, var: VarId, node: NodeId) {
        self.vals.push((var, node));
        self.deltas.push((var, node));
    }

    /// CEP:WHAT: Dedups a delta list keeping the last write per variable.
    /// CEP:WHY: A merge cares about final path values, not write history.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(k^2) in delta size k; k is variables written in one region, small in
    /// practice.
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_if_else_makes_phi.
    fn dedup_last(deltas: Vec<Binding>) -> Vec<Binding> {
        let mut out: Vec<Binding> = Vec::with_capacity(deltas.len());
        for (v, n) in deltas {
            match out.iter_mut().find(|(vv, _)| *vv == v) {
                Some(slot) => slot.1 = n,
                None => out.push((v, n)),
            }
        }
        out
    }

    /// CEP:WHAT: Extracts the delta log entries recorded at or after a raw mark, deduped.
    /// CEP:WHY: Marks index the raw log; dedup must run after the cut, never before.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(log size).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_loop_makes_header_phi.
    fn cut_deltas(raw: &[Binding], mark: usize) -> Vec<Binding> {
        Flattener::dedup_last(
            raw.iter()
                .enumerate()
                .filter(|(i, _)| *i >= mark)
                .map(|(_, b)| *b)
                .collect(),
        )
    }

    /// CEP:WHAT: Builds a merge over incoming paths: emits phis for variables whose final
    /// values differ across paths, then the Merge control node, and rebinds candidates.
    /// CEP:WHY: Spec 6.1 requires phi insertion at control flow merge points.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnboundVariable if a base binding is missing.
    /// CEP:ASSUMES: at least one incoming when called; caller handles the empty case.
    /// CEP:COST: O(sum of delta sizes + candidates * incoming count).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_if_else_makes_phi.
    fn build_merge(&mut self, incoming: Vec<MergeIncoming>) -> Result<Option<NodeId>, LowerError> {
        if incoming.is_empty() {
            self.cur_ctrl = None;
            return Ok(None);
        }
        if incoming.len() == 1 {
            let single = incoming.into_iter().next().expect("len checked");
            let deltas = single.deltas;
            self.cur_ctrl = Some(single.ctrl);
            for (v, n) in deltas {
                self.bind(v, n);
            }
            return Ok(Some(single.ctrl));
        }
        // Multi-path joins live in their own synthesized block.
        self.cur_block = self.next_synth;
        self.next_synth += 1;
        // Candidate set: variables written on any path, in deterministic order.
        let mut candidates: Vec<VarId> = Vec::new();
        for inc in &incoming {
            for (v, _) in &inc.deltas {
                if !candidates.contains(v) {
                    candidates.push(*v);
                }
            }
        }
        candidates.sort();
        let mut phi_plans: Vec<(VarId, IrType, Vec<NodeId>)> = Vec::new();
        for var in candidates {
            let ty = self
                .hir
                .vars
                .get(var.0 as usize)
                .map(|hv| hv.ty)
                .unwrap_or(IrType::I32);
            let base = self.resolve(var)?;
            let mut path_values: Vec<NodeId> = Vec::with_capacity(incoming.len());
            let mut all_same = true;
            for inc in &incoming {
                let value = inc
                    .deltas
                    .iter()
                    .rev()
                    .find(|(v, _)| *v == var)
                    .map(|(_, n)| *n)
                    .unwrap_or(base);
                if let Some(first) = path_values.first() {
                    if *first != value {
                        all_same = false;
                    }
                }
                path_values.push(value);
            }
            if !all_same {
                phi_plans.push((var, ty, path_values));
            } else {
                self.bind(var, base);
            }
        }
        let mut phis: Vec<Binding> = Vec::with_capacity(phi_plans.len());
        for (var, ty, path_values) in phi_plans {
            let phi = self.emit(Opcode::Phi, ty, path_values, synth_span());
            phis.push((var, phi));
        }
        let ctrl_inputs: Vec<NodeId> = incoming.into_iter().map(|i| i.ctrl).collect();
        let merge = self.emit(Opcode::Merge, IrType::Unit, ctrl_inputs, synth_span());
        for (var, phi) in phis {
            self.bind(var, phi);
        }
        self.cur_ctrl = Some(merge);
        Ok(Some(merge))
    }

    /// CEP:WHAT: Lowers one expression tree to data nodes with an iterative post-order.
    /// CEP:WHY: Spec 6.1 statement-to-node mapping; shared by statements and conditions.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnboundVariable or UnsupportedOp.
    /// CEP:ASSUMES: expression tree acyclic (HIR verify passed).
    /// CEP:COST: O(E); each expression node descends, combines, and emits exactly once.
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_straight_line.
    fn lower_expr(&mut self, root: ExprId) -> Result<NodeId, LowerError> {
        enum EFrame {
            Descend(ExprId),
            Combine(ExprId, usize),
        }
        let mut work: Vec<EFrame> = vec![EFrame::Descend(root)];
        let mut value_stack: Vec<NodeId> = Vec::new();
        while let Some(frame) = work.pop() {
            match frame {
                EFrame::Descend(e) => {
                    let expr = self
                        .hir
                        .exprs
                        .get(e.0 as usize)
                        .ok_or(LowerError::UnsupportedOp { op: "dangling expr" })?;
                    match &expr.kind {
                        ExprKind::Const(_) | ExprKind::Var(_) => work.push(EFrame::Combine(e, 0)),
                        ExprKind::Unary(_, a) => {
                            work.push(EFrame::Combine(e, 1));
                            work.push(EFrame::Descend(*a));
                        }
                        ExprKind::Binary(_, a, b) => {
                            // The work stack is LIFO and Combine consumes the value
                            // stack in push order, so the RIGHT operand must be pushed
                            // first: the left operand then evaluates first and lands at
                            // the bottom of the arity window. Pushing a before b delivered
                            // operands [rhs, lhs] - every non-commutative binary operator
                            // (sub, div, comparisons, shifts) evaluated backwards.
                            work.push(EFrame::Combine(e, 2));
                            work.push(EFrame::Descend(*b));
                            work.push(EFrame::Descend(*a));
                        }
                        ExprKind::Call { args, .. } => {
                            // Same LIFO rule as Binary: reverse the argument pushes so
                            // Combine consumes them in declaration order.
                            work.push(EFrame::Combine(e, args.len()));
                            for &arg in args.iter().rev() {
                                work.push(EFrame::Descend(arg));
                            }
                        }
                    }
                }
                EFrame::Combine(e, arity) => {
                    let expr = self
                        .hir
                        .exprs
                        .get(e.0 as usize)
                        .ok_or(LowerError::UnsupportedOp { op: "dangling expr" })?
                        .clone();
                    let idx = value_stack
                        .len()
                        .checked_sub(arity)
                        .ok_or(LowerError::UnsupportedOp { op: "expr arity" })?;
                    let operands = value_stack.split_off(idx);
                    let id = match &expr.kind {
                        ExprKind::Const(v) => self.emit(Opcode::Const(*v), expr.ty, vec![], expr.span),
                        ExprKind::Var(var) => self.resolve(*var)?,
                        ExprKind::Unary(op, _) => {
                            let opcode = match op {
                                UnOp::Neg => Opcode::Neg,
                                UnOp::Not => Opcode::Not,
                            };
                            self.emit(opcode, expr.ty, operands, expr.span)
                        }
                        ExprKind::Binary(op, _, _) => {
                            let opcode = match op {
                                BinOp::Add => Opcode::Add,
                                BinOp::Sub => Opcode::Sub,
                                BinOp::Mul => Opcode::Mul,
                                BinOp::Div => Opcode::Div,
                                BinOp::Rem => Opcode::Rem,
                                BinOp::BitAnd => Opcode::And,
                                BinOp::BitOr => Opcode::Or,
                                BinOp::BitXor => Opcode::Xor,
                                BinOp::Shl => Opcode::Shl,
                                BinOp::Shr => Opcode::Shr,
                                BinOp::Eq => Opcode::CmpEq,
                                BinOp::Lt => Opcode::CmpLt,
                                BinOp::Le => Opcode::CmpLe,
                            };
                            self.emit(opcode, expr.ty, operands, expr.span)
                        }
                        ExprKind::Call { callee, .. } => {
                            self.emit(Opcode::Call { callee: *callee }, expr.ty, operands, expr.span)
                        }
                    };
                    value_stack.push(id);
                }
            }
        }
        value_stack
            .pop()
            .ok_or(LowerError::UnsupportedOp { op: "empty expr" })
    }

    /// CEP:WHAT: Collects variables written anywhere in a region subtree, sorted.
    /// CEP:WHY: Loop header phi placement needs the written set before body lowering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(subtree size); iterative.
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_loop_makes_header_phi.
    fn written_vars(&self, root: RegionId) -> Vec<VarId> {
        let mut out: Vec<VarId> = Vec::new();
        let mut work = vec![root];
        let mut seen: Vec<u32> = Vec::new();
        while let Some(r) = work.pop() {
            if seen.contains(&r.0) {
                continue;
            }
            seen.push(r.0);
            if let Some(region) = self.hir.regions.get(r.0 as usize) {
                for &sid in &region.stmts {
                    if let Some(stmt) = self.hir.stmts.get(sid.0 as usize) {
                        let var = match &stmt.kind {
                            StmtKind::Let { var, .. } | StmtKind::Assign { var, .. } => Some(*var),
                            _ => None,
                        };
                        match var {
                            Some(v) if !out.contains(&v) => out.push(v),
                            _ => {}
                        }
                    }
                }
                work.extend_from_slice(&region.children);
            }
        }
        out.sort();
        out
    }

    /// CEP:WHAT: Opens a plain region block: emits the region node chained to the current
    /// control and switches the current block.
    /// CEP:WHY: Shared prologue for Block/Scope frames and construct sub-blocks.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_straight_line.
    fn open_block(&mut self, region: RegionId) -> NodeId {
        let span = self
            .hir
            .regions
            .get(region.0 as usize)
            .map(|r| r.span)
            .unwrap_or_else(synth_span);
        // The block must be current BEFORE emission so the region node lands in it.
        self.cur_block = region.0;
        let inputs: Vec<NodeId> = self.cur_ctrl.take().into_iter().collect();
        let node = self.emit(Opcode::Region, IrType::Unit, inputs, span);
        self.cur_ctrl = Some(node);
        node
    }

    /// CEP:WHAT: Pushes the frame that enters a region or construct, opening plain
    /// blocks eagerly.
    /// CEP:WHY: Uniform child-entry for plain regions, conditionals, and loops; the
    /// current control chains into whatever the child opens.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
    fn enter_region_or_construct(&mut self, region: RegionId) {
        let kind = self
            .hir
            .regions
            .get(region.0 as usize)
            .map(|r| r.kind)
            .unwrap_or(RegionKind::Block);
        match kind {
            RegionKind::Block | RegionKind::Scope => {
                self.open_block(region);
                self.stack.push(Frame {
                    region: Some(region),
                    phase: Phase::RegionStmts,
                    terminated: false,
                });
            }
            RegionKind::Conditional => self.stack.push(Frame {
                region: Some(region),
                phase: Phase::CondEnter,
                terminated: false,
            }),
            RegionKind::Loop => self.stack.push(Frame {
                region: Some(region),
                phase: Phase::LoopEnter,
                terminated: false,
            }),
        }
    }

    /// CEP:WHAT: Lowers one statement; returns true when control left the region.
    /// CEP:WHY: Statement phases loop over this until a transfer or end.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: LowerError variants.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(expression size).
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
    fn lower_stmt(&mut self, sid: StmtId, region: RegionId) -> Result<bool, LowerError> {
        let kind = self
            .hir
            .stmts
            .get(sid.0 as usize)
            .map(|s| s.kind.clone())
            .ok_or(LowerError::UnsupportedOp { op: "dangling stmt" })?;
        match kind {
            StmtKind::Let { var, init } => {
                let value = match init {
                    Some(e) => self.lower_expr(e)?,
                    None => self.emit(Opcode::Const(0), IrType::I32, vec![], synth_span()),
                };
                self.bind(var, value);
                Ok(false)
            }
            StmtKind::Assign { var, value } => {
                let v = self.lower_expr(value)?;
                self.bind(var, v);
                Ok(false)
            }
            StmtKind::Expr(e) => {
                self.lower_expr(e)?;
                Ok(false)
            }
            StmtKind::Break(target) => {
                let pos = self
                    .stack
                    .iter()
                    .rposition(|f| f.region == Some(target))
                    .ok_or(LowerError::UnsupportedRegionShape { region })?;
                let delta_base = match &self.stack[pos].phase {
                    Phase::LoopHeaderWait { delta_mark, .. }
                    | Phase::LoopBody { delta_mark, .. }
                    | Phase::LoopLatchWait { delta_mark, .. } => *delta_mark,
                    _ => return Err(LowerError::UnsupportedRegionShape { region: target }),
                };
                let cut = Flattener::cut_deltas(&self.deltas, delta_base);
                if let Some(c) = self.cur_ctrl {
                    let incoming = MergeIncoming {
                        ctrl: c,
                        deltas: cut,
                    };
                    match &mut self.stack[pos].phase {
                        Phase::LoopHeaderWait { exit_incoming, .. }
                        | Phase::LoopBody { exit_incoming, .. }
                        | Phase::LoopLatchWait { exit_incoming, .. } => {
                            exit_incoming.push(incoming)
                        }
                        _ => {}
                    }
                }
                self.cur_ctrl = None;
                Ok(true)
            }
            StmtKind::Continue(target) => {
                let pos = self
                    .stack
                    .iter()
                    .rposition(|f| f.region == Some(target))
                    .ok_or(LowerError::UnsupportedRegionShape { region })?;
                let delta_base = match &self.stack[pos].phase {
                    Phase::LoopHeaderWait { delta_mark, .. }
                    | Phase::LoopBody { delta_mark, .. }
                    | Phase::LoopLatchWait { delta_mark, .. } => *delta_mark,
                    _ => return Err(LowerError::UnsupportedRegionShape { region: target }),
                };
                let cut = Flattener::cut_deltas(&self.deltas, delta_base);
                if let Some(c) = self.cur_ctrl {
                    let incoming = MergeIncoming {
                        ctrl: c,
                        deltas: cut,
                    };
                    match &mut self.stack[pos].phase {
                        Phase::LoopHeaderWait { latch_incoming, .. }
                        | Phase::LoopBody { latch_incoming, .. }
                        | Phase::LoopLatchWait { latch_incoming, .. } => {
                            latch_incoming.push(incoming)
                        }
                        _ => {}
                    }
                }
                self.cur_ctrl = None;
                Ok(true)
            }
            StmtKind::Return(value) => {
                let ctrl = self.cur_ctrl;
                let inputs = match value {
                    Some(e) => {
                        let v = self.lower_expr(e)?;
                        match ctrl {
                            Some(c) => vec![v, c],
                            None => vec![v],
                        }
                    }
                    None => ctrl.into_iter().collect(),
                };
                self.emit(Opcode::Return, IrType::Unit, inputs, synth_span());
                self.cur_ctrl = None;
                Ok(true)
            }
        }
    }

    /// CEP:WHAT: Runs the region statements of a plain region frame; marks the frame
    /// terminated on control transfers.
    /// CEP:WHY: Shared by RegionStmts, CondEnter statement preambles, and the header.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: LowerError variants.
    /// CEP:ASSUMES: frame top is a plain region frame.
    /// CEP:COST: O(statements + expressions).
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
    fn run_plain_stmts(&mut self, top: usize) -> Result<(), LowerError> {
        let region = self.stack[top].region.expect("region frame");
        self.stack[top].phase = Phase::RegionChildren { next: 0 };
        let stmts = self.hir.regions[region.0 as usize].stmts.clone();
        for sid in stmts {
            if self.lower_stmt(sid, region)? {
                self.stack[top].terminated = true;
                break;
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Runs the stack machine until every frame completes.
    /// CEP:WHY: The heart of spec 6.1; structured constructs are phase transitions and
    /// frame completions cascade upward through deliver.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: LowerError variants; never panics.
    /// CEP:ASSUMES: HIR verified; loop regions have no own statements.
    /// CEP:COST: O(N): every statement, expression node, and construct is processed a
    /// constant number of times.
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
    fn run(&mut self, root: RegionId) -> Result<(), LowerError> {
        // The root region node anchors the entry; every child chains from it.
        self.open_block(root);
        self.stack.push(Frame {
            region: Some(root),
            phase: Phase::RegionStmts,
            terminated: false,
        });
        while !self.stack.is_empty() {
            let top = self.stack.len() - 1;
            if self.stack[top].terminated {
                self.stack.pop();
                self.deliver(None)?;
                continue;
            }
            let phase = std::mem::replace(&mut self.stack[top].phase, Phase::RegionStmts);
            match phase {
                Phase::RegionStmts => self.run_plain_stmts(top)?,
                Phase::RegionChildren { next } => {
                    let region = self.stack[top].region.expect("region frame");
                    let children = self.hir.regions[region.0 as usize].children.clone();
                    if next < children.len() {
                        self.stack[top].phase = Phase::RegionChildren { next: next + 1 };
                        self.enter_region_or_construct(children[next]);
                    } else {
                        let ctrl = self.cur_ctrl;
                        self.stack.pop();
                        self.deliver(ctrl)?;
                    }
                }
                Phase::CondEnter => {
                    let region = self.stack[top].region.expect("cond frame");
                    let (cond_expr, then_r, else_r) = {
                        let r = &self.hir.regions[region.0 as usize];
                        let cond = r.cond.expect("verified conditional has cond");
                        let mut kids = r.children.clone();
                        if kids.len() != 2 {
                            return Err(LowerError::UnsupportedRegionShape { region });
                        }
                        let e = kids.split_off(1).pop().expect("else");
                        (cond, kids.pop().expect("then"), e)
                    };
                    let val_mark = self.vals.len();
                    let delta_mark = self.deltas.len();
                    // Condition block: region node, preamble statements, branch.
                    let region_node = self.open_block(region);
                    self.stack[top].phase = Phase::CondAfterThen {
                        else_region: else_r,
                        branch: NodeId(0), // patched below
                        val_mark,
                        delta_mark,
                    };
                    let stmts = self.hir.regions[region.0 as usize].stmts.clone();
                    let mut transferred = false;
                    for sid in stmts {
                        if self.lower_stmt(sid, region)? {
                            transferred = true;
                            break;
                        }
                    }
                    if transferred {
                        self.stack[top].terminated = true;
                        continue;
                    }
                    let cond_value = self.lower_expr(cond_expr)?;
                    let branch =
                        self.emit(Opcode::Branch, IrType::Unit, vec![cond_value, region_node], synth_span());
                    self.stack[top].phase = Phase::CondAfterThen {
                        else_region: else_r,
                        branch,
                        val_mark,
                        delta_mark,
                    };
                    // Then arm under the branch-true control.
                    self.cur_ctrl = Some(branch);
                    self.enter_region_or_construct(then_r);
                }
                Phase::CondAfterThen {
                    else_region,
                    branch,
                    val_mark,
                    delta_mark,
                } => {
                    // Delivering frame was the then arm; its ctrl is parked in cur_ctrl.
                    let then_delta = Flattener::cut_deltas(&self.deltas, delta_mark);
                    let then_incoming = self.cur_ctrl.map(|c| MergeIncoming {
                        ctrl: c,
                        deltas: then_delta,
                    });
                    self.stack[top].phase = Phase::CondAfterElse {
                        then_incoming,
                        val_mark,
                        delta_mark,
                    };
                    // Restore pre-branch bindings for the else side.
                    self.vals.truncate(val_mark);
                    self.deltas.truncate(delta_mark);
                    self.cur_ctrl = Some(branch);
                    self.enter_region_or_construct(else_region);
                }
                Phase::CondAfterElse {
                    then_incoming,
                    val_mark,
                    delta_mark,
                } => {
                    let else_delta = Flattener::cut_deltas(&self.deltas, delta_mark);
                    let else_incoming = self.cur_ctrl.map(|c| MergeIncoming {
                        ctrl: c,
                        deltas: else_delta,
                    });
                    self.stack.pop();
                    self.vals.truncate(val_mark);
                    self.deltas.truncate(delta_mark);
                    // The join lives in the parent's current block.
                    if let Some(parent) = self.stack.last() {
                        if let Some(r) = parent.region {
                            self.cur_block = r.0;
                        }
                    }
                    let mut incoming = Vec::new();
                    if let Some(t) = then_incoming {
                        incoming.push(t);
                    }
                    if let Some(e) = else_incoming {
                        incoming.push(e);
                    }
                    let ctrl = self.build_merge(incoming)?;
                    self.deliver(ctrl)?;
                }
                Phase::LoopEnter => {
                    let region = self.stack[top].region.expect("loop frame");
                    let (_cond_expr, header_r, latch_r, body_children) = {
                        let r = &self.hir.regions[region.0 as usize];
                        if !r.stmts.is_empty() {
                            return Err(LowerError::UnsupportedRegionShape { region });
                        }
                        let cond = r.cond.expect("verified loop has cond");
                        let header = r.header.expect("verified loop has header");
                        let latch = r.latch.expect("verified loop has latch");
                        let body: Vec<RegionId> = r
                            .children
                            .iter()
                            .copied()
                            .filter(|&c| c != header && c != latch)
                            .collect();
                        (cond, header, latch, body)
                    };
                    let val_mark = self.vals.len();
                    let delta_mark = self.deltas.len();
                    // Header block: region node chained to the entry control.
                    self.cur_block = header_r.0;
                    let header_span = self.hir.regions[header_r.0 as usize].span;
                    let entry_ctrl: Vec<NodeId> = self.cur_ctrl.take().into_iter().collect();
                    let header_region =
                        self.emit(Opcode::Region, IrType::Unit, entry_ctrl, header_span);
                    self.cur_ctrl = Some(header_region);
                    // Header phis for every variable written in the loop subtree.
                    let written = self.written_vars(region);
                    let mut header_phis: Vec<Binding> = Vec::with_capacity(written.len());
                    for var in written {
                        let base = self.resolve(var)?;
                        let ty = self
                            .hir
                            .vars
                            .get(var.0 as usize)
                            .map(|hv| hv.ty)
                            .unwrap_or(IrType::I32);
                        let phi = self.emit(Opcode::Phi, ty, vec![base, base], synth_span());
                        header_phis.push((var, phi));
                        self.bind(var, phi);
                    }
                    self.stack[top].phase = Phase::LoopHeaderWait {
                        latch: latch_r,
                        header_region,
                        header_phis,
                        body_children,
                        latch_incoming: Vec::new(),
                        exit_incoming: Vec::new(),
                        val_mark,
                        delta_mark,
                    };
                    // Header region frame runs its statements and children.
                    self.stack.push(Frame {
                        region: Some(header_r),
                        phase: Phase::RegionStmts,
                        terminated: false,
                    });
                }
                Phase::LoopHeaderWait {
                    latch,
                    header_region,
                    header_phis,
                    body_children,
                    latch_incoming,
                    exit_incoming,
                    val_mark,
                    delta_mark,
                } => {
                    // Delivering frame was the header region.
                    // Condition evaluation happens at the end of the header.
                    let cond_expr = {
                        // The loop region's cond was validated at LoopEnter; recover it.
                        // The frame does not carry it, so find the loop region via the
                        // header's parent.
                        let loop_region = self
                            .hir
                            .regions
                            .get(header_region_node_parent(self, header_region)?)
                            .and_then(|r| r.cond);
                        loop_region.ok_or(LowerError::UnsupportedRegionShape {
                            region: RegionId(0),
                        })?
                    };
                    let cond_value = self.lower_expr(cond_expr)?;
                    let branch = self.emit(
                        Opcode::Branch,
                        IrType::Unit,
                        vec![cond_value, header_region],
                        synth_span(),
                    );
                    let branch_delta = Flattener::cut_deltas(&self.deltas, delta_mark);
                    // Latch-entry binding context: header phis only.
                    let latch_mark = self.deltas.len();
                    if let Some(&first) = body_children.first() {
                        self.stack[top].phase = Phase::LoopBody {
                            latch,
                            header_region,
                            branch,
                            header_phis,
                            body_children: body_children.clone(),
                            next_body: 1,
                            latch_incoming,
                            exit_incoming,
                            val_mark,
                            delta_mark,
                            latch_mark,
                        };
                        // Enter the first body child under the branch-true control.
                        self.cur_ctrl = Some(branch);
                        self.enter_region_or_construct(first);
                    } else {
                        // Empty body: latch runs directly under the branch.
                        self.stack[top].phase = Phase::LoopLatchWait {
                            header_region,
                            branch,
                            branch_delta,
                            header_phis,
                            latch_incoming,
                            exit_incoming,
                            val_mark,
                            delta_mark,
                            latch_mark,
                        };
                        self.cur_ctrl = Some(branch);
                        self.open_block(latch);
                        self.stack.push(Frame {
                            region: Some(latch),
                            phase: Phase::RegionStmts,
                            terminated: false,
                        });
                    }
                }
                Phase::LoopBody {
                    latch,
                    header_region,
                    branch,
                    header_phis,
                    body_children,
                    next_body,
                    latch_incoming,
                    exit_incoming,
                    val_mark,
                    delta_mark,
                    // The carried mark is stale after body finalize; a fresh mark is
                    // captured below (see the re-capture comment in the else branch).
                    latch_mark: _stale_latch_mark,
                } => {
                    if next_body < body_children.len() && self.cur_ctrl.is_some() {
                        let child = body_children[next_body];
                        self.stack[top].phase = Phase::LoopBody {
                            latch,
                            header_region,
                            branch,
                            header_phis,
                            body_children,
                            next_body: next_body + 1,
                            latch_incoming,
                            exit_incoming,
                            val_mark,
                            delta_mark,
                            // Mid-body the carried mark is still exact: no truncation
                            // has happened yet.
                            latch_mark: _stale_latch_mark,
                        };
                        self.enter_region_or_construct(child);
                    } else {
                        // Body finished or dead. Live case: the latch is the SEQUENTIAL
                        // continuation of the body, so the body's writes and control
                        // chain into it (no parallel merge on the back-edge: a merge
                        // here would fabricate a second, phantom path into the loop
                        // header and destroy the loop-carried values). Dead case: park
                        // any earlier incoming so LoopLatchWait can still close.
                        if self.cur_ctrl.is_some() {
                            let body_delta = Flattener::cut_deltas(&self.deltas, delta_mark);
                            self.vals.truncate(val_mark);
                            self.deltas.truncate(delta_mark);
                            for (var, phi) in &header_phis {
                                self.vals.push((*var, *phi));
                                self.deltas.push((*var, *phi));
                            }
                            // The branch-false exit leaves the loop at the HEADER, so
                            // the exit values are exactly the header-phi bindings —
                            // cut BEFORE the body rebinds join the delta log.
                            let branch_delta = Flattener::cut_deltas(&self.deltas, delta_mark);
                            // Rebind the body's final values so the latch reads them.
                            for (v, n) in &body_delta {
                                self.vals.push((*v, *n));
                                self.deltas.push((*v, *n));
                            }
                            // Above this mark: exactly the latch's own writes.
                            let latch_mark = self.deltas.len();
                            self.stack[top].phase = Phase::LoopLatchWait {
                                header_region,
                                branch,
                                branch_delta,
                                header_phis,
                                latch_incoming,
                                exit_incoming,
                                val_mark,
                                delta_mark,
                                latch_mark,
                            };
                            // open_block consumes cur_ctrl as the latch region's
                            // control input, which is the body-to-latch edge.
                            self.open_block(latch);
                            self.stack.push(Frame {
                                region: Some(latch),
                                phase: Phase::RegionStmts,
                                terminated: false,
                            });
                        } else {
                            self.vals.truncate(val_mark);
                            self.deltas.truncate(delta_mark);
                            for (var, phi) in &header_phis {
                                self.vals.push((*var, *phi));
                                self.deltas.push((*var, *phi));
                            }
                            let latch_mark = self.deltas.len();
                            let branch_delta =
                                Flattener::cut_deltas(&self.deltas, delta_mark);
                            self.stack[top].phase = Phase::LoopLatchWait {
                                header_region,
                                branch,
                                branch_delta,
                                header_phis,
                                latch_incoming,
                                exit_incoming,
                                val_mark,
                                delta_mark,
                                latch_mark,
                            };
                            // No path reaches the latch; LoopLatchWait finalizes with
                            // latch_ctrl None on the next machine step.
                        }
                    }
                }
                Phase::LoopLatchWait {
                    header_region,
                    branch,
                    branch_delta,
                    header_phis,
                    mut latch_incoming,
                    exit_incoming,
                    val_mark,
                    delta_mark,
                    latch_mark,
                } => {
                    // Delivering frame was the latch region (or the phase was reached
                    // with a dead control when the body could not reach the latch).
                    // Back-edge values: resolve each header-phi variable NOW, before any
                    // truncation — the binding chain is base, header phis, body writes,
                    // latch writes, so the latest binding per variable is exactly the
                    // value the back-edge must carry.
                    let patch_values: Vec<(VarId, NodeId)> = header_phis
                        .iter()
                        .filter_map(|(v, _)| self.resolve(*v).ok().map(|n| (*v, n)))
                        .collect();
                    let latch_ctrl = self.cur_ctrl;
                    let latch_delta = Flattener::cut_deltas(&self.deltas, latch_mark);
                    if let Some(c) = latch_ctrl {
                        latch_incoming.push(MergeIncoming {
                            ctrl: c,
                            deltas: latch_delta.clone(),
                        });
                    }
                    self.stack.pop();
                    self.vals.truncate(val_mark);
                    self.deltas.truncate(delta_mark);
                    for (var, phi) in &header_phis {
                        self.vals.push((*var, *phi));
                    }
                    // Latch merge when more than one path reaches the latch (only in
                    // degenerate shapes; the sequential latch keeps a single incoming).
                    let mut back_ctrl = latch_ctrl;
                    if latch_incoming.len() > 1 {
                        back_ctrl = self.build_merge(latch_incoming)?;
                    } else if let Some(single) = latch_incoming.into_iter().next() {
                        back_ctrl = Some(single.ctrl);
                    }
                    // Back-edge: latch control returns to the header region node.
                    if let Some(lc) = back_ctrl {
                        if let Ok(n) = self.out.node_mut(header_region) {
                            n.inputs.push(lc);
                        }
                    }
                    // Patch header phis with the resolved back-edge values.
                    for (var, new_val) in &patch_values {
                        if let Some((_, phi)) =
                            header_phis.iter().find(|(v, _)| v == var)
                        {
                            if let Ok(n) = self.out.node_mut(*phi) {
                                if n.inputs.len() >= 2 {
                                    n.inputs[1] = *new_val;
                                }
                            }
                        }
                    }
                    // Exit merge: branch-false path plus every break.
                    let mut incoming = exit_incoming;
                    incoming.push(MergeIncoming {
                        ctrl: branch,
                        deltas: branch_delta,
                    });
                    if let Some(parent) = self.stack.last() {
                        if let Some(r) = parent.region {
                            self.cur_block = r.0;
                        }
                    }
                    let ctrl = self.build_merge(incoming)?;
                    self.deliver(ctrl)?;
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Delivers a completed child's outgoing control to the parent frame.
    /// CEP:WHY: Frame completion cascades; constructs park the control for their next
    /// machine step, plain parents store it, and dead control terminates plain frames
    /// while constructs continue on their alternative paths.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: never fails; merge construction happens in construct phases.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) per completion.
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
    fn deliver(&mut self, ctrl: Option<NodeId>) -> Result<(), LowerError> {
        match self.stack.last_mut() {
            None => {
                self.cur_ctrl = ctrl;
                Ok(())
            }
            Some(parent) => {
                if ctrl.is_none() {
                    match parent.phase {
                        Phase::CondAfterThen { .. }
                        | Phase::CondAfterElse { .. }
                        | Phase::LoopHeaderWait { .. }
                        | Phase::LoopBody { .. }
                        | Phase::LoopLatchWait { .. } => {
                            // Arm, body, or latch died; the construct continues on its
                            // alternative path. The parked None is read by the phase.
                            self.cur_ctrl = None;
                        }
                        _ => {
                            parent.terminated = true;
                            self.cur_ctrl = None;
                        }
                    }
                } else {
                    self.cur_ctrl = ctrl;
                }
                Ok(())
            }
        }
    }
}

/// CEP:WHAT: Finds the parent region index of the region block that owns a node.
/// CEP:WHY: The loop condition expression lives on the loop region; the header wait
/// phase recovers it from the header's parent link.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnsupportedOp when the node has no block (cannot happen for emitted
/// nodes).
/// CEP:ASSUMES: one region per block (BlockId equals RegionId in this flattening).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::lowering::tests::flatten_loop_makes_header_phi.
fn header_region_node_parent(f: &Flattener, node: NodeId) -> Result<usize, LowerError> {
    let block = f
        .out
        .node(node)
        .ok()
        .and_then(|n| n.block)
        .map(|b| b.0)
        .ok_or(LowerError::UnsupportedOp { op: "unblocked node" })?;
    let parent = f
        .hir
        .regions
        .get(block as usize)
        .and_then(|r| r.parent)
        .ok_or(LowerError::UnsupportedOp { op: "rootless header" })?;
    Ok(parent.0 as usize)
}

/// CEP:WHAT: Flattens a verified HIR function into a Sea-of-Nodes graph (spec 6.1).
/// CEP:WHY: The single entry point used by the frontend pipeline and tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: LowerError; the input HIR is verified first and structural problems are
/// reported, never panicked.
/// CEP:ASSUMES: one function per HIR container.
/// CEP:COST: O(N) plus merge construction; the output passes verify_structure before
/// returning.
/// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::*.
pub fn hir_to_son(hir: &HirFunction) -> Result<SeaOfNodes, LowerError> {
    hir.verify().map_err(LowerError::Hir)?;
    let root = hir
        .root
        .ok_or(LowerError::UnsupportedRegionShape { region: RegionId(u32::MAX) })?;
    let mut f = Flattener {
        hir,
        out: SeaOfNodes::new(),
        vals: Vec::new(),
        deltas: Vec::new(),
        stack: Vec::new(),
        cur_ctrl: None,
        cur_block: root.0,
        next_synth: hir.regions.len() as u32,
    };
    // Parameter entry bindings (ABI argument order): each declared parameter resolves,
    // before any statement runs, to an Opcode::Parameter(i) node. Downstream, son_to_hetir
    // maps these to ComputeOp::Param and the backend pre-colors them to argument registers.
    // Binding through `bind` (not just `vals`) keeps loop/merge delta snapshots consistent:
    // params are base bindings, so joins never phi them unless the body reassigns.
    for (i, &var) in hir.params.iter().enumerate() {
        let index = i as u16;
        if i > u16::MAX as usize {
            return Err(LowerError::UnsupportedOp { op: "param count" });
        }
        let ty = hir
            .vars
            .get(var.0 as usize)
            .map(|v| v.ty)
            .ok_or(LowerError::UnboundVariable { var })?;
        let node = f.emit(Opcode::Parameter(index), ty, vec![], synth_span());
        f.bind(var, node);
    }
    f.run(root)?;
    f.out
        .verify_structure()
        .map_err(|_| LowerError::UnsupportedOp { op: "son structure" })?;
    Ok(f.out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::hir::*;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Blank span for tests.
    /// CEP:WHY: Keeps test bodies readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span::new(FileId(0), 0, 0)
    }

    /// CEP:WHAT: Builds fn { 1 + 2 } and flattens it.
    /// CEP:WHY: Straight-line statements become Const/Add nodes in block 0.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_straight_line() {
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let a = h.add_expr(ExprKind::Const(1), IrType::I32, sp());
        let b = h.add_expr(ExprKind::Const(2), IrType::I32, sp());
        let sum = h.add_expr(ExprKind::Binary(BinOp::Add, a, b), IrType::I32, sp());
        h.add_stmt(root, StmtKind::Expr(sum), sp());
        let son = hir_to_son(&h).expect("flatten straight line");
        assert!(son.verify_structure().is_ok());
        let adds = son.nodes.iter().filter(|n| matches!(n.opcode, Opcode::Add)).count();
        assert_eq!(adds, 1);
        let consts = son
            .nodes
            .iter()
            .filter(|n| matches!(n.opcode, Opcode::Const(_)))
            .count();
        assert_eq!(consts, 2);
    }

    /// CEP:WHAT: Builds if/else where both arms assign x, then flattens.
    /// CEP:WHY: Spec 6.1 phi insertion at the merge; the phi has both path values.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_if_else_makes_phi() {
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let x = h.add_var("x", IrType::I32, root);
        h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
        let cond = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let cif = h.add_region(RegionKind::Conditional, Some(root), sp());
        h.regions[cif.0 as usize].cond = Some(cond);
        let then_r = h.add_region(RegionKind::Block, Some(cif), sp());
        let else_r = h.add_region(RegionKind::Block, Some(cif), sp());
        let ten = h.add_expr(ExprKind::Const(10), IrType::I32, sp());
        h.add_stmt(then_r, StmtKind::Assign { var: x, value: ten }, sp());
        let twenty = h.add_expr(ExprKind::Const(20), IrType::I32, sp());
        h.add_stmt(else_r, StmtKind::Assign { var: x, value: twenty }, sp());
        let read = h.add_expr(ExprKind::Var(x), IrType::I32, sp());
        h.add_stmt(root, StmtKind::Expr(read), sp());
        let son = hir_to_son(&h).expect("flatten if/else");
        let phis: Vec<&crate::son::SeaNode> = son
            .nodes
            .iter()
            .filter(|n| matches!(n.opcode, Opcode::Phi))
            .collect();
        assert_eq!(phis.len(), 1, "one phi for x at the join");
        assert_eq!(phis[0].inputs.len(), 2);
    }

    /// CEP:WHAT: Builds a counting loop and flattens it.
    /// CEP:WHY: Spec 6.1 loop header phis and back-edges; the header phi's second input
    /// must reference the latch-side value.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_loop_makes_header_phi() {
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let i = h.add_var("i", IrType::I32, root);
        h.add_stmt(root, StmtKind::Let { var: i, init: None }, sp());
        let cond = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let loop_r = h.add_region(RegionKind::Loop, Some(root), sp());
        h.regions[loop_r.0 as usize].cond = Some(cond);
        let header = h.add_region(RegionKind::Block, Some(loop_r), sp());
        let latch = h.add_region(RegionKind::Block, Some(loop_r), sp());
        h.regions[loop_r.0 as usize].header = Some(header);
        h.regions[loop_r.0 as usize].latch = Some(latch);
        let read_i = h.add_expr(ExprKind::Var(i), IrType::I32, sp());
        h.add_stmt(header, StmtKind::Expr(read_i), sp());
        let one = h.add_expr(ExprKind::Const(1), IrType::I32, sp());
        let read2 = h.add_expr(ExprKind::Var(i), IrType::I32, sp());
        let inc = h.add_expr(ExprKind::Binary(BinOp::Add, read2, one), IrType::I32, sp());
        h.add_stmt(latch, StmtKind::Assign { var: i, value: inc }, sp());
        h.add_stmt(latch, StmtKind::Continue(loop_r), sp());
        let son = hir_to_son(&h).expect("flatten loop");
        let phis: Vec<&crate::son::SeaNode> = son
            .nodes
            .iter()
            .filter(|n| matches!(n.opcode, Opcode::Phi))
            .collect();
        assert_eq!(phis.len(), 1, "one header phi for i");
        let headers: Vec<&crate::son::SeaNode> = son
            .nodes
            .iter()
            .filter(|n| matches!(n.opcode, Opcode::Region))
            .collect();
        assert!(
            headers.iter().any(|n| n.inputs.len() == 2),
            "header region must gain the back-edge input"
        );
        let phi = phis[0];
        let back = phi.inputs[1];
        assert!(matches!(son.nodes[back.0 as usize].opcode, Opcode::Add));
    }

    /// CEP:WHAT: A loop whose latch is reached via continue from a nested conditional
    /// produces a latch merge with a phi.
    /// CEP:WHY: Multi-path latch entries are the hard case of merge construction.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_continue_makes_latch_merge() {
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let x = h.add_var("x", IrType::I32, root);
        h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
        let cond0 = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let loop_r = h.add_region(RegionKind::Loop, Some(root), sp());
        h.regions[loop_r.0 as usize].cond = Some(cond0);
        let header = h.add_region(RegionKind::Block, Some(loop_r), sp());
        let latch = h.add_region(RegionKind::Block, Some(loop_r), sp());
        h.regions[loop_r.0 as usize].header = Some(header);
        h.regions[loop_r.0 as usize].latch = Some(latch);
        // Body: conditional; then: x = 5; continue; else: x = 6 (falls to latch).
        let body = h.add_region(RegionKind::Scope, Some(loop_r), sp());
        let bcond = h.add_expr(ExprKind::Const(0), IrType::Bool, sp());
        let bc = h.add_region(RegionKind::Conditional, Some(body), sp());
        h.regions[bc.0 as usize].cond = Some(bcond);
        let bt = h.add_region(RegionKind::Block, Some(bc), sp());
        let be = h.add_region(RegionKind::Block, Some(bc), sp());
        let five = h.add_expr(ExprKind::Const(5), IrType::I32, sp());
        h.add_stmt(bt, StmtKind::Assign { var: x, value: five }, sp());
        h.add_stmt(bt, StmtKind::Continue(loop_r), sp());
        let six = h.add_expr(ExprKind::Const(6), IrType::I32, sp());
        h.add_stmt(be, StmtKind::Assign { var: x, value: six }, sp());
        let son = hir_to_son(&h).expect("flatten continue loop");
        assert!(son.verify_structure().is_ok());
        let phis = son
            .nodes
            .iter()
            .filter(|n| matches!(n.opcode, Opcode::Phi))
            .count();
        assert!(phis >= 2, "expected header and latch phis, got {phis}");
    }

    /// CEP:WHAT: Flatten is deterministic: two runs over identical HIR produce identical
    /// canonical hashes.
    /// CEP:WHY: 38.10 and 38.19.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_is_deterministic() {
        let build = || {
            let mut h = HirFunction::new("d");
            let root = h.add_region(RegionKind::Block, None, sp());
            let x = h.add_var("x", IrType::I32, root);
            let cond = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
            let cif = h.add_region(RegionKind::Conditional, Some(root), sp());
            h.regions[cif.0 as usize].cond = Some(cond);
            let t = h.add_region(RegionKind::Block, Some(cif), sp());
            let e = h.add_region(RegionKind::Block, Some(cif), sp());
            h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
            let a = h.add_expr(ExprKind::Const(3), IrType::I32, sp());
            h.add_stmt(t, StmtKind::Assign { var: x, value: a }, sp());
            let b = h.add_expr(ExprKind::Const(4), IrType::I32, sp());
            h.add_stmt(e, StmtKind::Assign { var: x, value: b }, sp());
            h
        };
        let s1 = hir_to_son(&build()).expect("run 1");
        let s2 = hir_to_son(&build()).expect("run 2");
        assert_eq!(s1.canonical_hash(), s2.canonical_hash());
    }

    /// CEP:WHAT: An unbound variable read must fail flattening.
    /// CEP:WHY: Fail-closed on verifier-escaped input.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_rejects_unbound_variable() {
        let mut h = HirFunction::new("bad");
        let root = h.add_region(RegionKind::Block, None, sp());
        let ghost = h.add_var("ghost", IrType::I32, root);
        // The read is in scope per HIR rules; the binding statement is absent, which
        // HIR scope checking cannot see. Flattening must fail closed.
        let read = h.add_expr(ExprKind::Var(ghost), IrType::I32, sp());
        h.add_stmt(root, StmtKind::Expr(read), sp());
        match hir_to_son(&h) {
            Err(LowerError::UnboundVariable { .. }) => {}
            other => panic!("expected UnboundVariable, got {other:?}"),
        }
    }

    /// CEP:WHAT: Invalid HIR must be rejected before flattening starts.
    /// CEP:WHY: The lowering verifies its input (38.18).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn flatten_rejects_invalid_hir() {
        let mut h = HirFunction::new("bad");
        let root = h.add_region(RegionKind::Block, None, sp());
        h.add_stmt(root, StmtKind::Break(root), sp());
        match hir_to_son(&h) {
            Err(LowerError::Hir(HirError::InvalidBreakTarget { .. })) => {}
            other => panic!("expected Hir error, got {other:?}"),
        }
    }
}
