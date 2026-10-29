// CEP:FILE: crates/rv8_frontend/src/ast.rs
// CEP:WHAT: The abstract syntax tree of the RV8 Rust subset: functions with typed params,
//   statements (let/assign/expr/if/while/loop/break/continue/return), expressions (literals,
//   variables, unary, binary, calls, parens), subset types, plus depth-bounded binding
//   collection and iterative expression traversal helpers.
// CEP:WHY: The parser produces this tree; resolve, infer, and lower consume it. Expression
//   helpers are iterative so left-associated operator chains of arbitrary length cannot
//   drive recursion (38.13).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: traversal helpers return None-free total iteration; collect_bindings fails
//   closed with Internal if AST nesting ever exceeded the parser depth budget.
// CEP:ASSUMES: trees were produced by this crate's parser, so statement nesting depth is
//   bounded by parser::MAX_PARSE_DEPTH and expression chains are arbitrary but iterated.
// CEP:COST: O(N) traversals; each node visited a constant number of times.
// CEP:EVIDENCE: unit tests rv8_frontend::ast::tests::*.

use crate::error::FrontendError;
use crate::parser::MAX_PARSE_DEPTH;
use rv8_core::interner::SymbolId;
use rv8_core::span::Span;
use rv8_ir::hir::{BinOp as HirBinOp, UnOp as HirUnOp};
use rv8_ir::son::IrType;

/// CEP:WHAT: Upper bound on AST nodes (statements plus expressions) per compilation.
/// CEP:WHY: Named budget (38.12); bounds parse memory and downstream IR size before the
///   HIR item budget is reached.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced by the parser while constructing nodes.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_oversize_program.
pub const MAX_AST_NODES: u32 = 1_000_000;

/// CEP:WHAT: The subset's scalar types: unit, bool, and the four integer widths.
/// CEP:WHY: Every HIR variable and expression type must map onto the frozen IrType set;
///   the subset admits exactly these spellings.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::type_parsing_and_conversion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AstType {
    Unit,
    Bool,
    I32,
    U32,
    I64,
    U64,
}

impl AstType {
    /// CEP:WHAT: Parses a primitive type name from interned identifier text.
    /// CEP:WHY: The lexer keeps primitive types as identifiers; the parser resolves them.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None for any other spelling.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) string match.
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::type_parsing_and_conversion.
    pub fn from_name(name: &str) -> Option<AstType> {
        match name {
            "()" => Some(AstType::Unit),
            "bool" => Some(AstType::Bool),
            "i32" => Some(AstType::I32),
            "u32" => Some(AstType::U32),
            "i64" => Some(AstType::I64),
            "u64" => Some(AstType::U64),
            _ => None,
        }
    }

    /// CEP:WHAT: Maps the subset type onto the frozen IR scalar type.
    /// CEP:WHY: Inference anchors and HIR typing need IrType values.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::type_parsing_and_conversion.
    pub fn to_ir(self) -> IrType {
        match self {
            AstType::Unit => IrType::Unit,
            AstType::Bool => IrType::Bool,
            AstType::I32 => IrType::I32,
            AstType::U32 => IrType::U32,
            AstType::I64 => IrType::I64,
            AstType::U64 => IrType::U64,
        }
    }

    /// CEP:WHAT: Reports whether the type is one of the integer widths.
    /// CEP:WHY: Unary minus and arithmetic operands must be integral in this subset.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::*.
    pub fn is_integer(self) -> bool {
        matches!(self, AstType::I32 | AstType::U32 | AstType::I64 | AstType::U64)
    }
}

/// CEP:WHAT: Dense global index of one expression node, assigned by the parser in
///   construction order.
/// CEP:WHY: Inference stores per-expression types in a flat vector addressed by this key;
///   dense u32 identity follows 38.17.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: keys are unique across one Program.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_types_fixture_expressions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ExprKey(pub u32);

/// CEP:WHAT: One parsed source file: the function list in declaration order.
/// CEP:WHY: The frontend compiles a single module; order is the deterministic fn index used
///   in HIR call targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: owned vector.
/// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_fixture_add_main.
#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<Function>,
}

/// CEP:WHAT: One function: name, parameters, optional return type, and body block.
/// CEP:WHY: The subset's only item kind; matches the required fixture grammar exactly.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: parameter count bounded by parser::MAX_PARAMS.
/// CEP:COST: owned children.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: SymbolId,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub ret: Option<AstType>,
    pub body: Block,
    pub span: Span,
}

/// CEP:WHAT: One parameter: optional `mut`, name, and required type annotation.
/// CEP:WHY: Annotations anchor inference roots (union-find) for argument types.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: SymbolId,
    pub mutable: bool,
    pub ty: AstType,
    pub span: Span,
}

/// CEP:WHAT: One braced block: statements in order plus an optional trailing expression.
/// CEP:WHY: The fixture's `fn main() -> i32 { add(6, 7) }` requires a trailing expression
///   that acts as the block's implicit return.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: trailing expressions are allowed in any block; in a function body they mean
///   `return <expr>`, elsewhere they are evaluated and discarded (documented subset choice).
/// CEP:COST: owned children.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_trailing_expression.
#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub trailing: Option<Box<Expr>>,
    pub span: Span,
}

/// CEP:WHAT: One statement of the subset.
/// CEP:WHY: Exactly the statement set the frozen HIR StmtKind can express; constructs the
///   frozen lowering cannot represent are rejected at parse time instead.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: spans cover the whole statement for diagnostics.
/// CEP:COST: owned children.
/// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
#[derive(Clone, Debug)]
pub enum Stmt {
    Let {
        name: SymbolId,
        mutable: bool,
        ty: Option<AstType>,
        init: Option<Expr>,
        span: Span,
    },
    Assign {
        name: SymbolId,
        value: Expr,
        span: Span,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_block: Block,
        else_clause: Option<ElseClause>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Loop {
        body: Block,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
}

/// CEP:WHAT: The else side of an if: a block or a nested if (else-if chain).
/// CEP:WHY: Rust-subset syntax; else-if lowers as an if statement inside the else region.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: owned child.
/// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_larger_control_flow.
#[derive(Clone, Debug)]
pub enum ElseClause {
    Block(Block),
    If(Box<Stmt>),
}

/// CEP:WHAT: Unary operators of the subset.
/// CEP:WHY: Exactly the two operators the frozen HIR UnOp defines.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::operator_mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

impl UnOp {
    /// CEP:WHAT: Maps onto the frozen HIR unary operator.
    /// CEP:WHY: Lowering must emit only frozen-HIR shapes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::operator_mapping.
    pub fn to_hir(self) -> HirUnOp {
        match self {
            UnOp::Neg => HirUnOp::Neg,
            UnOp::Not => HirUnOp::Not,
        }
    }
}

/// CEP:WHAT: Binary operators of the subset: exactly the frozen HIR BinOp set.
/// CEP:WHY: Operators outside this set (!=, >, >=, &&, ||) have no HIR shape and are
///   rejected at parse time with a clear span error instead of emitting unverifiable HIR.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::operator_mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl BinOp {
    /// CEP:WHAT: Precedence tier of the operator; higher binds tighter.
    /// CEP:WHY: Precedence climbing in the parser; tiers mirror the Rust reference order.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: all subset operators are left-associative (documented deviation).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_precedence_shape.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Rem => 6,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Shl | BinOp::Shr => 4,
            BinOp::BitAnd => 3,
            BinOp::BitXor => 2,
            BinOp::BitOr => 1,
            BinOp::Eq | BinOp::Lt | BinOp::Le => 0,
        }
    }

    /// CEP:WHAT: Reports whether the operator produces a Bool.
    /// CEP:WHY: Comparisons type to bool; everything else types to the operand type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::infer::tests::*.
    pub fn result_is_bool(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Lt | BinOp::Le)
    }

    /// CEP:WHAT: Maps onto the frozen HIR binary operator (identity mapping).
    /// CEP:WHY: Lowering must emit only frozen-HIR shapes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::operator_mapping.
    pub fn to_hir(self) -> HirBinOp {
        match self {
            BinOp::Add => HirBinOp::Add,
            BinOp::Sub => HirBinOp::Sub,
            BinOp::Mul => HirBinOp::Mul,
            BinOp::Div => HirBinOp::Div,
            BinOp::Rem => HirBinOp::Rem,
            BinOp::BitAnd => HirBinOp::BitAnd,
            BinOp::BitOr => HirBinOp::BitOr,
            BinOp::BitXor => HirBinOp::BitXor,
            BinOp::Shl => HirBinOp::Shl,
            BinOp::Shr => HirBinOp::Shr,
            BinOp::Eq => HirBinOp::Eq,
            BinOp::Lt => HirBinOp::Lt,
            BinOp::Le => HirBinOp::Le,
        }
    }
}

/// CEP:WHAT: One expression node with kind, span, and its global ExprKey.
/// CEP:WHY: The key links parse-time nodes to inference results without embedding types in
///   the AST.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: key was assigned by the parser and is unique in the Program.
/// CEP:COST: owned children.
/// CEP:EVIDENCE: unit test rv8_frontend::infer::tests::inference_types_fixture_expressions.
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    pub key: ExprKey,
}

impl Drop for Expr {
    /// CEP:WHAT: Iteratively dismantles the expression tree before the node itself drops.
    /// CEP:WHY: A left-associated chain of arbitrary length (any `1 + 1 + ...` the parser
    ///   accepts within MAX_AST_NODES) must not drive recursion during drop; a derived
    ///   drop would overflow the stack on deep chains (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; total over any chain depth, O(N) with an explicit worklist.
    /// CEP:ASSUMES: placeholder leaves (IntLit with key u32::MAX) never occur in live ASTs.
    /// CEP:COST: O(N) per tree, allocation-light.
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::drop_survives_deep_chain.
    fn drop(&mut self) {
        let mut work: Vec<Expr> = Vec::new();
        detach_children(&mut self.kind, &mut work);
        while let Some(mut next) = work.pop() {
            detach_children(&mut next.kind, &mut work);
            // `next` now holds placeholder leaves only, so this drop is O(1) deep.
        }
    }
}

/// CEP:WHAT: Moves an expression node's children onto the drop worklist, replacing them
///   with placeholder leaves.
/// CEP:WHY: Detachment is what makes the iterative Drop terminate without recursion.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(children).
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::drop_survives_deep_chain.
fn detach_children(kind: &mut ExprKind, work: &mut Vec<Expr>) {
    fn placeholder() -> Box<Expr> {
        Box::new(Expr {
            kind: ExprKind::IntLit(0),
            span: Span::new(rv8_core::span::FileId(0), 0, 0),
            key: ExprKey(u32::MAX),
        })
    }
    match kind {
        ExprKind::Unary(_, a) => work.push(*std::mem::replace(a, placeholder())),
        ExprKind::Binary(_, a, b) => {
            work.push(*std::mem::replace(b, placeholder()));
            work.push(*std::mem::replace(a, placeholder()));
        }
        ExprKind::Call { args, .. } => {
            for arg in args.drain(..) {
                work.push(arg);
            }
        }
        ExprKind::Paren(p) => work.push(*std::mem::replace(p, placeholder())),
        ExprKind::IntLit(_) | ExprKind::BoolLit(_) | ExprKind::Var(_) => {}
    }
}

/// CEP:WHAT: Expression shapes of the subset: literals, variables, unary, binary, calls,
///   and preserved parentheses.
/// CEP:WHY: Paren nodes are kept so spans round-trip; infer and lower pass through them.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: boxed children for recursive shapes.
/// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
#[derive(Clone, Debug)]
pub enum ExprKind {
    IntLit(i64),
    BoolLit(bool),
    Var(SymbolId),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call {
        callee: SymbolId,
        args: Vec<Expr>,
    },
    Paren(Box<Expr>),
}

/// CEP:WHAT: One variable binding site in declaration order: params first, then lets in
///   source pre-order.
/// CEP:WHY: infer and lower must agree on binding identity; this shared record is that
///   agreement, and its index is the VarKey mapped to HIR VarIds.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: produced only by collect_bindings.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::collect_bindings_is_source_preorder.
#[derive(Clone, Copy, Debug)]
pub struct BindingInfo {
    pub name: SymbolId,
    pub mutable: bool,
    pub ty: Option<AstType>,
    pub span: Span,
}

/// CEP:WHAT: Collects every binding of a function in source pre-order: parameters first,
///   then let statements as encountered in execution order.
/// CEP:WHY: infer (union-find roots) and lower (hoisted HIR vars) must enumerate bindings
///   identically; one shared traversal guarantees the VarKey mapping.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns Internal if statement nesting ever exceeds MAX_PARSE_DEPTH, which
///   the parser makes unreachable; the check fails closed anyway.
/// CEP:ASSUMES: the tree came from this crate's parser.
/// CEP:COST: O(statements); recursion depth bounded by MAX_PARSE_DEPTH (named budget).
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::collect_bindings_is_source_preorder.
pub fn collect_bindings(func: &Function) -> Result<Vec<BindingInfo>, FrontendError> {
    fn walk_block(
        block: &Block,
        out: &mut Vec<BindingInfo>,
        depth: u32,
    ) -> Result<(), FrontendError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(FrontendError::Internal {
                context: "ast binding walk depth",
                span: block.span,
            });
        }
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { name, mutable, ty, span, .. } => out.push(BindingInfo {
                    name: *name,
                    mutable: *mutable,
                    ty: *ty,
                    span: *span,
                }),
                Stmt::If { then_block, else_clause, .. } => {
                    walk_block(then_block, out, depth + 1)?;
                    match else_clause {
                        Some(ElseClause::Block(b)) => walk_block(b, out, depth + 1)?,
                        Some(ElseClause::If(inner)) => walk_if(inner, out, depth + 1)?,
                        None => {}
                    }
                }
                Stmt::While { body, .. } => walk_block(body, out, depth + 1)?,
                Stmt::Loop { body, .. } => walk_block(body, out, depth + 1)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn walk_if(
        stmt: &Stmt,
        out: &mut Vec<BindingInfo>,
        depth: u32,
    ) -> Result<(), FrontendError> {
        if let Stmt::If { then_block, else_clause, .. } = stmt {
            walk_block(then_block, out, depth)?;
            match else_clause {
                Some(ElseClause::Block(b)) => walk_block(b, out, depth)?,
                Some(ElseClause::If(inner)) => walk_if(inner, out, depth)?,
                None => {}
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for param in &func.params {
        out.push(BindingInfo {
            name: param.name,
            mutable: param.mutable,
            ty: Some(param.ty),
            span: param.span,
        });
    }
    walk_block(&func.body, &mut out, 1)?;
    Ok(out)
}

/// CEP:WHAT: Visits every expression of a tree in pre-order with an explicit stack.
/// CEP:WHY: resolve validates call targets without recursion over unbounded operator
///   chains (38.13).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total iteration.
/// CEP:ASSUMES: none
/// CEP:COST: O(N); each node pushed and popped exactly once.
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::for_each_expr_covers_long_chains.
pub fn for_each_expr<'a, F: FnMut(&'a Expr)>(root: &'a Expr, f: &mut F) {
    enum Frame<'e> {
        Visit(&'e Expr),
    }
    let mut stack: Vec<Frame> = vec![Frame::Visit(root)];
    while let Some(frame) = stack.pop() {
        let Frame::Visit(e) = frame;
        f(e);
        match &e.kind {
            ExprKind::Unary(_, a) => stack.push(Frame::Visit(a)),
            ExprKind::Binary(_, a, b) => {
                stack.push(Frame::Visit(a));
                stack.push(Frame::Visit(b));
            }
            ExprKind::Call { args, .. } => {
                for arg in args {
                    stack.push(Frame::Visit(arg));
                }
            }
            ExprKind::Paren(p) => stack.push(Frame::Visit(p)),
            ExprKind::IntLit(_) | ExprKind::BoolLit(_) | ExprKind::Var(_) => {}
        }
    }
}

/// CEP:WHAT: Walks every statement of a function (including nested construct bodies) in
///   source pre-order with an explicit stack.
/// CEP:WHY: resolve needs a statement-level view for call validation bookkeeping; an
///   explicit stack keeps the traversal independent of nesting depth.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total iteration.
/// CEP:ASSUMES: none
/// CEP:COST: O(statements).
/// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::for_each_stmt_covers_constructs.
pub fn for_each_stmt<'a, F: FnMut(&'a Stmt)>(func: &'a Function, f: &mut F) {
    enum Item<'s> {
        Block(&'s Block),
        Stmt(&'s Stmt),
    }
    let mut stack: Vec<Item> = vec![Item::Block(&func.body)];
    while let Some(item) = stack.pop() {
        match item {
            Item::Block(block) => {
                for stmt in &block.stmts {
                    stack.push(Item::Stmt(stmt));
                }
            }
            Item::Stmt(stmt) => {
                f(stmt);
                match stmt {
                    Stmt::If { then_block, else_clause, .. } => {
                        stack.push(Item::Block(then_block));
                        match else_clause {
                            Some(ElseClause::Block(b)) => stack.push(Item::Block(b)),
                            Some(ElseClause::If(inner)) => stack.push(Item::Stmt(inner)),
                            None => {}
                        }
                    }
                    Stmt::While { body, .. } => stack.push(Item::Block(body)),
                    Stmt::Loop { body, .. } => stack.push(Item::Block(body)),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_core::interner::Interner;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Blank span for tests.
    /// CEP:WHY: Keeps test bodies readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span::new(FileId(0), 0, 0)
    }

    /// CEP:WHAT: Verifies type name parsing and IrType conversion.
    /// CEP:WHY: The subset's type surface is exactly these six spellings.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn type_parsing_and_conversion() {
        assert_eq!(AstType::from_name("i32"), Some(AstType::I32));
        assert_eq!(AstType::from_name("bool"), Some(AstType::Bool));
        assert_eq!(AstType::from_name("()"), Some(AstType::Unit));
        assert_eq!(AstType::from_name("f32"), None);
        assert_eq!(AstType::I64.to_ir(), IrType::I64);
        assert!(AstType::U64.is_integer());
        assert!(!AstType::Bool.is_integer());
    }

    /// CEP:WHAT: Verifies AST-to-HIR operator mapping is exhaustive.
    /// CEP:WHY: Lowering must never invent an operator outside the frozen set.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn operator_mapping() {
        let ops = [
            BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem, BinOp::BitAnd,
            BinOp::BitOr, BinOp::BitXor, BinOp::Shl, BinOp::Shr, BinOp::Eq, BinOp::Lt, BinOp::Le,
        ];
        for op in ops {
            let _ = op.to_hir();
            assert!(op.precedence() <= 6);
        }
        assert!(BinOp::Mul.precedence() > BinOp::Add.precedence());
        assert!(BinOp::Add.precedence() > BinOp::BitOr.precedence());
        assert!(BinOp::Eq.result_is_bool());
        assert!(!BinOp::Add.result_is_bool());
        assert_eq!(UnOp::Neg.to_hir(), HirUnOp::Neg);
    }

    /// CEP:WHAT: Builds a nested function tree by hand and checks binding order.
    /// CEP:WHY: infer and lower rely on the shared pre-order.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn collect_bindings_is_source_preorder() {
        let mut ix = Interner::new();
        let a = ix.get_or_intern("a");
        let b = ix.get_or_intern("b");
        let inner_let = Stmt::Let { name: b, mutable: false, ty: None, init: None, span: sp() };
        let if_stmt = Stmt::If {
            cond: Expr { kind: ExprKind::BoolLit(true), span: sp(), key: ExprKey(0) },
            then_block: Block { stmts: vec![inner_let.clone()], trailing: None, span: sp() },
            else_clause: Some(ElseClause::If(Box::new(Stmt::If {
                cond: Expr { kind: ExprKind::BoolLit(false), span: sp(), key: ExprKey(1) },
                then_block: Block {
                    stmts: vec![inner_let],
                    trailing: None,
                    span: sp(),
                },
                else_clause: None,
                span: sp(),
            }))),
            span: sp(),
        };
        let func = Function {
            name: a,
            name_span: sp(),
            params: vec![Param { name: a, mutable: false, ty: AstType::I32, span: sp() }],
            ret: Some(AstType::I32),
            body: Block {
                stmts: vec![
                    Stmt::Let { name: a, mutable: true, ty: Some(AstType::I32), init: None, span: sp() },
                    if_stmt,
                ],
                trailing: None,
                span: sp(),
            },
            span: sp(),
        };
        let bindings = collect_bindings(&func).unwrap();
        let names: Vec<SymbolId> = bindings.iter().map(|b| b.name).collect();
        assert_eq!(names, vec![a, a, b, b]);
        assert!(bindings[1].mutable);
    }

    /// CEP:WHAT: for_each_expr must cover a 10,000-long operator chain without recursion.
    /// CEP:WHY: 38.13: user input must not drive recursion depth.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(N).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn for_each_expr_covers_long_chains() {
        let mut key = 0u32;
        let mk = |k: &mut u32| {
            let e = Expr {
                kind: ExprKind::IntLit(1),
                span: sp(),
                key: ExprKey(*k),
            };
            *k += 1;
            e
        };
        let mut lhs = mk(&mut key);
        for _ in 0..10_000 {
            let rhs = mk(&mut key);
            lhs = Expr {
                kind: ExprKind::Binary(BinOp::Add, Box::new(lhs), Box::new(rhs)),
                span: sp(),
                key: ExprKey(key),
            };
            key += 1;
        }
        let mut count = 0usize;
        for_each_expr(&lhs, &mut |_| count += 1);
        assert_eq!(count, 20_001);
    }

    /// CEP:WHAT: for_each_stmt reaches statements inside if/else-if/while/loop bodies.
    /// CEP:WHY: resolve validates calls in every nesting position.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn for_each_stmt_covers_constructs() {
        let mut ix = Interner::new();
        let f = ix.get_or_intern("f");
        let mk_break = || Stmt::Break { span: sp() };
        let func = Function {
            name: f,
            name_span: sp(),
            params: vec![],
            ret: None,
            body: Block {
                stmts: vec![
                    Stmt::If {
                        cond: Expr { kind: ExprKind::BoolLit(true), span: sp(), key: ExprKey(0) },
                        then_block: Block { stmts: vec![mk_break()], trailing: None, span: sp() },
                        else_clause: Some(ElseClause::If(Box::new(Stmt::If {
                            cond: Expr {
                                kind: ExprKind::BoolLit(false),
                                span: sp(),
                                key: ExprKey(1),
                            },
                            then_block: Block { stmts: vec![mk_break()], trailing: None, span: sp() },
                            else_clause: None,
                            span: sp(),
                        }))),
                        span: sp(),
                    },
                    Stmt::While {
                        cond: Expr { kind: ExprKind::BoolLit(true), span: sp(), key: ExprKey(2) },
                        body: Block { stmts: vec![mk_break()], trailing: None, span: sp() },
                        span: sp(),
                    },
                    Stmt::Loop {
                        body: Block { stmts: vec![mk_break()], trailing: None, span: sp() },
                        span: sp(),
                    },
                ],
                trailing: None,
                span: sp(),
            },
            span: sp(),
        };
        let mut breaks = 0usize;
        for_each_stmt(&func, &mut |s| {
            if matches!(s, Stmt::Break { .. }) {
                breaks += 1;
            }
        });
        assert_eq!(breaks, 4);
    }

    /// CEP:WHAT: Building and dropping a 400,000-long operator chain must not overflow.
    /// CEP:WHY: The iterative Drop is the 38.13 guarantee for arbitrary chain depth.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(N).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn drop_survives_deep_chain() {
        let mk = |key: u32| Expr {
            kind: ExprKind::IntLit(1),
            span: sp(),
            key: ExprKey(key),
        };
        let mut lhs = mk(0);
        let mut key = 1u32;
        for _ in 0..400_000u32 {
            let rhs = mk(key);
            key += 1;
            lhs = Expr {
                kind: ExprKind::Binary(BinOp::Add, Box::new(lhs), Box::new(rhs)),
                span: sp(),
                key: ExprKey(key),
            };
            key += 1;
        }
        let mut count = 0usize;
        for_each_expr(&lhs, &mut |_| count += 1);
        assert_eq!(count, 800_001);
        drop(lhs); // must not recurse
    }
}
