// CEP:FILE: crates/rv8_ir/src/hir.rs
// CEP:WHAT: Level 0 High-Level IR (HIR) per docs/specs/multi_level_ir.md section 1: a region
//   tree preserving structured control flow and lexical scoping, with statements, expression
//   trees, and typed variables, plus a verifier for the four HIR invariants (1.3.1-1.3.4).
// CEP:WHY: The frontend produces HIR from the AST (spec 7); loop nesting and scope structure
//   must survive as explicit data so loop transformations and SSA construction stay simple.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: accessors are total over dense indices; verify() returns a descriptive
//   HirError for every violated invariant instead of panicking.
// CEP:ASSUMES: ids are dense u32 indices into the owning HirFunction; identity is index-based
//   (38.17); the region graph is a tree enforced by verify().
// CEP:COST: flat POD vectors; construction amortized O(1); verify() is O(R + S + E) with
//   iterative traversals only.
// CEP:EVIDENCE: unit tests rv8_ir::hir::tests::*.
// CEP:SECURITY: all traversals are iterative with explicit stacks; untrusted input cannot
//   drive recursion depth (38.13).
// CEP:NOTE: Region execution order convention: a region's statements execute before its
//   child regions in child order; source code that follows a structured construct lives
//   in a subsequent sibling Block region. The flattening pass relies on this order.

use crate::son::IrType;
use rv8_core::span::Span;
use std::fmt::Write as _;

/// CEP:WHAT: Maximum regions, statements, expressions, or variables per HIR function.
/// CEP:WHY: Bounded compile time and memory (38.12); named limit, not a magic number.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced by the producer (frontend) and re-checked by verify().
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_budget_guard_rejects_oversize.
pub const MAX_HIR_ITEMS: u32 = 1_000_000;

/// CEP:WHAT: Stable region identity inside one HirFunction.
/// CEP:WHY: Index-based identity satisfies the no-pointer-identity rule (38.17).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionId(pub u32);

/// CEP:WHAT: Stable statement identity inside one HirFunction.
/// CEP:WHY: Region statement lists reference statements by index.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StmtId(pub u32);

/// CEP:WHAT: Stable expression-node identity inside one HirFunction.
/// CEP:WHY: Expression trees are flattened into the exprs vector; edges are indices.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExprId(pub u32);

/// CEP:WHAT: Stable variable identity inside one HirFunction.
/// CEP:WHY: Variables carry their declaring region for scope checking (1.3.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_out_of_scope_variable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct VarId(pub u32);

/// CEP:WHAT: Region kinds: straight-line block, lexical scope, loop, conditional.
/// CEP:WHY: The four structured-control containers named by spec 1.2; a closed set keeps
///   the flattening pass and verifier exhaustive.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: loop and conditional regions carry their controlling expression in
///   HirRegion::cond; other kinds leave cond as None.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_loop_without_header_and_latch.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RegionKind {
    Block,
    Scope,
    Loop,
    Conditional,
}

/// CEP:WHAT: One HIR region: kind, parent, children, statement list, controlling
/// expression, loop header/latch children, and source span.
/// CEP:WHY: The nesting container whose invariants 1.3.1-1.3.2 the verifier enforces.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; structural problems are reported by HirFunction::verify.
/// CEP:ASSUMES: execution order is statements first, then children in list order;
/// header/latch are Some exactly for Loop regions (verifier-checked); children and
/// parents are mutually consistent (verifier-checked).
/// CEP:COST: POD plus child vector.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Debug)]
pub struct HirRegion {
    pub kind: RegionKind,
    pub parent: Option<RegionId>,
    pub children: Vec<RegionId>,
    pub stmts: Vec<StmtId>,
    pub cond: Option<ExprId>,
    /// CEP:WHAT: The unique header subregion of a Loop region.
    /// CEP:WHY: Invariant 1.3.2 requires exactly one header per loop.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: set only on Loop regions; must name a direct child.
    /// CEP:COST: POD.
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_loop_without_header_and_latch.
    pub header: Option<RegionId>,
    /// CEP:WHAT: The unique latch subregion of a Loop region.
    /// CEP:WHY: Invariant 1.3.2 requires exactly one latch per loop; the latch is the sole
    ///   back-edge source during flattening (spec 6.1).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: set only on Loop regions; must name a direct child distinct from header.
    /// CEP:COST: POD.
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_loop_without_header_and_latch.
    pub latch: Option<RegionId>,
    pub span: Span,
}

/// CEP:WHAT: Binary expression operators of the HIR expression language.
/// CEP:WHY: One closed operator set keeps lowering to Sea-of-Nodes opcodes total.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable (used in hashing).
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Lt,
    Le,
}

/// CEP:WHAT: Unary expression operators.
/// CEP:WHY: Negation and bitwise complement lower to Neg/Not nodes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

/// CEP:WHAT: One HIR expression node: constants, variable reads, unary and binary trees,
///   and calls.
/// CEP:WHY: Spec 1.2 requires computation trees; flattening maps each kind to one or two
///   Sea-of-Nodes nodes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: operand edges reference existing expressions (verifier-checked for cycles).
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Debug)]
pub enum ExprKind {
    Const(i64),
    Var(VarId),
    Unary(UnOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    Call { callee: u32, args: Vec<ExprId> },
}

/// CEP:WHAT: One HIR expression tree node with its scalar type and source span.
/// CEP:WHY: Carries provenance (span) through flattening into MIR diagnostics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ty is the result type of the node.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Debug)]
pub struct HirExpr {
    pub kind: ExprKind,
    pub ty: IrType,
    pub span: Span,
}

/// CEP:WHAT: One HIR statement: let, assignment, expression statement, break, continue,
///   or return.
/// CEP:WHY: Spec 1.2 statement set; break/continue carry their target loop for 1.3.4.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: break/continue targets are Loop regions (verifier-checked for validity).
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Debug)]
pub enum StmtKind {
    Let { var: VarId, init: Option<ExprId> },
    Assign { var: VarId, value: ExprId },
    Expr(ExprId),
    Break(RegionId),
    Continue(RegionId),
    Return(Option<ExprId>),
}

/// CEP:WHAT: One HIR statement with its source span.
/// CEP:WHY: Provenance chain requirement (38.15).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
#[derive(Clone, Debug)]
pub struct HirStmt {
    pub kind: StmtKind,
    pub span: Span,
}

/// CEP:WHAT: One HIR variable: type annotation and declaring region.
/// CEP:WHY: Invariant 1.3.3 scope checking needs the declaration site.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: declared region exists in the same function.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_out_of_scope_variable.
#[derive(Clone, Debug)]
pub struct HirVar {
    pub name: String,
    pub ty: IrType,
    pub declared_in: RegionId,
}

/// CEP:WHAT: Error returned by HirFunction::verify for every violated spec-1.3 invariant.
/// CEP:WHY: Fail-closed contracts without panics; each variant names the offending ids.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit tests rv8_ir::hir::tests::hir_rejects_*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HirError {
    DanglingRegion(RegionId),
    DanglingStmt(StmtId),
    DanglingExpr(ExprId),
    DanglingVar(VarId),
    ParentChildMismatch { region: RegionId, parent: RegionId },
    RegionCycle { region: RegionId },
    LoopMissingHeaderOrLatch { region: RegionId },
    LoopHeaderNotChild { region: RegionId, header: RegionId },
    LoopLatchNotChild { region: RegionId, latch: RegionId },
    CondMissing { region: RegionId },
    OutOfScopeVariable { var: VarId, used_in: RegionId },
    InvalidParam { var: VarId },
    InvalidBreakTarget { stmt: StmtId, target: RegionId },
    InvalidContinueTarget { stmt: StmtId, target: RegionId },
    ExprCycle { expr: ExprId },
    Oversized { items: u32 },
}

/// CEP:WHAT: Enforces the per-function item budget.
/// CEP:WHY: Named compile-time budget (38.12); factored as a pure function so the guard is
///   unit-testable without constructing a million-item function.
/// CEP:STATUS: complete
/// CEP:FAILURE: Oversized when items exceeds MAX_HIR_ITEMS.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_budget_guard_rejects_oversize.
fn check_budget(items: u32) -> Result<(), HirError> {
    if items > MAX_HIR_ITEMS {
        Err(HirError::Oversized { items })
    } else {
        Ok(())
    }
}

/// CEP:WHAT: The HIR for one function: flat region/statement/expression/variable vectors
///   plus the ordered parameter list and the root region.
/// CEP:WHY: Central Level-0 container consumed by structural passes and by flattening
///   (spec 6.1); `params` carries the ABI-visible argument order so flattening can emit
///   Opcode::Parameter entry nodes in declaration order.
/// CEP:STATUS: complete
/// CEP:FAILURE: see HirError.
/// CEP:ASSUMES: one function per container; root names a valid region without parent.
/// CEP:COST: construction amortized O(1) per item.
/// CEP:EVIDENCE: unit tests rv8_ir::hir::tests::*.
#[derive(Clone, Debug, Default)]
pub struct HirFunction {
    pub name: String,
    pub regions: Vec<HirRegion>,
    pub stmts: Vec<HirStmt>,
    pub exprs: Vec<HirExpr>,
    pub vars: Vec<HirVar>,
    /// CEP:WHAT: Ordered parameter variables, index = ABI argument position.
    /// CEP:WHY: Flattening binds each to an Opcode::Parameter entry node; downstream
    ///   ABI pre-coloring maps the index to the argument register.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: verify rejects dangling, duplicate, or non-root param vars.
    /// CEP:ASSUMES: each id names a var declared in the root region.
    /// CEP:COST: POD vector.
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::flatten_binds_parameters.
    pub params: Vec<VarId>,
    pub root: Option<RegionId>,
}

impl HirFunction {
    /// CEP:WHAT: Creates an empty HIR function.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
    pub fn new(name: &str) -> HirFunction {
        HirFunction {
            name: name.to_string(),
            regions: Vec::new(),
            stmts: Vec::new(),
            exprs: Vec::new(),
            vars: Vec::new(),
            params: Vec::new(),
            root: None,
        }
    }

    /// CEP:WHAT: Declares the ordered parameter variables of the function.
    /// CEP:WHY: Single construction path for the ABI argument order; fails closed on
    ///   dangling, duplicate, or non-root variables instead of trusting the caller.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: InvalidParam when a var id is dangling, duplicated, or declared
    ///   outside the root region.
    /// CEP:ASSUMES: the root region exists (set before calling).
    /// CEP:COST: O(params).
    /// CEP:EVIDENCE: unit tests rv8_ir::lowering::tests::flatten_binds_parameters and
    ///   hir_rejects_bad_params.
    pub fn set_params(&mut self, params: Vec<VarId>) -> Result<(), HirError> {
        let root = self.root.ok_or(HirError::DanglingRegion(RegionId(u32::MAX)))?;
        for (i, &var) in params.iter().enumerate() {
            let v = self
                .vars
                .get(var.0 as usize)
                .ok_or(HirError::InvalidParam { var })?;
            if v.declared_in != root {
                return Err(HirError::InvalidParam { var });
            }
            if params[..i].contains(&var) {
                return Err(HirError::InvalidParam { var });
            }
        }
        self.params = params;
        Ok(())
    }

    /// CEP:WHAT: Appends a region with a parent and returns its id.
    /// CEP:WHY: Single construction path keeps indices dense and parent links mandatory.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; oversize is reported by verify (Oversized).
    /// CEP:ASSUMES: parent names an existing region or None for the root.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
    pub fn add_region(
        &mut self,
        kind: RegionKind,
        parent: Option<RegionId>,
        span: Span,
    ) -> RegionId {
        let id = RegionId(self.regions.len() as u32);
        self.regions.push(HirRegion {
            kind,
            parent,
            children: Vec::new(),
            stmts: Vec::new(),
            cond: None,
            header: None,
            latch: None,
            span,
        });
        match parent {
            Some(p) => {
                if let Some(pr) = self.regions.get_mut(p.0 as usize) {
                    pr.children.push(id);
                }
            }
            None => {
                if self.root.is_none() {
                    self.root = Some(id);
                }
            }
        }
        id
    }

    /// CEP:WHAT: Appends a statement and registers it in the given region.
    /// CEP:WHY: Regions own their statement order; one path keeps both vectors consistent.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: region exists.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
    pub fn add_stmt(&mut self, region: RegionId, kind: StmtKind, span: Span) -> StmtId {
        let id = StmtId(self.stmts.len() as u32);
        self.stmts.push(HirStmt { kind, span });
        if let Some(r) = self.regions.get_mut(region.0 as usize) {
            r.stmts.push(id);
        }
        id
    }

    /// CEP:WHAT: Appends an expression node and returns its id.
    /// CEP:WHY: Expression trees flatten into the dense exprs vector.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
    pub fn add_expr(&mut self, kind: ExprKind, ty: IrType, span: Span) -> ExprId {
        let id = ExprId(self.exprs.len() as u32);
        self.exprs.push(HirExpr { kind, ty, span });
        id
    }

    /// CEP:WHAT: Appends a variable declared in the given region.
    /// CEP:WHY: Variables carry their declaration site for scope checking (1.3.3).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: region exists.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_out_of_scope_variable.
    pub fn add_var(&mut self, name: &str, ty: IrType, declared_in: RegionId) -> VarId {
        let id = VarId(self.vars.len() as u32);
        self.vars.push(HirVar {
            name: name.to_string(),
            ty,
            declared_in,
        });
        id
    }

    /// CEP:WHAT: Immutable region accessor.
    /// CEP:WHY: Fail-closed access (38.6).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingRegion for unknown ids.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_construction_and_verify.
    pub fn region(&self, id: RegionId) -> Result<&HirRegion, HirError> {
        self.regions.get(id.0 as usize).ok_or(HirError::DanglingRegion(id))
    }

    /// CEP:WHAT: Verifies all four spec-1.3 invariants over the whole function.
    /// CEP:WHY: 38.18 requires verification after construction and after every pass; HIR
    ///   structural passes must re-establish 1.3.1-1.3.4 before flattening.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns the first violated invariant as a HirError variant naming ids.
    /// CEP:ASSUMES: none; total over any HirFunction contents.
    /// CEP:COST: O(R + S + E); all traversals iterative.
    /// CEP:EVIDENCE: unit tests rv8_ir::hir::tests::hir_rejects_* (one per invariant).
    pub fn verify(&self) -> Result<(), HirError> {
        let total = match self
            .regions
            .len()
            .checked_add(self.stmts.len())
            .and_then(|v| v.checked_add(self.exprs.len()))
            .and_then(|v| v.checked_add(self.vars.len()))
        {
            Some(v) => v as u32,
            None => return Err(HirError::Oversized { items: u32::MAX }),
        };
        check_budget(total)?;
        let root = self.root.ok_or(HirError::DanglingRegion(RegionId(u32::MAX)))?;

        // Parameter contract: every param var exists, is unique in the list, and is
        // declared in the root region (mirrors set_params; verify is authoritative for
        // functions assembled without the setter).
        for (i, &var) in self.params.iter().enumerate() {
            let v = self
                .vars
                .get(var.0 as usize)
                .ok_or(HirError::InvalidParam { var })?;
            if v.declared_in != root || self.params[..i].contains(&var) {
                return Err(HirError::InvalidParam { var });
            }
        }

        // 1.3.1 acyclic and properly nested: parent/child links are mutually consistent and
        // every parent chain terminates at the root without revisiting a region.
        for (i, region) in self.regions.iter().enumerate() {
            let id = RegionId(i as u32);
            match region.parent {
                Some(p) => {
                    let pr = self.region(p)?;
                    if !pr.children.contains(&id) {
                        return Err(HirError::ParentChildMismatch { region: id, parent: p });
                    }
                }
                None => {
                    if id != root {
                        return Err(HirError::ParentChildMismatch { region: id, parent: id });
                    }
                }
            }
            let mut steps = 0u32;
            let mut cur = region.parent;
            while let Some(p) = cur {
                if p == id {
                    return Err(HirError::RegionCycle { region: id });
                }
                steps += 1;
                if steps > self.regions.len() as u32 {
                    return Err(HirError::RegionCycle { region: id });
                }
                cur = self.region(p)?.parent;
            }
        }
        let roots = self.regions.iter().filter(|r| r.parent.is_none()).count();
        if roots != 1 {
            return Err(HirError::ParentChildMismatch { region: root, parent: root });
        }

        // 1.3.2 every loop region has exactly one header and one latch, both direct
        // children, plus a controlling expression; conditionals also require a condition.
        for (i, region) in self.regions.iter().enumerate() {
            match region.kind {
                RegionKind::Loop => {
                    let id = RegionId(i as u32);
                    let (header, latch) = match (region.header, region.latch) {
                        (Some(h), Some(l)) if h != l => (h, l),
                        _ => return Err(HirError::LoopMissingHeaderOrLatch { region: id }),
                    };
                    if !region.children.contains(&header) {
                        return Err(HirError::LoopHeaderNotChild { region: id, header });
                    }
                    if !region.children.contains(&latch) {
                        return Err(HirError::LoopLatchNotChild { region: id, latch });
                    }
                    if region.cond.is_none() {
                        return Err(HirError::CondMissing { region: id });
                    }
                }
                RegionKind::Conditional => {
                    if region.cond.is_none() {
                        return Err(HirError::CondMissing { region: RegionId(i as u32) });
                    }
                }
                RegionKind::Block | RegionKind::Scope => {}
            }
        }

        // Expression forest acyclicity and operand existence, O(E), before scope walks.
        self.verify_expr_forest()?;

        // 1.3.3 variable scoping and 1.3.4 transfer-target validity, statement by statement.
        for var in &self.vars {
            self.region(var.declared_in)?;
        }
        for (ri, region) in self.regions.iter().enumerate() {
            let rid = RegionId(ri as u32);
            for &sid in &region.stmts {
                let stmt = self
                    .stmts
                    .get(sid.0 as usize)
                    .ok_or(HirError::DanglingStmt(sid))?;
                match &stmt.kind {
                    StmtKind::Let { var, init } => {
                        self.check_var_scope(*var, rid)?;
                        if let Some(e) = init {
                            self.check_expr_scope(*e, rid)?;
                        }
                    }
                    StmtKind::Assign { var, value } => {
                        self.check_var_scope(*var, rid)?;
                        self.check_expr_scope(*value, rid)?;
                    }
                    StmtKind::Expr(e) => self.check_expr_scope(*e, rid)?,
                    StmtKind::Break(target) => {
                        let tr = self.region(*target)?;
                        if tr.kind != RegionKind::Loop || !self.is_ancestor_or_self(*target, rid)
                        {
                            return Err(HirError::InvalidBreakTarget {
                                stmt: sid,
                                target: *target,
                            });
                        }
                    }
                    StmtKind::Continue(target) => {
                        let tr = self.region(*target)?;
                        if tr.kind != RegionKind::Loop || !self.is_ancestor_or_self(*target, rid)
                        {
                            return Err(HirError::InvalidContinueTarget {
                                stmt: sid,
                                target: *target,
                            });
                        }
                    }
                    StmtKind::Return(value) => {
                        if let Some(e) = value {
                            self.check_expr_scope(*e, rid)?;
                        }
                    }
                }
            }
            if let Some(c) = region.cond {
                self.check_expr_scope(c, rid)?;
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Detects cycles and dangling operands across the whole expression forest
    /// with an iterative gray/black DFS.
    /// CEP:WHY: Scope checks and lowering assume trees; a per-root chain walk misses cycles
    /// reachable through second operands, so the forest is colored once in O(E).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: ExprCycle for any back edge; DanglingExpr for any operand id outside
    /// the vector.
    /// CEP:ASSUMES: none; total over any contents.
    /// CEP:COST: O(E); each expression enters the stack once, operands scanned exactly
    /// once, one small Vec per visited node.
    /// CEP:EVIDENCE: unit test
    /// rv8_ir::hir::tests::hir_rejects_expr_cycle_through_second_operand.
    fn verify_expr_forest(&self) -> Result<(), HirError> {
        const UNVISITED: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;
        let mut state = vec![UNVISITED; self.exprs.len()];
        for start in 0..self.exprs.len() {
            if state[start] != UNVISITED {
                continue;
            }
            state[start] = GRAY;
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            while let Some(frame) = stack.last_mut() {
                let operands: Vec<ExprId> = match &self.exprs[frame.0].kind {
                    ExprKind::Unary(_, a) => vec![*a],
                    ExprKind::Binary(_, a, b) => vec![*a, *b],
                    ExprKind::Call { args, .. } => args.clone(),
                    ExprKind::Const(_) | ExprKind::Var(_) => Vec::new(),
                };
                if frame.1 < operands.len() {
                    let child = operands[frame.1];
                    frame.1 += 1;
                    if child.0 as usize >= self.exprs.len() {
                        return Err(HirError::DanglingExpr(child));
                    }
                    match state[child.0 as usize] {
                        GRAY => return Err(HirError::ExprCycle { expr: child }),
                        UNVISITED => {
                            state[child.0 as usize] = GRAY;
                            stack.push((child.0 as usize, 0));
                        }
                        _ => {}
                    }
                } else {
                    state[frame.0] = BLACK;
                    stack.pop();
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Checks one variable read or write against the scope of a region.
    /// CEP:WHY: Invariant 1.3.3 shared by statement and expression checks.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingVar or OutOfScopeVariable.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(depth).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_out_of_scope_variable.
    fn check_var_scope(&self, var: VarId, region: RegionId) -> Result<(), HirError> {
        let v = self
            .vars
            .get(var.0 as usize)
            .ok_or(HirError::DanglingVar(var))?;
        if self.is_ancestor_or_self(v.declared_in, region) {
            Ok(())
        } else {
            Err(HirError::OutOfScopeVariable { var, used_in: region })
        }
    }

    /// CEP:WHAT: Checks that every variable read inside an expression tree is in scope for
    /// the given region, and that operand ids exist.
    /// CEP:WHY: Invariant 1.3.3 applies transitively through expression trees.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingExpr or OutOfScopeVariable.
    /// CEP:ASSUMES: expression trees are acyclic (verify_expr_forest ran first).
    /// CEP:COST: O(E) per call with an explicit worklist.
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_out_of_scope_variable.
    fn check_expr_scope(&self, root: ExprId, region: RegionId) -> Result<(), HirError> {
        let mut work = vec![root];
        let mut steps = 0u32;
        while let Some(e) = work.pop() {
            steps += 1;
            if steps > self.exprs.len() as u32 + 1 {
                return Err(HirError::ExprCycle { expr: e });
            }
            let node = self
                .exprs
                .get(e.0 as usize)
                .ok_or(HirError::DanglingExpr(e))?;
            match &node.kind {
                ExprKind::Var(var) => self.check_var_scope(*var, region),
                ExprKind::Unary(_, a) => {
                    work.push(*a);
                    Ok(())
                }
                ExprKind::Binary(_, a, b) => {
                    work.push(*a);
                    work.push(*b);
                    Ok(())
                }
                ExprKind::Call { args, .. } => {
                    work.extend_from_slice(args);
                    Ok(())
                }
                ExprKind::Const(_) => Ok(()),
            }?;
        }
        Ok(())
    }

    /// CEP:WHAT: Reports whether ancestor is an ancestor of descendant (or equal).
    /// CEP:WHY: Scope and break/continue validity checks (1.3.3, 1.3.4).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: parent chains are acyclic (verify() enforces before use).
    /// CEP:COST: O(depth).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_rejects_invalid_break_target.
    pub fn is_ancestor_or_self(&self, ancestor: RegionId, descendant: RegionId) -> bool {
        let mut cur = Some(descendant);
        while let Some(r) = cur {
            if r == ancestor {
                return true;
            }
            cur = self.regions.get(r.0 as usize).and_then(|rg| rg.parent);
        }
        false
    }

    /// CEP:WHAT: Prints the region tree with statements in stable id order.
    /// CEP:WHY: Printable IR is part of the contract (38.17); structural passes diff text.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: verify() passed (tree structure).
    /// CEP:COST: O(R + S).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_print_is_stable.
    pub fn print(&self) -> String {
        let mut out = String::new();
        let mut work: Vec<(RegionId, u32)> = self.root.map(|r| vec![(r, 0)]).unwrap_or_default();
        while let Some((rid, depth)) = work.pop() {
            let region = match self.regions.get(rid.0 as usize) {
                Some(r) => r,
                None => continue,
            };
            let pad = "  ".repeat(depth as usize);
            let _ = writeln!(
                out,
                "{}region r{} {:?} stmts={}",
                pad,
                rid.0,
                region.kind,
                region.stmts.len()
            );
            for &sid in &region.stmts {
                let _ = writeln!(out, "{}  stmt s{}", pad, sid.0);
            }
            for &child in region.children.iter().rev() {
                work.push((child, depth + 1));
            }
        }
        out
    }

    /// CEP:WHAT: Computes a deterministic content hash over regions, statements,
    /// expressions, and variables.
    /// CEP:WHY: Structural passes and the CAS cache key on content (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: vector order is canonical (construction order).
    /// CEP:COST: O(N).
    /// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_hash_is_deterministic.
    pub fn canonical_hash(&self) -> u64 {
        let mut hash: u64 = 0;
        for region in &self.regions {
            hash = mix(hash, &[region.kind as u8]);
            hash = mix(
                hash,
                &region.parent.map(|p| p.0).unwrap_or(u32::MAX).to_le_bytes(),
            );
            hash = mix(
                hash,
                &region.header.map(|p| p.0).unwrap_or(u32::MAX).to_le_bytes(),
            );
            hash = mix(
                hash,
                &region.latch.map(|p| p.0).unwrap_or(u32::MAX).to_le_bytes(),
            );
            hash = mix(
                hash,
                &region.cond.map(|p| p.0).unwrap_or(u32::MAX).to_le_bytes(),
            );
            for &c in &region.children {
                hash = mix(hash, &c.0.to_le_bytes());
            }
            for &s in &region.stmts {
                hash = mix(hash, &s.0.to_le_bytes());
            }
        }
        for stmt in &self.stmts {
            hash = mix(hash, &[stmt_kind_tag(&stmt.kind)]);
            match &stmt.kind {
                StmtKind::Let { var, init } => {
                    hash = mix(hash, &var.0.to_le_bytes());
                    hash = mix(
                        hash,
                        &init.map(|e| e.0).unwrap_or(u32::MAX).to_le_bytes(),
                    );
                }
                StmtKind::Assign { var, value } => {
                    hash = mix(hash, &var.0.to_le_bytes());
                    hash = mix(hash, &value.0.to_le_bytes());
                }
                StmtKind::Expr(e) => hash = mix(hash, &e.0.to_le_bytes()),
                StmtKind::Break(t) | StmtKind::Continue(t) => {
                    hash = mix(hash, &t.0.to_le_bytes());
                }
                StmtKind::Return(v) => {
                    hash = mix(hash, &v.map(|e| e.0).unwrap_or(u32::MAX).to_le_bytes());
                }
            }
        }
        for expr in &self.exprs {
            hash = mix(hash, &[expr.ty as u8]);
            hash = mix(hash, &[expr_tag(&expr.kind)]);
            match &expr.kind {
                ExprKind::Const(v) => hash = mix(hash, &v.to_le_bytes()),
                ExprKind::Var(v) => hash = mix(hash, &v.0.to_le_bytes()),
                ExprKind::Unary(op, a) => {
                    hash = mix(hash, &(*op as u8).to_le_bytes());
                    hash = mix(hash, &a.0.to_le_bytes());
                }
                ExprKind::Binary(op, a, b) => {
                    hash = mix(hash, &(*op as u8).to_le_bytes());
                    hash = mix(hash, &a.0.to_le_bytes());
                    hash = mix(hash, &b.0.to_le_bytes());
                }
                ExprKind::Call { callee, args } => {
                    hash = mix(hash, &callee.to_le_bytes());
                    for &a in args {
                        hash = mix(hash, &a.0.to_le_bytes());
                    }
                }
            }
        }
        for var in &self.vars {
            hash = mix(hash, var.name.as_bytes());
            hash = mix(hash, &var.declared_in.0.to_le_bytes());
        }
        hash
    }
}

/// CEP:WHAT: Chains bytes into the running canonical hash state.
/// CEP:WHY: Shared helper for HIR hashing; keeps call sites one line.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_hash_is_deterministic.
fn mix(state: u64, bytes: &[u8]) -> u64 {
    rv8_core::fx_mix(state, rv8_core::fxhash64(bytes))
}

/// CEP:WHAT: Assigns a stable discriminant byte to each statement kind for hashing.
/// CEP:WHY: Explicit tag map instead of relying on enum layout stability (38.19).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_hash_is_deterministic.
fn stmt_kind_tag(kind: &StmtKind) -> u8 {
    match kind {
        StmtKind::Let { .. } => 0,
        StmtKind::Assign { .. } => 1,
        StmtKind::Expr(_) => 2,
        StmtKind::Break(_) => 3,
        StmtKind::Continue(_) => 4,
        StmtKind::Return(_) => 5,
    }
}

/// CEP:WHAT: Assigns a stable discriminant byte to each expression kind for hashing.
/// CEP:WHY: Same rationale as stmt_kind_tag.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hir::tests::hir_hash_is_deterministic.
fn expr_tag(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::Const(_) => 0,
        ExprKind::Var(_) => 1,
        ExprKind::Unary(_, _) => 2,
        ExprKind::Binary(_, _, _) => 3,
        ExprKind::Call { .. } => 4,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Blank span for tests.
    /// CEP:WHY: Keeps test bodies readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span {
            file: FileId(0),
            lo: 0,
            hi: 0,
        }
    }

    /// CEP:WHAT: Builds fn { let x = 1 + 2; x } and verifies all invariants.
    /// CEP:WHY: Happy-path construction, verify, print, and hash stability.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_construction_and_verify() {
        let mut f = HirFunction::new("f");
        let root = f.add_region(RegionKind::Block, None, sp());
        let x = f.add_var("x", IrType::I32, root);
        let a = f.add_expr(ExprKind::Const(1), IrType::I32, sp());
        let b = f.add_expr(ExprKind::Const(2), IrType::I32, sp());
        let sum = f.add_expr(ExprKind::Binary(BinOp::Add, a, b), IrType::I32, sp());
        f.add_stmt(root, StmtKind::Let { var: x, init: Some(sum) }, sp());
        let read = f.add_expr(ExprKind::Var(x), IrType::I32, sp());
        f.add_stmt(root, StmtKind::Expr(read), sp());
        f.verify().expect("well-formed HIR must verify");
        assert!(f.print().contains("region r0"));
    }

    /// CEP:WHAT: Builds an if/else with a loop and checks loop header/latch invariants.
    /// CEP:WHY: Covers 1.3.2 happy path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_accepts_well_formed_loop() {
        let mut f = HirFunction::new("loop_fn");
        let root = f.add_region(RegionKind::Block, None, sp());
        let cond = f.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let loopr = f.add_region(RegionKind::Loop, Some(root), sp());
        f.regions[loopr.0 as usize].cond = Some(cond);
        let header = f.add_region(RegionKind::Block, Some(loopr), sp());
        let latch = f.add_region(RegionKind::Block, Some(loopr), sp());
        f.regions[loopr.0 as usize].header = Some(header);
        f.regions[loopr.0 as usize].latch = Some(latch);
        let t = f.add_expr(ExprKind::Const(0), IrType::Bool, sp());
        f.add_stmt(latch, StmtKind::Continue(loopr), sp());
        f.add_stmt(header, StmtKind::Return(Some(t)), sp());
        f.verify().expect("well-formed loop must verify");
    }

    /// CEP:WHAT: A loop region without header/latch must fail verification.
    /// CEP:WHY: Invariant 1.3.2.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_rejects_loop_without_header_and_latch() {
        let mut f = HirFunction::new("bad");
        let root = f.add_region(RegionKind::Block, None, sp());
        let cond = f.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let loopr = f.add_region(RegionKind::Loop, Some(root), sp());
        f.regions[loopr.0 as usize].cond = Some(cond);
        match f.verify() {
            Err(HirError::LoopMissingHeaderOrLatch { .. }) => {}
            other => panic!("expected LoopMissingHeaderOrLatch, got {other:?}"),
        }
    }

    /// CEP:WHAT: A variable read outside its declaration scope must fail verification.
    /// CEP:WHY: Invariant 1.3.3.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_rejects_out_of_scope_variable() {
        let mut f = HirFunction::new("bad");
        let root = f.add_region(RegionKind::Block, None, sp());
        let inner_scope = f.add_region(RegionKind::Scope, Some(root), sp());
        let x = f.add_var("x", IrType::I32, inner_scope);
        // x is read in root, which is not inner_scope or a descendant of it.
        let read = f.add_expr(ExprKind::Var(x), IrType::I32, sp());
        f.add_stmt(root, StmtKind::Expr(read), sp());
        match f.verify() {
            Err(HirError::OutOfScopeVariable { .. }) => {}
            other => panic!("expected OutOfScopeVariable, got {other:?}"),
        }
    }

    /// CEP:WHAT: A break whose target is not an enclosing loop must fail verification.
    /// CEP:WHY: Invariant 1.3.4.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_rejects_invalid_break_target() {
        let mut f = HirFunction::new("bad");
        let root = f.add_region(RegionKind::Block, None, sp());
        // Break targets root, which is a Block, not a Loop.
        f.add_stmt(root, StmtKind::Break(root), sp());
        match f.verify() {
            Err(HirError::InvalidBreakTarget { .. }) => {}
            other => panic!("expected InvalidBreakTarget, got {other:?}"),
        }
    }

    /// CEP:WHAT: A region that is its own ancestor must fail verification.
    /// CEP:WHY: Invariant 1.3.1.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_rejects_region_cycle() {
        let mut f = HirFunction::new("bad");
        let root = f.add_region(RegionKind::Block, None, sp());
        let a = f.add_region(RegionKind::Scope, Some(root), sp());
        // Forge a cycle: root's parent points at its own child.
        f.regions[root.0 as usize].parent = Some(a);
        match f.verify() {
            Err(HirError::RegionCycle { .. }) | Err(HirError::ParentChildMismatch { .. }) => {}
            other => panic!("expected cycle error, got {other:?}"),
        }
    }

    /// CEP:WHAT: A cycle reachable only through a second operand must fail verification.
    /// CEP:WHY: Guards the O(E) forest coloring against the chain-walk blind spot.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_rejects_expr_cycle_through_second_operand() {
        let mut f = HirFunction::new("bad");
        let root = f.add_region(RegionKind::Block, None, sp());
        let a = f.add_expr(ExprKind::Const(0), IrType::I32, sp());
        // y (id 1) references x (id 3) before it exists; x = Binary(p, y) closes the
        // cycle reachable only through y's second operand slot.
        let y = f.add_expr(ExprKind::Binary(BinOp::Add, a, ExprId(3)), IrType::I32, sp());
        let p = f.add_expr(ExprKind::Const(1), IrType::I32, sp());
        let _x = f.add_expr(ExprKind::Binary(BinOp::Add, p, y), IrType::I32, sp());
        let _ = root;
        match f.verify() {
            Err(HirError::ExprCycle { .. }) => {}
            other => panic!("expected ExprCycle, got {other:?}"),
        }
    }

    /// CEP:WHAT: Printing twice yields identical text.
    /// CEP:WHY: 38.19 determinism of textual IR.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_print_is_stable() {
        let mut f = HirFunction::new("p");
        let root = f.add_region(RegionKind::Block, None, sp());
        let c = f.add_expr(ExprKind::Const(7), IrType::I64, sp());
        f.add_stmt(root, StmtKind::Expr(c), sp());
        assert_eq!(f.print(), f.print());
    }

    /// CEP:WHAT: canonical_hash is equal for identical content.
    /// CEP:WHY: 38.19 content-addressed caching.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_hash_is_deterministic() {
        let build = || {
            let mut f = HirFunction::new("h");
            let root = f.add_region(RegionKind::Block, None, sp());
            let x = f.add_var("x", IrType::I32, root);
            let a = f.add_expr(ExprKind::Const(1), IrType::I32, sp());
            f.add_stmt(root, StmtKind::Let { var: x, init: Some(a) }, sp());
            f
        };
        let f1 = build();
        let f2 = build();
        assert_eq!(f1.canonical_hash(), f2.canonical_hash());
        // Mutated content must hash differently.
        let mut f3 = build();
        f3.exprs[0].kind = ExprKind::Const(2);
        assert_ne!(f1.canonical_hash(), f3.canonical_hash());
    }

    /// CEP:WHAT: The budget guard rejects counts above MAX_HIR_ITEMS and accepts the
    /// boundary.
    /// CEP:WHY: Named compile-time budget (38.12) enforced by verify through check_budget.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(1).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn hir_budget_guard_rejects_oversize() {
        assert!(check_budget(MAX_HIR_ITEMS).is_ok());
        match check_budget(MAX_HIR_ITEMS + 1) {
            Err(HirError::Oversized { items }) => assert_eq!(items, MAX_HIR_ITEMS + 1),
            other => panic!("expected Oversized, got {other:?}"),
        }
    }
}
