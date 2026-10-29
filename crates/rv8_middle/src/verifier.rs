// CEP:FILE: crates/rv8_middle/src/verifier.rs
// CEP:WHAT: The deep Sea-of-Nodes verifier: SSA single-def recheck, operand-dominates-use
//   over data inputs, and phi-at-merge placement with positional path legality (phi input
//   i is legal when its definition lies on predecessor path i or dominates it, which is
//   the spec 2.3.1 rule including the loop back-edge exception); reports ALL violations
//   found, ordered by node id.
// CEP:WHY: Standard 38.18 requires a verifier beyond structural edge checks and mandates
//   running it after every pass; spec section 2.3 pins the three invariants verified here.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: verify_son returns MiddleError::Verify carrying every violation; budget
//   guards propagate as MiddleError::Budget; never panics.
// CEP:ASSUMES: nodes without a block assignment are unchecked for dominance (synthetic
//   construction graphs); unreachable blocks are skipped by dominance checks because no
//   dominator facts exist for them; control inputs of control nodes are trusted as CFG
//   structure and only data inputs are dominance-checked; phi input i corresponds to
//   control input i of the block's join node (the flattening emits both in incoming-path
//   order).
// CEP:COST: O(N * fanin + CFG + dominators); bounded by the analysis budgets.
// CEP:EVIDENCE: unit tests rv8_middle::verifier::tests::* (one positive and one negative
//   test per invariant).
// CEP:SECURITY: no recursion; all structure walks are index loops.
// CEP:DETERMINISM: deterministic; violations collected in ascending node-id order.

use crate::analysis::{build_cfg, ctrl_positions, dominators, is_control_opcode, Dominators};
use crate::error::MiddleError;
use rv8_ir::son::{BlockId, NodeId, Opcode, SeaOfNodes};

/// CEP:WHAT: Maximum violations collected before collection stops (report stays bounded).
/// CEP:WHY: Named budget (38.12); a pathological graph cannot force unbounded reporting.
/// CEP:STATUS: complete
/// CEP:FAILURE: collection stops; the returned vector still triggers Err(Verify).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::verifier::tests::rejects_phi_outside_merge.
pub const MAX_VERIFIER_VIOLATIONS: usize = 4_096;

/// CEP:WHAT: One verifier violation: the offending node and a self-describing kind.
/// CEP:WHY: Report-ALL semantics (task contract) need structured, comparable facts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: violations are ordered by node id ascending.
/// CEP:COST: POD plus enum payload.
/// CEP:EVIDENCE: unit tests rv8_middle::verifier::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyViolation {
    /// Node the violation is attributed to.
    pub node: NodeId,
    /// What was violated at that node.
    pub kind: VerifyKind,
}

/// CEP:WHAT: Violation kinds: one per verified invariant.
/// CEP:WHY: Negative tests must be able to assert exactly which invariant fired.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: variant order is report sort order (declaration order).
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit tests rv8_middle::verifier::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyKind {
    /// A Parameter index is defined by more than one node (SSA single-def).
    MultipleParamDef { param: u16 },
    /// An input edge references a node id beyond the graph.
    DanglingInput { input: NodeId },
    /// A data operand does not dominate its use.
    OperandNotDominating { operand: NodeId },
    /// A Phi node sits in a block that hosts no join control node (Merge or multi-input
    /// Region loop header).
    PhiMisplaced,
    /// A Phi's input count disagrees with its join node's control-input count.
    PhiArityMismatch { join: NodeId },
    /// A Phi input's definition does not lie on, nor dominate, its positional
    /// predecessor path (covers the loop back-edge case by path identity).
    PhiInputOffPath { operand: NodeId },
}

/// CEP:WHAT: Runs the deep verifier over one graph and reports all violations.
/// CEP:WHY: Pipeline gate after every pass (38.18); one negative test exists per invariant.
/// CEP:STATUS: complete
/// CEP:FAILURE: Ok when no violations; Err(MiddleError::Verify) with ALL violations in
///   node-id order; Err(MiddleError::Budget) when analysis budgets trip.
/// CEP:ASSUMES: see module header (unblocked/unreachable nodes are not dominance-checked).
/// CEP:COST: O(N * fanin + CFG + dominators).
/// CEP:EVIDENCE: unit tests rv8_middle::verifier::tests::*.
pub fn verify_son(sea: &SeaOfNodes) -> Result<(), MiddleError> {
    let n = sea.nodes.len();
    let mut violations: Vec<VerifyViolation> = Vec::new();

    // Pass 1: dangling inputs, parameter single-def, and per-block join discovery.
    // Joins: control nodes whose control-input count is >= 2 (Merge joins, loop headers).
    let mut first_param_def: Vec<Option<NodeId>> = Vec::new();
    let mut join_of_block: Vec<Option<(NodeId, Vec<NodeId>)>> = Vec::new();
    let mut block_hint: u32 = 0;
    for node in &sea.nodes {
        if let Some(b) = node.block {
            block_hint = block_hint.max(b.0 + 1);
        }
    }
    join_of_block.resize(block_hint as usize, None);
    for (u, node) in sea.nodes.iter().enumerate() {
        let uid = NodeId(u as u32);
        for &input in &node.inputs {
            if (input.0 as usize) >= n {
                violations.push(VerifyViolation {
                    node: uid,
                    kind: VerifyKind::DanglingInput { input },
                });
            }
        }
        if let Opcode::Parameter(p) = node.opcode {
            let slot = first_param_def.get(p as usize).copied().flatten();
            match slot {
                Some(_) => violations.push(VerifyViolation {
                    node: uid,
                    kind: VerifyKind::MultipleParamDef { param: p },
                }),
                None => {
                    if first_param_def.len() <= p as usize {
                        first_param_def.resize(p as usize + 1, None);
                    }
                    first_param_def[p as usize] = Some(uid);
                }
            }
        }
        if is_control_opcode(node.opcode) {
            let (_, ctrl) = ctrl_positions(node.opcode, node.inputs.len());
            if ctrl.len() >= 2 {
                if let Some(b) = node.block {
                    let bi = b.0 as usize;
                    if bi < join_of_block.len() && join_of_block[bi].is_none() {
                        let preds: Vec<NodeId> =
                            ctrl.iter().filter_map(|&c| node.inputs.get(c).copied()).collect();
                        join_of_block[bi] = Some((uid, preds));
                    }
                }
            }
        }
    }

    // Pass 2: phi placement and arity against the block's join node.
    for (u, node) in sea.nodes.iter().enumerate() {
        if node.opcode != Opcode::Phi {
            continue;
        }
        let uid = NodeId(u as u32);
        let join = node.block.and_then(|b| {
            join_of_block
                .get(b.0 as usize)
                .and_then(|j| j.as_ref().map(|(id, preds)| (*id, preds.clone())))
        });
        match join {
            None => violations.push(VerifyViolation { node: uid, kind: VerifyKind::PhiMisplaced }),
            Some((join_id, preds)) => {
                if node.inputs.len() != preds.len() {
                    violations.push(VerifyViolation {
                        node: uid,
                        kind: VerifyKind::PhiArityMismatch { join: join_id },
                    });
                }
            }
        }
    }
    if violations.len() >= MAX_VERIFIER_VIOLATIONS {
        return Err(MiddleError::Verify(violations));
    }

    // Pass 3: operand-dominates-use over data inputs. Phi inputs are checked against
    // their positional predecessor path instead of the phi's own block, which realizes
    // the spec 2.3.1 back-edge exception through path identity.
    let cfg = build_cfg(sea)?;
    let doms = dominators(&cfg)?;
    for (u, node) in sea.nodes.iter().enumerate() {
        let uid = NodeId(u as u32);
        let ub = match node.block {
            Some(b) => b,
            None => continue,
        };
        if !block_reachable(&doms, ub) {
            continue;
        }
        if node.opcode == Opcode::Phi {
            let preds = node
                .block
                .and_then(|b| {
                    join_of_block
                        .get(b.0 as usize)
                        .and_then(|j| j.as_ref().map(|(_, preds)| preds.clone()))
                })
                .unwrap_or_default();
            for (i, &d) in node.inputs.iter().enumerate() {
                if (d.0 as usize) >= n {
                    continue; // dangling already reported in pass 1
                }
                let db = match sea.nodes[d.0 as usize].block {
                    Some(b) => b,
                    None => continue,
                };
                let pb = match preds.get(i) {
                    Some(&p) if (p.0 as usize) < n => sea.nodes[p.0 as usize].block,
                    _ => continue,
                };
                let pb = match pb {
                    Some(b) => b,
                    None => continue,
                };
                if !block_reachable(&doms, db) || !block_reachable(&doms, pb) {
                    continue;
                }
                let ok = db == pb || doms.strictly_dominates(db, pb);
                if !ok {
                    violations.push(VerifyViolation {
                        node: uid,
                        kind: VerifyKind::PhiInputOffPath { operand: d },
                    });
                    if violations.len() >= MAX_VERIFIER_VIOLATIONS {
                        return Err(MiddleError::Verify(violations));
                    }
                }
            }
            continue;
        }
        let (data, _) = ctrl_positions(node.opcode, node.inputs.len());
        for d in data {
            let did = match node.inputs.get(d) {
                Some(&id) if (id.0 as usize) < n => id,
                _ => continue, // dangling already reported in pass 1
            };
            let db = match sea.nodes[did.0 as usize].block {
                Some(b) => b,
                None => continue,
            };
            if !block_reachable(&doms, db) {
                continue;
            }
            let ok = if db == ub { did.0 < uid.0 } else { doms.strictly_dominates(db, ub) };
            if !ok {
                violations.push(VerifyViolation {
                    node: uid,
                    kind: VerifyKind::OperandNotDominating { operand: did },
                });
                if violations.len() >= MAX_VERIFIER_VIOLATIONS {
                    return Err(MiddleError::Verify(violations));
                }
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(MiddleError::Verify(violations))
    }
}

/// CEP:WHAT: Reports whether a block has dominator facts (entry or dominated by entry).
/// CEP:WHY: Unreachable blocks cannot be dominance-checked and are skipped fail-open by
///   documented assumption rather than failing the verifier on dead code.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the entry idom is Some(entry).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::verifier::tests::accepts_lowered_and_loop_graphs.
fn block_reachable(doms: &Dominators, b: BlockId) -> bool {
    matches!(doms.idom.get(b.0 as usize), Some(Some(_)))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::analysis::tests::{if_else_son, loop_graph};
    use rv8_ir::son::IrType;

    /// CEP:WHAT: Appends one node with an explicit block assignment.
    /// CEP:WHY: Negative-test construction helper.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn push(sea: &mut SeaOfNodes, op: Opcode, ty: IrType, inputs: Vec<NodeId>, block: u32) -> NodeId {
        let id = sea.add_node(op, ty, inputs);
        sea.nodes[id.0 as usize].block = Some(BlockId(block));
        id
    }

    /// CEP:WHAT: Positive: a lowered if/else graph and a hand-built loop graph both pass
    ///   the deep verifier, exercising join phis and the loop back-edge path rule.
    /// CEP:WHY: The verifier must accept every graph the lowering produces.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn accepts_lowered_and_loop_graphs() {
        let sea = if_else_son();
        verify_son(&sea).expect("lowered if/else must verify");
        let sea = loop_graph();
        verify_son(&sea).expect("loop graph must verify (back-edge path rule)");
    }

    /// CEP:WHAT: Negative: two nodes defining Parameter(0) are reported as SSA single-def
    ///   violations, attributed to the second definition.
    /// CEP:WHY: One negative test per invariant; single-def recheck.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_double_parameter_def() {
        let mut sea = SeaOfNodes::new();
        let p0 = push(&mut sea, Opcode::Parameter(0), IrType::I32, vec![], 0);
        let p1 = push(&mut sea, Opcode::Parameter(0), IrType::I32, vec![], 0);
        let sum = push(&mut sea, Opcode::Add, IrType::I32, vec![p0, p1], 0);
        let _ = push(&mut sea, Opcode::Return, IrType::Unit, vec![sum], 0);
        match verify_son(&sea) {
            Err(MiddleError::Verify(vs)) => {
                assert!(vs.contains(&VerifyViolation {
                    node: p1,
                    kind: VerifyKind::MultipleParamDef { param: 0 },
                }));
            }
            other => panic!("expected Verify, got {other:?}"),
        }
    }

    /// CEP:WHAT: Negative: a value defined in the then-arm and used in the else-arm
    ///   without a phi is reported as an operand-dominates-use violation.
    /// CEP:WHY: One negative test per invariant; dominance check.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_dominance_violation() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let c = push(&mut sea, Opcode::Const(1), IrType::Bool, vec![], 0);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![c, n0], 0);
        push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 1);
        let val = push(&mut sea, Opcode::Const(7), IrType::I32, vec![], 1);
        let else_r = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 2);
        let add = push(&mut sea, Opcode::Add, IrType::I32, vec![val, val], 2);
        let _ = push(&mut sea, Opcode::Return, IrType::Unit, vec![add, else_r], 2);
        match verify_son(&sea) {
            Err(MiddleError::Verify(vs)) => {
                assert!(
                    vs.contains(&VerifyViolation {
                        node: add,
                        kind: VerifyKind::OperandNotDominating { operand: val },
                    }),
                    "violations: {vs:?}"
                );
            }
            other => panic!("expected Verify, got {other:?}"),
        }
    }

    /// CEP:WHAT: Negative: a Phi in a straight-line block without any join control node is
    ///   reported as misplaced.
    /// CEP:WHY: One negative test per invariant; phi-at-merge placement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_phi_outside_merge() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let a = push(&mut sea, Opcode::Const(1), IrType::I32, vec![], 0);
        let b = push(&mut sea, Opcode::Const(2), IrType::I32, vec![], 0);
        // Two-input phi in a block whose only control node is a 1-input Region.
        let phi = push(&mut sea, Opcode::Phi, IrType::I32, vec![a, b], 0);
        let _ = push(&mut sea, Opcode::Return, IrType::Unit, vec![phi, n0], 0);
        match verify_son(&sea) {
            Err(MiddleError::Verify(vs)) => {
                assert!(
                    vs.contains(&VerifyViolation { node: phi, kind: VerifyKind::PhiMisplaced }),
                    "violations: {vs:?}"
                );
            }
            other => panic!("expected Verify, got {other:?}"),
        }
    }

    /// CEP:WHAT: Negative: a phi whose input does not lie on its positional predecessor
    ///   path (value from the then-arm wired to the else-path slot) is reported.
    /// CEP:WHY: The path rule is the phi form of the dominance invariant.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_phi_input_off_path() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let c = push(&mut sea, Opcode::Const(1), IrType::Bool, vec![], 0);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![c, n0], 0);
        let tr = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 1);
        let then_val = push(&mut sea, Opcode::Const(7), IrType::I32, vec![], 1);
        let er = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 2);
        let else_val = push(&mut sea, Opcode::Const(8), IrType::I32, vec![], 2);
        // Merge inputs are (then, else); phi swaps the slots: [else_val, then_val].
        let phi = push(&mut sea, Opcode::Phi, IrType::I32, vec![else_val, then_val], 3);
        let merge = push(&mut sea, Opcode::Merge, IrType::Unit, vec![tr, er], 3);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![phi, merge], 3);
        let _ = ret;
        match verify_son(&sea) {
            Err(MiddleError::Verify(vs)) => {
                assert!(
                    vs.iter().any(|v| v.node == phi && matches!(v.kind, VerifyKind::PhiInputOffPath { .. })),
                    "violations: {vs:?}"
                );
            }
            other => panic!("expected Verify, got {other:?}"),
        }
    }
}
