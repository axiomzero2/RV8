// CEP:FILE: crates/rv8_ir/src/lib.rs
// CEP:WHAT: The RV8 five-level IR contract per docs/specs/multi_level_ir.md: Level 0
//   HIR (regions, statements, expressions, variables), Level 1 Sea-of-Nodes MIR, Level 2
//   Heterogeneous IR, Level 3 LIR with register classes, Level 4 Machine IR, plus the
//   level-transition lowerings 6.1-6.3 and physical register types.
// CEP:WHY: One IR contract guarantees printable, hashable, stable-identity
//   representations across frontend, middle-end, backend, and autotuner (standard
//   38.17-38.19); the level transitions are the only sanctioned paths between levels.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: node accessors return Result and fail closed on dangling ids; lowering
//   returns LowerError; verifiers return descriptive errors per level.
// CEP:ASSUMES: ids are dense u32 indices; identity is index-based, never pointer-based;
//   enforced by the per-level verifiers.
// CEP:COST: flat POD vectors; access is O(1) indexed; lowering is O(N + E).
// CEP:EVIDENCE: unit tests across modules and the full-chain integration test below.
// CEP:SECURITY: untrusted source is bounded by per-level item budgets before lowering;
//   all traversals are iterative.

pub mod hir;
pub mod hetir;
pub mod lir;
pub mod lower_het;
pub mod lowering;
pub mod machir;
pub mod son;

pub use hir::{
    BinOp, ExprId, ExprKind, HirError, HirFunction, RegionId, RegionKind, StmtId, StmtKind,
    UnOp, VarId,
};
pub use hetir::{
    AddressSpace, CommOp, ComputeOp, ControlOp, DepKind, HetEdgeKind, HetError, HetIr, HetKind,
    HetNode, HetNodeId, MemoryOp, MetaOp, SyncOp,
};
pub use lir::{
    LirBlock, LirFunction, LirInstruction, LirValue, MachineOpcode, RegClassId, RegId,
    RegisterClass, PhysicalRegister, SetKind, ValueId,
};
pub use lower_het::{hetir_to_lir, son_to_hetir};
pub use lowering::{hir_to_son, LowerError};
pub use machir::{
    Isa, MachineIrFunction, MachineInstr, MachineOperand, MemOperand, MirError, PhysReg,
    Predicate,
};
pub use son::{BlockId, IrType, NodeId, Opcode, SeaNode, SeaOfNodes};

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Runs the full HIR to Sea-of-Nodes to Heterogeneous-IR to LIR chain on
    /// an if/else function and checks level invariants at every boundary.
    /// CEP:WHY: Spec 6 level transitions must compose; this is the pipeline contract
    /// test for levels 0-3.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn full_chain_hir_to_lir() {
        // fn f() { let x = 0; if 1 { x = 10 } else { x = 20 }; return x }
        // Region convention: a region's statements run before its child regions, so the
        // trailing return lives in a sibling tail block after the conditional.
        let sp = || rv8_core::span::Span::new(rv8_core::span::FileId(0), 0, 0);
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let x = h.add_var("x", IrType::I32, root);
        h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
        let cond = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let cif = h.add_region(RegionKind::Conditional, Some(root), sp());
        h.regions[cif.0 as usize].cond = Some(cond);
        let t = h.add_region(RegionKind::Block, Some(cif), sp());
        let e = h.add_region(RegionKind::Block, Some(cif), sp());
        let ten = h.add_expr(ExprKind::Const(10), IrType::I32, sp());
        h.add_stmt(t, StmtKind::Assign { var: x, value: ten }, sp());
        let twenty = h.add_expr(ExprKind::Const(20), IrType::I32, sp());
        h.add_stmt(e, StmtKind::Assign { var: x, value: twenty }, sp());
        let tail = h.add_region(RegionKind::Block, Some(root), sp());
        let read = h.add_expr(ExprKind::Var(x), IrType::I32, sp());
        h.add_stmt(tail, StmtKind::Return(Some(read)), sp());

        // Level 0: verify.
        h.verify().expect("HIR verifies");
        // Transition 6.1.
        let son = hir_to_son(&h).expect("flatten");
        son.verify_structure().expect("SoN structure");
        assert!(
            son.nodes.iter().any(|n| matches!(n.opcode, Opcode::Phi)),
            "merge phi exists"
        );
        // Transition 6.2.
        let het = son_to_hetir(&son).expect("linearize");
        het.verify().expect("HetIR verifies");
        // Transition 6.3.
        let lir = hetir_to_lir(&het, "f").expect("lower to LIR");
        assert!(lir.blocks.len() >= 4, "entry, then, else, join blocks");
        for block in &lir.blocks {
            let last = block.instructions.last().expect("terminator");
            assert!(matches!(
                last.opcode,
                MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret
            ));
        }
        // Values carry register classes (invariant 4.3.2).
        assert!(lir.values.iter().all(|v| {
            matches!(v.reg_class, RegClassId::Gpr) && !v.ty.is_float()
        }));
    }

    /// CEP:WHAT: Runs the full chain twice and requires identical canonical hashes at
    /// every level.
    /// CEP:WHY: Deterministic translation end to end (38.10, 38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn full_chain_is_deterministic() {
        let build = || {
            let sp = || rv8_core::span::Span::new(rv8_core::span::FileId(0), 0, 0);
            let mut h = HirFunction::new("d");
            let root = h.add_region(RegionKind::Block, None, sp());
            let x = h.add_var("x", IrType::I64, root);
            h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
            let a = h.add_expr(ExprKind::Const(5), IrType::I64, sp());
            let b = h.add_expr(ExprKind::Const(7), IrType::I64, sp());
            let sum = h.add_expr(ExprKind::Binary(BinOp::Mul, a, b), IrType::I64, sp());
            h.add_stmt(root, StmtKind::Assign { var: x, value: sum }, sp());
            let read = h.add_expr(ExprKind::Var(x), IrType::I64, sp());
            h.add_stmt(root, StmtKind::Return(Some(read)), sp());
            h
        };
        let (h1, h2) = (build(), build());
        assert_eq!(h1.canonical_hash(), h2.canonical_hash());
        let (s1, s2) = (
            hir_to_son(&h1).expect("run 1"),
            hir_to_son(&h2).expect("run 2"),
        );
        assert_eq!(s1.canonical_hash(), s2.canonical_hash());
        let (g1, g2) = (
            son_to_hetir(&s1).expect("het 1"),
            son_to_hetir(&s2).expect("het 2"),
        );
        assert_eq!(g1.canonical_hash(), g2.canonical_hash());
        let (l1, l2) = (
            hetir_to_lir(&g1, "d").expect("lir 1"),
            hetir_to_lir(&g2, "d").expect("lir 2"),
        );
        assert_eq!(format!("{:?}", l1), format!("{:?}", l2));
    }

    /// CEP:WHAT: Verifies Sea-of-Nodes construction, printing, and canonical hashing
    /// stability.
    /// CEP:WHY: IR contract (printable, hashable, stable).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn son_construction_and_print() {
        let mut ir = SeaOfNodes::new();
        let c1 = ir.add_node(Opcode::Const(1), IrType::I32, vec![]);
        let c2 = ir.add_node(Opcode::Const(2), IrType::I32, vec![]);
        let add = ir.add_node(Opcode::Add, IrType::I32, vec![c1, c2]);
        let ret = ir.add_node(Opcode::Return, IrType::Unit, vec![add]);
        let text = ir.print();
        assert!(text.contains("add"), "print must name opcodes: {text}");
        assert!(text.contains("ret"), "print must name ret: {text}");
        assert_eq!(ir.node(ret).unwrap().inputs, vec![add]);
        let h1 = ir.canonical_hash();
        let h2 = ir.canonical_hash();
        assert_eq!(h1, h2);
        assert!(ir.verify_structure().is_ok());
    }

    /// CEP:WHAT: Verifies LIR construction and value/use wiring.
    /// CEP:WHY: Backend consumers depend on consistent LIR invariants.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lir_construction() {
        let mut func = LirFunction::new("f");
        let v0 = func.add_value(IrType::I32, RegClassId::Gpr);
        let v1 = func.add_value(IrType::I32, RegClassId::Gpr);
        let mut block = LirBlock::new(son::BlockId(0));
        block.push(LirInstruction::add(v0, vec![v0, v1]));
        func.push_block(block);
        assert_eq!(func.blocks.len(), 1);
        assert_eq!(func.values.len(), 2);
    }

    /// CEP:WHAT: Machine IR construction and verification on the x86-64 target.
    /// CEP:WHY: Level 4 contract check inside the workspace test suite.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn machir_level_smoke() {
        let mut f = MachineIrFunction::new("m", Isa::X86_64);
        use machir::PhysReg;
        f.push_block(machir::MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::MovRegImm32,
                    vec![
                        MachineOperand::Reg(PhysReg(0)),
                        MachineOperand::Imm(42),
                    ],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![],
        });
        f.verify().expect("valid MIR");
    }
}
