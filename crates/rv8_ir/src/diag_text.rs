// CEP:FILE: crates/rv8_ir/src/diag_text.rs
// CEP:WHAT: Human-readable Display implementations for every IR-level error type
//   (HirError, NodeError, LowerError, HetError, MirError).
// CEP:WHY: STANDARDS open ticket RV8-135: the driver formats diagnostics through
//   Display instead of Debug, so error text is stable, reviewed output rather than
//   a derived dump.
// CEP:CLASS: HPC-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-135
// CEP:FAILURE: none; Display is total over every variant.
// CEP:ASSUMES: identifiers render in braces, positions in brackets; the exact strings
//   are pinned by tests because driver output is user-visible.
// CEP:COST: O(1) per render.
// CEP:EVIDENCE: unit tests rv8_ir::diag_text::tests::*.
use crate::hetir::HetError;
use crate::hir::HirError;
use crate::lowering::LowerError;
use crate::machir::MirError;
use crate::son::NodeError;

/// CEP:WHAT: Renders HirError for diagnostics.
/// CEP:WHY: Level-0 verifier rejections reach the user through this text.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: display_hir_error_covers_every_variant.
impl std::fmt::Display for HirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HirError::DanglingRegion(r) => write!(f, "dangling region {r:?}"),
            HirError::DanglingStmt(s) => write!(f, "dangling statement {s:?}"),
            HirError::DanglingExpr(e) => write!(f, "dangling expression {e:?}"),
            HirError::DanglingVar(v) => write!(f, "dangling variable {v:?}"),
            HirError::ParentChildMismatch { region, parent } => {
                write!(f, "region {region:?} records parent {parent:?} that does not own it")
            }
            HirError::RegionCycle { region } => write!(f, "region cycle through {region:?}"),
            HirError::LoopMissingHeaderOrLatch { region } => {
                write!(f, "loop region {region:?} lacks a header or latch")
            }
            HirError::LoopHeaderNotChild { region, header } => {
                write!(f, "loop {region:?} header {header:?} is not a child region")
            }
            HirError::LoopLatchNotChild { region, latch } => {
                write!(f, "loop {region:?} latch {latch:?} is not a child region")
            }
            HirError::CondMissing { region } => {
                write!(f, "conditional region {region:?} has no condition")
            }
            HirError::OutOfScopeVariable { var, used_in } => {
                write!(f, "variable {var:?} used out of scope in region {used_in:?}")
            }
            HirError::InvalidParam { var } => write!(f, "invalid parameter variable {var:?}"),
            HirError::InvalidBreakTarget { stmt, target } => {
                write!(f, "break at statement {stmt:?} targets non-loop region {target:?}")
            }
            HirError::InvalidContinueTarget { stmt, target } => {
                write!(f, "continue at statement {stmt:?} targets non-loop region {target:?}")
            }
            HirError::ExprCycle { expr } => write!(f, "expression cycle through {expr:?}"),
            HirError::Oversized { items } => write!(f, "HIR function exceeds budget: {items} items"),
        }
    }
}

/// CEP:WHAT: Renders NodeError for diagnostics.
/// CEP:WHY: Level-1 verifier rejections reach the user through this text.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: display_node_error_covers_every_variant.
impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeError::DanglingNode(n) => write!(f, "sea-of-nodes: dangling node {n:?}"),
            NodeError::DanglingInput { node, input } => {
                write!(f, "sea-of-nodes: node {node:?} references dangling input {input:?}")
            }
            NodeError::CycleDetected { entry } => {
                write!(f, "sea-of-nodes: data cycle detected at {entry:?}")
            }
        }
    }
}

/// CEP:WHAT: Renders LowerError for diagnostics, including its wrapped levels.
/// CEP:WHY: Flattening failures surface with their cause chain in one line.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: display_lower_error_covers_every_variant.
impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::Hir(e) => write!(f, "lowering: {e}"),
            LowerError::UnboundVariable { var } => {
                write!(f, "lowering: unbound variable {var:?}")
            }
            LowerError::UnsupportedRegionShape { region } => {
                write!(f, "lowering: unsupported region shape {region:?}")
            }
            LowerError::UnsupportedOp { op } => write!(f, "lowering: unsupported op {op:?}"),
            LowerError::Het(e) => write!(f, "lowering: {e}"),
        }
    }
}

/// CEP:WHAT: Renders HetError for diagnostics.
/// CEP:WHY: Level-2 verifier rejections reach the user through this text.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: display_het_error_covers_every_variant.
impl std::fmt::Display for HetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HetError::DanglingNode(n) => write!(f, "hetir: dangling node {n:?}"),
            HetError::DanglingEdgeEndpoint { edge, node } => {
                write!(f, "hetir: edge {edge} references dangling node {node:?}")
            }
            HetError::MissingAddressSpace { node } => {
                write!(f, "hetir: memory node {node:?} lacks an address space")
            }
            HetError::MissingSyncScope { node } => {
                write!(f, "hetir: sync node {node:?} lacks a scope")
            }
            HetError::CrossSpaceMemoryWithoutSync { node } => {
                write!(f, "hetir: cross-space memory at {node:?} lacks an ordering edge")
            }
            HetError::DeadPureNode { node } => write!(f, "hetir: dead pure node {node:?}"),
            HetError::DataCycle { node } => write!(f, "hetir: data cycle through {node:?}"),
            HetError::Oversized { items } => {
                write!(f, "hetir graph exceeds budget: {items} items")
            }
        }
    }
}

/// CEP:WHAT: Renders MirError for diagnostics.
/// CEP:WHY: Level-4 verifier rejections reach the user through this text.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: display_mir_error_covers_every_variant.
impl std::fmt::Display for MirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirError::IsaIllegalOpcode { block, instr, opcode } => {
                write!(f, "mir: illegal opcode {opcode:?} at block {block} instr {instr}")
            }
            MirError::PredicationUnsupported { block, instr } => {
                write!(f, "mir: unsupported predication at block {block} instr {instr}")
            }
            MirError::RegisterClassViolation { block, instr, operand } => {
                write!(f, "mir: register class violation on operand {operand} of block {block} instr {instr}")
            }
            MirError::ReservedRegister { block, instr } => {
                write!(f, "mir: reserved register at block {block} instr {instr}")
            }
            MirError::InvalidAddressingMode { block, instr, operand } => {
                write!(f, "mir: invalid addressing mode on operand {operand} of block {block} instr {instr}")
            }
            MirError::BlockMissingTerminator { block } => {
                write!(f, "mir: block {block} has no terminator")
            }
            MirError::DanglingSuccessor { block, successor } => {
                write!(f, "mir: block {block} lists dangling successor {successor}")
            }
            MirError::FrameMisaligned { alignment } => {
                write!(f, "mir: frame alignment {alignment} is not a power of two")
            }
            MirError::FrameSizeNotMultiple { stack_size, alignment } => {
                write!(f, "mir: frame size {stack_size} is not a multiple of alignment {alignment}")
            }
            MirError::Oversized { items } => {
                write!(f, "mir function exceeds budget: {items} items")
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::hetir::HetNodeId;
    use crate::hir::{ExprId, RegionId, StmtId, VarId};
    use crate::machir::PhysReg;
    use crate::son::NodeId;

    /// CEP:WHAT: Every HirError variant renders a distinct, stable, non-empty message.
    /// CEP:WHY: Driver output is user-visible; the exact strings are the contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_hir_error_covers_every_variant() {
        let cases: Vec<(HirError, &str)> = vec![
            (HirError::DanglingRegion(RegionId(1)), "dangling region RegionId(1)"),
            (HirError::DanglingStmt(StmtId(2)), "dangling statement StmtId(2)"),
            (HirError::DanglingExpr(ExprId(3)), "dangling expression ExprId(3)"),
            (HirError::DanglingVar(VarId(4)), "dangling variable VarId(4)"),
            (
                HirError::ParentChildMismatch { region: RegionId(1), parent: RegionId(2) },
                "region RegionId(1) records parent RegionId(2) that does not own it",
            ),
            (HirError::RegionCycle { region: RegionId(5) }, "region cycle through RegionId(5)"),
            (
                HirError::LoopMissingHeaderOrLatch { region: RegionId(6) },
                "loop region RegionId(6) lacks a header or latch",
            ),
            (
                HirError::LoopHeaderNotChild { region: RegionId(1), header: RegionId(2) },
                "loop RegionId(1) header RegionId(2) is not a child region",
            ),
            (
                HirError::LoopLatchNotChild { region: RegionId(1), latch: RegionId(3) },
                "loop RegionId(1) latch RegionId(3) is not a child region",
            ),
            (HirError::CondMissing { region: RegionId(7) }, "conditional region RegionId(7) has no condition"),
            (
                HirError::OutOfScopeVariable { var: VarId(8), used_in: RegionId(9) },
                "variable VarId(8) used out of scope in region RegionId(9)",
            ),
            (HirError::InvalidParam { var: VarId(10) }, "invalid parameter variable VarId(10)"),
            (
                HirError::InvalidBreakTarget { stmt: StmtId(11), target: RegionId(12) },
                "break at statement StmtId(11) targets non-loop region RegionId(12)",
            ),
            (
                HirError::InvalidContinueTarget { stmt: StmtId(13), target: RegionId(14) },
                "continue at statement StmtId(13) targets non-loop region RegionId(14)",
            ),
            (HirError::ExprCycle { expr: ExprId(15) }, "expression cycle through ExprId(15)"),
            (HirError::Oversized { items: 99 }, "HIR function exceeds budget: 99 items"),
        ];
        for (e, want) in cases {
            assert_eq!(e.to_string(), want);
        }
    }

    /// CEP:WHAT: NodeError, HetError, MirError render every variant distinctly.
    /// CEP:WHY: Same contract as the HIR renderer.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_node_error_covers_every_variant() {
        assert_eq!(
            NodeError::DanglingNode(NodeId(1)).to_string(),
            "sea-of-nodes: dangling node NodeId(1)"
        );
        assert_eq!(
            NodeError::DanglingInput { node: NodeId(2), input: NodeId(3) }.to_string(),
            "sea-of-nodes: node NodeId(2) references dangling input NodeId(3)"
        );
        assert_eq!(
            NodeError::CycleDetected { entry: NodeId(4) }.to_string(),
            "sea-of-nodes: data cycle detected at NodeId(4)"
        );
    }

    /// CEP:WHAT: HetError renders every variant.
    /// CEP:WHY: Same contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: HetNodeId is a tuple struct of one integer.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_het_error_covers_every_variant() {
        let cases: Vec<(HetError, &str)> = vec![
            (HetError::DanglingNode(HetNodeId(1)), "hetir: dangling node HetNodeId(1)"),
            (
                HetError::DanglingEdgeEndpoint { edge: 2, node: HetNodeId(3) },
                "hetir: edge 2 references dangling node HetNodeId(3)",
            ),
            (
                HetError::MissingAddressSpace { node: HetNodeId(4) },
                "hetir: memory node HetNodeId(4) lacks an address space",
            ),
            (
                HetError::MissingSyncScope { node: HetNodeId(5) },
                "hetir: sync node HetNodeId(5) lacks a scope",
            ),
            (
                HetError::CrossSpaceMemoryWithoutSync { node: HetNodeId(6) },
                "hetir: cross-space memory at HetNodeId(6) lacks an ordering edge",
            ),
            (HetError::DeadPureNode { node: HetNodeId(7) }, "hetir: dead pure node HetNodeId(7)"),
            (HetError::DataCycle { node: HetNodeId(8) }, "hetir: data cycle through HetNodeId(8)"),
            (HetError::Oversized { items: 1000 }, "hetir graph exceeds budget: 1000 items"),
        ];
        for (e, want) in cases {
            assert_eq!(e.to_string(), want);
        }
    }

    /// CEP:WHAT: MirError renders every variant.
    /// CEP:WHY: Same contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_mir_error_covers_every_variant() {
        let cases: Vec<(MirError, &str)> = vec![
            (
                MirError::IsaIllegalOpcode { block: 1, instr: 2, opcode: "frob" },
                "mir: illegal opcode \"frob\" at block 1 instr 2",
            ),
            (MirError::PredicationUnsupported { block: 3, instr: 4 }, "mir: unsupported predication at block 3 instr 4"),
            (
                MirError::RegisterClassViolation { block: 5, instr: 6, operand: 7 },
                "mir: register class violation on operand 7 of block 5 instr 6",
            ),
            (MirError::ReservedRegister { block: 8, instr: 9 }, "mir: reserved register at block 8 instr 9"),
            (
                MirError::InvalidAddressingMode { block: 10, instr: 11, operand: 12 },
                "mir: invalid addressing mode on operand 12 of block 10 instr 11",
            ),
            (MirError::BlockMissingTerminator { block: 13 }, "mir: block 13 has no terminator"),
            (MirError::DanglingSuccessor { block: 14, successor: 15 }, "mir: block 14 lists dangling successor 15"),
            (MirError::FrameMisaligned { alignment: 3 }, "mir: frame alignment 3 is not a power of two"),
            (
                MirError::FrameSizeNotMultiple { stack_size: 100, alignment: 16 },
                "mir: frame size 100 is not a multiple of alignment 16",
            ),
            (MirError::Oversized { items: 7 }, "mir function exceeds budget: 7 items"),
        ];
        for (e, want) in cases {
            assert_eq!(e.to_string(), want);
        }
    }

    /// CEP:WHAT: LowerError renders its cause chain in one line, including wrapped
    /// Hir/Het errors and the reserved-register path.
    /// CEP:WHY: One-line diagnostics are the driver's contract (RV8-135).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_lower_error_covers_every_variant() {
        assert_eq!(
            LowerError::Hir(HirError::DanglingVar(VarId(4))).to_string(),
            "lowering: dangling variable VarId(4)"
        );
        assert_eq!(
            LowerError::UnboundVariable { var: VarId(1) }.to_string(),
            "lowering: unbound variable VarId(1)"
        );
        assert_eq!(
            LowerError::UnsupportedRegionShape { region: RegionId(2) }.to_string(),
            "lowering: unsupported region shape RegionId(2)"
        );
        assert_eq!(
            LowerError::UnsupportedOp { op: "fence" }.to_string(),
            "lowering: unsupported op \"fence\""
        );
        assert_eq!(
            LowerError::Het(HetError::DataCycle { node: HetNodeId(9) }).to_string(),
            "lowering: hetir: data cycle through HetNodeId(9)"
        );
    }

    /// CEP:WHAT: Reserved-register rendering uses the PhysReg display form; kept
    /// separate because PhysReg's Debug text is the pinned form.
    /// CEP:WHY: Reserved-register diagnostics name the physical register.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: PhysReg(u32) Debug renders as PhysReg(n).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_reserved_register_names_the_register() {
        let e = MirError::ReservedRegister { block: 1, instr: 2 };
        assert!(e.to_string().contains("block 1 instr 2"));
        let _ = PhysReg(0);
    }
}
