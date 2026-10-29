// CEP:FILE: crates/rv8_middle/src/pipeline.rs
// CEP:WHAT: The middle-end pipelines: optimize_son runs the fixed pass order
//   [CONST_FOLD, GVN, DCE] over a Sea-of-Nodes graph with structure and deep verification
//   after every pass; optimize_hetir runs dead-node elimination over a Heterogeneous-IR
//   graph (spec 3.3 invariant 4) with verification before and after.
// CEP:WHY: Standard 38.18 mandates verification after every transformation and 38.20 pins
//   pass order; one entry point per IR level keeps the contract auditable.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: every failure path returns MiddleError (verify, pass, or budget); the
//   input graphs are left valid because passes verify at their own boundaries.
// CEP:ASSUMES: optimize_hetir tolerates a dead-pure-node violation on input (it is the
//   fix for invariant 3.3.4) and rejects any other pre-existing HetError.
// CEP:COST: three passes at O(N * fanin) each plus a verifier run after every pass.
// CEP:EVIDENCE: unit tests rv8_middle::pipeline::tests::*.
// CEP:SECURITY: node budgets are checked before any work; no recursion anywhere.
// CEP:DETERMINISM: deterministic; fixed pass order, no hash iteration reaches output.

use crate::analysis::check_budget;
use crate::error::MiddleError;
use crate::passes::{dce, gvn, const_fold, PassReport};
use crate::verifier::verify_son;
use rv8_ir::hetir::{self, HetError, HetIr};
use rv8_ir::son::SeaOfNodes;

/// CEP:WHAT: Summary of one optimization run: reports in fixed execution order.
/// CEP:WHY: Driver diagnostics (38.16) and autotuner features consume per-pass counts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: reports appear in pipeline order (CONST_FOLD, GVN, DCE, or HET_DCE).
/// CEP:COST: POD plus vector.
/// CEP:EVIDENCE: unit test rv8_middle::pipeline::tests::optimize_son_pipeline_is_deterministic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptSummary {
    /// One report per executed pass, in execution order.
    pub passes: Vec<PassReport>,
}

/// CEP:WHAT: Runs verify_structure plus the deep verifier; the gate applied after every
///   pass.
/// CEP:WHY: 38.18 requires both layers after each HPC-0 transformation.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Ir or MiddleError::Verify naming all violations.
/// CEP:ASSUMES: none
/// CEP:COST: O(N * fanin + CFG).
/// CEP:EVIDENCE: unit test rv8_middle::pipeline::tests::optimize_son_pipeline_is_deterministic.
fn verify_pipeline(sea: &SeaOfNodes) -> Result<(), MiddleError> {
    sea.verify_structure()?;
    verify_son(sea)
}

/// CEP:WHAT: Optimizes a Sea-of-Nodes graph with the fixed pass order
///   [CONST_FOLD, GVN, DCE], verifying after every pass.
/// CEP:WHY: The single Level-1 optimization entry point; order is fixed by contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError variants; graph content stays structurally valid on error.
/// CEP:ASSUMES: node count within MAX_MIDDLE_NODES (checked first, fail closed).
/// CEP:COST: O(N * fanin) per pass plus O(CFG) verification after each.
/// CEP:EVIDENCE: unit test rv8_middle::pipeline::tests::optimize_son_pipeline_is_deterministic.
pub fn optimize_son(sea: &mut SeaOfNodes) -> Result<OptSummary, MiddleError> {
    check_budget(sea.nodes.len())?;
    verify_pipeline(sea)?;
    let mut passes: Vec<PassReport> = Vec::with_capacity(3);
    // Fixed order per 38.20 and the task contract: CONST_FOLD, then GVN, then DCE.
    let r = const_fold(sea)?;
    verify_pipeline(sea)?;
    passes.push(r);
    let r = gvn(sea)?;
    verify_pipeline(sea)?;
    passes.push(r);
    let r = dce(sea)?;
    verify_pipeline(sea)?;
    passes.push(r);
    Ok(OptSummary { passes })
}

/// CEP:WHAT: Optimizes a Heterogeneous-IR graph by dead-node elimination: iteratively
///   removes pure nodes with no outgoing edges (invariant 3.3.4) with a worklist, then
///   rebuilds nodes and edges with an ascending id remap and verifies.
/// CEP:WHY: Level 2 must be free of dead pure nodes before the autotuner and LIR
///   lowering consume it (spec 3.3.4, 3.4).
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Het for pre-existing non-dead-node violations on input or
///   any post-pass violation; MiddleError::Budget for oversize graphs.
/// CEP:ASSUMES: a pre-existing DeadPureNode violation is the expected input condition and
///   is repaired; every other HetError fails the run.
/// CEP:COST: O(N + E) adjacency plus O(N + E) worklist and rebuild.
/// CEP:EVIDENCE: unit test rv8_middle::pipeline::tests::optimize_hetir_removes_dead_nodes.
pub fn optimize_hetir(het: &mut HetIr) -> Result<OptSummary, MiddleError> {
    let total = het.nodes.len().saturating_add(het.edges.len());
    if total > hetir::MAX_HET_ITEMS as usize {
        return Err(MiddleError::Budget {
            budget: "MAX_HET_ITEMS",
            limit: u64::from(hetir::MAX_HET_ITEMS),
            actual: total as u64,
        });
    }
    // A dead-pure-node violation is the pass's reason to exist; anything else fails.
    match het.verify() {
        Ok(()) => {}
        Err(HetError::DeadPureNode { .. }) => {}
        Err(e) => return Err(MiddleError::Het(e)),
    }
    let n = het.nodes.len();
    let (out_adj, in_adj) = het.adjacency();
    let mut outdeg: Vec<u32> = out_adj.iter().map(|v| v.len() as u32).collect();
    let mut dead = vec![false; n];
    // Worklist seeded with pure nodes that have no uses; killing a node only reduces the
    // out-degree of its feeders, which is exactly the cascade condition.
    let mut work: Vec<usize> = (0..n)
        .filter(|&i| !het.nodes[i].kind.has_side_effects() && outdeg[i] == 0)
        .collect();
    let mut removed = 0usize;
    while let Some(x) = work.pop() {
        if dead[x] {
            continue;
        }
        dead[x] = true;
        removed += 1;
        if removed > n {
            return Err(MiddleError::Pass { pass: "HET_DCE", detail: "worklist overrun" });
        }
        for &ei in &in_adj[x] {
            let feeder = het.edges[ei].from.0 as usize;
            if !dead[feeder] {
                outdeg[feeder] -= 1;
                if outdeg[feeder] == 0 && !het.nodes[feeder].kind.has_side_effects() {
                    work.push(feeder);
                }
            }
        }
    }
    // Deterministic rebuild: kept nodes in ascending order, edges in original order.
    let mut map: Vec<Option<u32>> = vec![None; n];
    let mut out = HetIr::new();
    for (i, node) in het.nodes.iter().enumerate() {
        if !dead[i] {
            map[i] = Some(out.nodes.len() as u32);
            out.nodes.push(node.clone());
        }
    }
    for edge in &het.edges {
        if let (Some(f), Some(t)) = (
            map[edge.from.0 as usize],
            map[edge.to.0 as usize],
        ) {
            out.edges.push(rv8_ir::hetir::HetEdge {
                from: rv8_ir::hetir::HetNodeId(f),
                to: rv8_ir::hetir::HetNodeId(t),
                kind: edge.kind,
            });
        }
    }
    out.verify()?;
    *het = out;
    Ok(OptSummary {
        passes: vec![PassReport { name: "HET_DCE", changed: removed }],
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::son::{IrType, Opcode};
    use rv8_ir::hetir::{ComputeOp, ControlOp, HetEdgeKind, HetKind, HetNode};
    use rv8_ir::son::BlockId;

    /// CEP:WHAT: Builds a graph with a duplicate pure subgraph and foldable constants.
    /// CEP:WHY: Shared fixture for the pipeline redundancy and determinism tests.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn redundant_graph() -> SeaOfNodes {
        let mut sea = SeaOfNodes::new();
        let a = sea.add_node(Opcode::Const(2), IrType::I32, vec![]);
        let b = sea.add_node(Opcode::Const(3), IrType::I32, vec![]);
        let s1 = sea.add_node(Opcode::Add, IrType::I32, vec![a, b]);
        let a2 = sea.add_node(Opcode::Const(2), IrType::I32, vec![]);
        let b2 = sea.add_node(Opcode::Const(3), IrType::I32, vec![]);
        let s2 = sea.add_node(Opcode::Add, IrType::I32, vec![a2, b2]);
        // Foldable product feeding a root.
        let c = sea.add_node(Opcode::Const(4), IrType::I32, vec![]);
        let d = sea.add_node(Opcode::Const(5), IrType::I32, vec![]);
        let m = sea.add_node(Opcode::Mul, IrType::I32, vec![c, d]);
        let both = sea.add_node(Opcode::Add, IrType::I32, vec![s1, s2]);
        let _ = sea.add_node(Opcode::Return, IrType::Unit, vec![both, m]);
        // Nodes are assigned the entry block so GVN's same-block scope applies.
        for node in sea.nodes.iter_mut() {
            node.block = Some(BlockId(0));
        }
        sea
    }

    /// CEP:WHAT: After optimize_son the redundancy is gone, structure verifies, and a
    ///   second optimize run leaves the canonical hash stable (fixpoint).
    /// CEP:WHY: The task-level pipeline contract and the determinism gate (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn optimize_son_pipeline_is_deterministic() {
        let mut sea = redundant_graph();
        let summary = optimize_son(&mut sea).expect("optimize");
        assert_eq!(
            summary.passes.iter().map(|p| p.name).collect::<Vec<_>>(),
            vec!["CONST_FOLD", "GVN", "DCE"],
            "fixed pass order"
        );
        assert!(sea.verify_structure().is_ok());
        // Redundancy gone: the duplicate subgraph and both adds fold and sweep away,
        // leaving the two folded results and the return.
        let count = |pred: &dyn Fn(&rv8_ir::son::SeaNode) -> bool| -> usize {
            sea.nodes.iter().filter(|n| pred(n)).count()
        };
        assert_eq!(sea.nodes.len(), 3, "folded add results plus return remain");
        assert_eq!(count(&|n| matches!(n.opcode, Opcode::Const(10))), 1, "2+3+2+3 folded");
        assert_eq!(count(&|n| matches!(n.opcode, Opcode::Const(20))), 1, "4*5 folded");
        assert_eq!(
            count(&|n| matches!(n.opcode, Opcode::Add)),
            0,
            "duplicate adds folded and swept"
        );
        let h1 = sea.canonical_hash();
        let summary2 = optimize_son(&mut sea).expect("re-optimize");
        assert_eq!(summary2.passes.iter().map(|p| p.changed).sum::<usize>(), 0, "fixpoint");
        assert_eq!(sea.canonical_hash(), h1, "hash stable across second run");
    }

    /// CEP:WHAT: Two fresh identical graphs optimize to identical hashes.
    /// CEP:WHY: Deterministic translation (38.10): same input, same output.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn optimize_son_same_input_same_hash() {
        let mut s1 = redundant_graph();
        let mut s2 = redundant_graph();
        optimize_son(&mut s1).expect("run 1");
        optimize_son(&mut s2).expect("run 2");
        assert_eq!(s1.canonical_hash(), s2.canonical_hash());
        assert_eq!(format!("{s1:?}"), format!("{s2:?}"));
    }

    /// CEP:WHAT: optimize_son rejects an unverifyable graph with the deep verifier's
    ///   violations instead of optimizing past a broken invariant.
    /// CEP:WHY: Verification after construction is the pipeline's first gate.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn optimize_son_rejects_broken_graph() {
        let mut sea = SeaOfNodes::new();
        let p0 = sea.add_node(Opcode::Parameter(0), IrType::I32, vec![]);
        let p1 = sea.add_node(Opcode::Parameter(0), IrType::I32, vec![]);
        let sum = sea.add_node(Opcode::Add, IrType::I32, vec![p0, p1]);
        let _ = sea.add_node(Opcode::Return, IrType::Unit, vec![sum]);
        match optimize_son(&mut sea) {
            Err(MiddleError::Verify(vs)) => assert!(!vs.is_empty()),
            other => panic!("expected Verify, got {other:?}"),
        }
    }

    /// CEP:WHAT: optimize_hetir removes dead pure nodes (including cascades) and keeps
    ///   the graph verifiable; a second run is a fixpoint.
    /// CEP:WHY: Invariant 3.3.4 is the minimum Level-2 optimization contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: het node construction follows rv8_ir test patterns.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn optimize_hetir_removes_dead_nodes() {
        let mk = |kind: HetKind| HetNode {
            kind,
            ty: IrType::I32,
            addr_space: None,
            addr_dst: None,
            sync_scope: None,
            imm: 0,
            block: None,
            span: rv8_core::span::Span::new(rv8_core::span::FileId(0), 0, 0),
        };
        let mut het = HetIr::new();
        let a = het.add_node(mk(HetKind::Compute(ComputeOp::Const)));
        let b = het.add_node(mk(HetKind::Compute(ComputeOp::Const)));
        let dead1 = het.add_node(mk(HetKind::Compute(ComputeOp::Add)));
        let dead2 = het.add_node(mk(HetKind::Compute(ComputeOp::Mul)));
        let ret = het.add_node(mk(HetKind::Control(ControlOp::Return)));
        // b stays used by the return; dead2 consumes only dead1, so both die as a cascade.
        het.add_edge(a, dead1, HetEdgeKind::Data { ty: IrType::I32 });
        het.add_edge(dead1, dead2, HetEdgeKind::Data { ty: IrType::I32 });
        het.add_edge(a, ret, HetEdgeKind::Control);
        het.add_edge(b, ret, HetEdgeKind::Control);
        let summary = optimize_hetir(&mut het).expect("het dce");
        assert_eq!(summary.passes[0].name, "HET_DCE");
        assert_eq!(summary.passes[0].changed, 2, "cascade removes both dead nodes");
        assert!(het.verify().is_ok());
        assert_eq!(het.nodes.len(), 3, "a, b, ret kept");
        let h1 = het.canonical_hash();
        let again = optimize_hetir(&mut het).expect("het dce 2");
        assert_eq!(again.passes[0].changed, 0, "fixpoint");
        assert_eq!(het.canonical_hash(), h1);
    }

    /// CEP:WHAT: A graph whose only remaining node id reference is the kept Return keeps
    ///   its edge wiring after the rebuild (endpoint remap correctness).
    /// CEP:WHY: Id remapping bugs are the classic DCE failure mode.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn optimize_hetir_remaps_edges() {
        let mk = || HetNode {
            kind: HetKind::Compute(ComputeOp::Const),
            ty: IrType::I32,
            addr_space: None,
            addr_dst: None,
            sync_scope: None,
            imm: 0,
            block: None,
            span: rv8_core::span::Span::new(rv8_core::span::FileId(0), 0, 0),
        };
        let mut het = HetIr::new();
        let live1 = het.add_node(mk());
        let dead = het.add_node(mk());
        let live2 = het.add_node(mk());
        let ret = het.add_node(HetNode {
            kind: HetKind::Control(ControlOp::Return),
            ty: IrType::Unit,
            ..mk()
        });
        het.add_edge(live1, live2, HetEdgeKind::Data { ty: IrType::I32 });
        het.add_edge(live1, dead, HetEdgeKind::Data { ty: IrType::I32 });
        het.add_edge(live2, ret, HetEdgeKind::Control);
        optimize_hetir(&mut het).expect("het dce");
        assert_eq!(het.nodes.len(), 3);
        assert_eq!(het.edges.len(), 2, "dead node's edge dropped, others kept");
        assert!(het.verify().is_ok());
    }
}
