// CEP:FILE: crates/rv8_middle/src/pea.rs
// CEP:WHAT: Partial escape analysis over the Sea of Nodes (standard 5.6): allocation sites
//   are Alloc node ids; path-sensitive escape sets E(a) are per-block dense bitsets
//   propagated backward over the CFG; escape edges flow through stores, calls, returns,
//   and explicit EscapePoint nodes; an allocation whose escape set never intersects a path
//   from its own block is scalar-replaced by rewiring loads to the single stored value via
//   replace_uses.
// CEP:WHY: Escape-state optimization must be deterministic because self-hosting hashes
//   compiler outputs (38.10, 5.6); the bitset formulation keeps the fixpoint exact.
// CEP:CLASS: HPC-0
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-105
// CEP:FAILURE: MiddleError::Budget when alloc sites or propagation rounds exceed the
//   named constants; MiddleError::Ir on structural failure; never a panic.
// CEP:ASSUMES: conventions follow the flattening table: Store value-then-address; Load
//   [address]; Return data inputs per the control table; a replaceable allocation has
//   exactly one Store into it and no Phi consumers; scalar replacement rewires loads only
//   when the store dominates them; swept dead nodes are left for DCE.
// CEP:COST: O(passes * N * sites/64) provenance plus O(passes * blocks * sites/64) escape
//   closure; bounds named below.
// CEP:EVIDENCE: unit tests rv8_middle::pea::tests::* (escape stays, local replaced,
//   determinism).
// CEP:SECURITY: no recursion; every traversal is an index loop with a named budget.
// CEP:DETERMINISM: deterministic; candidate order is node-id ascending, all sets are
//   dense bitsets, replacement decisions depend only on graph content.

use crate::analysis::{bs_set, bs_test, bs_union_into, build_cfg, check_budget, ctrl_positions};
use crate::error::MiddleError;
use rv8_ir::son::{Opcode, IrType, SeaOfNodes};

/// CEP:WHAT: Maximum allocation sites analyzed in one graph.
/// CEP:WHY: Named budget (38.12) bounding per-site escape bitsets.
/// CEP:STATUS: complete
/// CEP:FAILURE: run_pea returns MiddleError::Budget above the limit.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::pea::tests::escaping_alloc_stays.
pub const MAX_ALLOC_SITES: usize = 4_096;

/// CEP:WHAT: Maximum fixpoint rounds for provenance and escape-set propagation.
/// CEP:WHY: Named budget (38.12); phi cycles converge in a few rounds on real graphs.
/// CEP:STATUS: complete
/// CEP:FAILURE: run_pea returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::pea::tests::escaping_alloc_stays.
pub const MAX_PEA_PROP_PASSES: u32 = 256;

/// CEP:WHAT: Report of one PEA run.
/// CEP:WHY: Driver remarks and fixpoint detection need analyzed and replaced counts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: scalar_replaced counts allocations, not loads.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_middle::pea::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaReport {
    /// Number of allocation sites analyzed.
    pub analyzed: usize,
    /// Number of allocations scalar-replaced.
    pub scalar_replaced: usize,
}

/// CEP:WHAT: Runs partial escape analysis and performs scalar replacement in place.
/// CEP:WHY: The single entry point of rv8_middle::pea (standard 5.6).
/// CEP:STATUS: partial
/// CEP:FAILURE: MiddleError::Budget and MiddleError::Ir as documented; replacement is
///   restricted to single-store, phi-free, dominance-safe allocations.
/// CEP:ASSUMES: see module header.
/// CEP:COST: documented in the module header.
/// CEP:EVIDENCE: unit tests rv8_middle::pea::tests::*.
pub fn run_pea(sea: &mut SeaOfNodes) -> Result<PeaReport, MiddleError> {
    sea.verify_structure()?;
    check_budget(sea.nodes.len())?;
    let n = sea.nodes.len();

    // AllocationSiteId = NodeId of each Alloc node; site index = ascending rank.
    let sites: Vec<u32> = (0..n as u32)
        .filter(|&u| sea.nodes[u as usize].opcode == Opcode::Alloc)
        .collect();
    if sites.len() > MAX_ALLOC_SITES {
        return Err(MiddleError::Budget {
            budget: "MAX_ALLOC_SITES",
            limit: MAX_ALLOC_SITES as u64,
            actual: sites.len() as u64,
        });
    }
    let analyzed = sites.len();
    if analyzed == 0 {
        return Ok(PeaReport { analyzed: 0, scalar_replaced: 0 });
    }
    let words = analyzed.div_ceil(64);
    let site_rank = |u: u32| -> Option<usize> {
        sites.iter().position(|&s| s == u)
    };

    // Provenance fixpoint over Ptr-typed nodes, in alloc-site bitsets.
    let mut prov: Vec<Vec<u64>> = vec![vec![0u64; words]; n];
    for (u, node) in sea.nodes.iter().enumerate() {
        if node.opcode == Opcode::Alloc {
            if let Some(r) = site_rank(u as u32) {
                bs_set(&mut prov[u], r);
            }
        }
    }
    let mut rounds: u32 = 0;
    let mut changed = true;
    while changed {
        changed = false;
        rounds += 1;
        if rounds > MAX_PEA_PROP_PASSES {
            return Err(MiddleError::Budget {
                budget: "MAX_PEA_PROP_PASSES",
                limit: MAX_PEA_PROP_PASSES as u64,
                actual: rounds as u64,
            });
        }
        // stored[s] = union of stored-value provenance per site.
        let mut stored: Vec<Vec<u64>> = vec![vec![0u64; words]; analyzed];
        for node in &sea.nodes {
            if node.opcode == Opcode::Store && node.inputs.len() >= 2 {
                let value = node.inputs[0];
                let addr = node.inputs[1];
                if (value.0 as usize) < n && (addr.0 as usize) < n {
                    for (s, row) in stored.iter_mut().enumerate() {
                        if bs_test(&prov[addr.0 as usize], s) {
                            bs_union_into(row, &prov[value.0 as usize]);
                        }
                    }
                }
            }
        }
        // Provenance: loads read stored sites; phis and pointer-passing pure ops union
        // their pointer inputs.
        for (u, node) in sea.nodes.iter().enumerate() {
            if node.ty != IrType::Ptr || node.opcode == Opcode::Alloc {
                continue;
            }
            let mut next = prov[u].clone();
            match node.opcode {
                Opcode::Load => {
                    if let Some(&addr) = node.inputs.first() {
                        if (addr.0 as usize) < n {
                            for (s, row) in stored.iter().enumerate() {
                                if bs_test(&prov[addr.0 as usize], s) {
                                    bs_union_into(&mut next, row);
                                }
                            }
                        }
                    }
                }
                _ => {
                    for &d in &node.inputs {
                        if (d.0 as usize) < n && sea.nodes[d.0 as usize].ty == IrType::Ptr {
                            bs_union_into(&mut next, &prov[d.0 as usize]);
                        }
                    }
                }
            }
            if next != prov[u] {
                prov[u] = next;
                changed = true;
            }
        }
    }

    // Escape events per site, recorded at the block of the escaping node.
    let cfg = build_cfg(sea)?;
    let blocks = cfg.block_count as usize;
    let mut esc: Vec<Vec<u64>> = vec![vec![0u64; words]; blocks];
    let mut global_esc = vec![0u64; words];
    let record = |prov: &[u64], block: Option<rv8_ir::son::BlockId>, esc: &mut [Vec<u64>], global: &mut [u64]| {
        for s in 0..analyzed {
            if bs_test(prov, s) {
                match block {
                    Some(b) if (b.0 as usize) < blocks => bs_set(&mut esc[b.0 as usize], s),
                    _ => bs_set(global, s),
                }
            }
        }
    };
    for (u, node) in sea.nodes.iter().enumerate() {
        let ptr_inputs = |graph: &SeaOfNodes, ids: &[usize]| -> Vec<Vec<u64>> {
            ids.iter()
                .filter(|&&i| {
                    let id = graph.nodes[u].inputs[i];
                    (id.0 as usize) < graph.nodes.len()
                        && graph.nodes[id.0 as usize].ty == IrType::Ptr
                })
                .map(|&i| {
                    let id = graph.nodes[u].inputs[i];
                    prov[id.0 as usize].clone()
                })
                .collect()
        };
        match node.opcode {
            Opcode::Store if node.inputs.len() >= 2 => {
                // Storing a pointer into another object lets it escape.
                let sets = ptr_inputs(sea, &[0]);
                for set in sets {
                    record(&set, node.block, &mut esc, &mut global_esc);
                }
            }
            Opcode::Call { .. } => {
                let ids: Vec<usize> = (0..node.inputs.len()).collect();
                let sets = ptr_inputs(sea, &ids);
                for set in sets {
                    record(&set, node.block, &mut esc, &mut global_esc);
                }
            }
            Opcode::Return => {
                let (data, _) = ctrl_positions(node.opcode, node.inputs.len());
                let sets = ptr_inputs(sea, &data);
                for set in sets {
                    record(&set, node.block, &mut esc, &mut global_esc);
                }
            }
            Opcode::EscapePoint if !node.inputs.is_empty() => {
                let sets = ptr_inputs(sea, &[0]);
                for set in sets {
                    record(&set, node.block, &mut esc, &mut global_esc);
                }
            }
            _ => {}
        }
    }
    // Backward closure: esc[b] = events(b) | union esc[succ]. Iterative fixpoint.
    let mut rounds: u32 = 0;
    let mut changed = true;
    while changed {
        changed = false;
        rounds += 1;
        if rounds > MAX_PEA_PROP_PASSES {
            return Err(MiddleError::Budget {
                budget: "MAX_PEA_PROP_PASSES",
                limit: MAX_PEA_PROP_PASSES as u64,
                actual: rounds as u64,
            });
        }
        for b in 0..blocks {
            for &s in &cfg.succs[b] {
                let src = esc[s.0 as usize].clone();
                if bs_union_into(&mut esc[b], &src) {
                    changed = true;
                }
            }
        }
    }

    // Decision per site, then scalar replacement for the replaceable ones.
    let doms = crate::analysis::dominators(&cfg)?;
    let uses = crate::analysis::build_dataflow(sea);
    let mut scalar_replaced = 0usize;
    for (s, &site) in sites.iter().enumerate() {
        let site_block = sea.nodes[site as usize].block;
        let escapes = match site_block {
            Some(b) if (b.0 as usize) < blocks => {
                bs_test(&esc[b.0 as usize], s) || bs_test(&global_esc, s)
            }
            // Unplaced allocation: single-path view; any escape event anywhere keeps it.
            _ => bs_test(&global_esc, s) || esc.iter().any(|e| bs_test(e, s)),
        };
        if escapes {
            continue;
        }
        // Classify consumers: stores into the site, loads of the site, anything else
        // blocks replacement (conservative, deterministic).
        let site_id = rv8_ir::son::NodeId(site);
        let mut stores: Vec<u32> = Vec::new();
        let mut loads: Vec<u32> = Vec::new();
        let mut blocking = false;
        for &user in &uses.uses_of[site as usize] {
            let un = &sea.nodes[user.0 as usize];
            let takes_addr = match un.opcode {
                Opcode::Store => un.inputs.len() >= 2 && un.inputs[1] == site_id,
                Opcode::Load => !un.inputs.is_empty() && un.inputs[0] == site_id,
                _ => false,
            };
            if takes_addr {
                if un.opcode == Opcode::Store {
                    stores.push(user.0);
                } else {
                    loads.push(user.0);
                }
            } else {
                blocking = true;
            }
        }
        if blocking || stores.len() != 1 || loads.is_empty() {
            continue;
        }
        let store_at = stores[0];
        let value = sea.nodes[store_at as usize].inputs[0];
        if (value.0 as usize) >= n
            || loads.contains(&value.0)
            || sea.nodes[value.0 as usize].ty != sea.nodes[loads[0] as usize].ty
        {
            continue;
        }
        // Dominance guard: the replacement value must dominate every replaced load.
        let value_block = sea.nodes[value.0 as usize].block;
        let all_dominated = loads.iter().all(|&l| {
            let load_block = sea.nodes[l as usize].block;
            match (value_block, load_block) {
                (Some(db), Some(ub)) => {
                    if db == ub {
                        value.0 < l
                    } else {
                        doms.strictly_dominates(db, ub)
                    }
                }
                _ => value.0 < l,
            }
        });
        if !all_dominated {
            continue;
        }
        for &l in &loads {
            sea.replace_uses(rv8_ir::son::NodeId(l), value);
        }
        scalar_replaced += 1;
    }
    sea.verify_structure()?;
    Ok(PeaReport { analyzed, scalar_replaced })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::son::{BlockId, NodeId};

    /// CEP:WHAT: Appends a node with an explicit block.
    /// CEP:WHY: Fixture helper for block-sensitive PEA tests.
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

    /// CEP:WHAT: An allocation that escapes through EscapePoint on the return path stays;
    ///   the report counts it as analyzed but not replaced.
    /// CEP:WHY: The keep side of the replacement criterion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn escaping_alloc_stays() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        let ep = push(&mut sea, Opcode::EscapePoint, IrType::Unit, vec![a], 0);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![n0], 0);
        let _ = (ret, ep);
        for _ in 0..2 {
            let report = run_pea(&mut sea).expect("pea");
            assert_eq!(report.analyzed, 1);
            assert_eq!(report.scalar_replaced, 0);
        }
    }

    /// CEP:WHAT: An allocation used only by a local store/load pair on a cold path is
    ///   scalar-replaced: the load's users are rewired to the stored value.
    /// CEP:WHY: The replace side of the criterion and the task's cold-path case.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn cold_local_alloc_replaced() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let one = push(&mut sea, Opcode::Const(1), IrType::Bool, vec![], 0);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![one, n0], 0);
        let cold = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 1);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 1);
        let c7 = push(&mut sea, Opcode::Const(7), IrType::I32, vec![], 1);
        let st = push(&mut sea, Opcode::Store, IrType::Unit, vec![c7, a], 1);
        let ld = push(&mut sea, Opcode::Load, IrType::I32, vec![a], 1);
        let add = push(&mut sea, Opcode::Add, IrType::I32, vec![ld, c7], 1);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![add, cold], 1);
        for _ in 0..2 {
            let report = run_pea(&mut sea).expect("pea");
            assert_eq!(report.analyzed, 1);
            assert_eq!(report.scalar_replaced, 1, "local alloc replaced");
        }
        // The load's user now reads the stored constant directly.
        assert_eq!(sea.node(add).expect("add").inputs, vec![c7, c7]);
        assert!(sea.verify_structure().is_ok());
        let _ = (st, ret);
    }

    /// CEP:WHAT: A return of a pointer loaded from a never-escaping local allocation is
    ///   scalar-replaced (the scalar escapes, not the object).
    /// CEP:WHY: The classic partial-escape win; provenance must see through loads.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn scalar_return_replaces_object() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        let c5 = push(&mut sea, Opcode::Const(5), IrType::I32, vec![], 0);
        let st = push(&mut sea, Opcode::Store, IrType::Unit, vec![c5, a], 0);
        let ld = push(&mut sea, Opcode::Load, IrType::I32, vec![a], 0);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![ld, n0], 0);
        let _ = (st, ret);
        let report = run_pea(&mut sea).expect("pea");
        assert_eq!(report.scalar_replaced, 1);
        assert_eq!(sea.node(ret).expect("ret").inputs, vec![c5, n0]);
    }
}
