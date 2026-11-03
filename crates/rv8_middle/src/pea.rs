// CEP:FILE: crates/rv8_middle/src/pea.rs
// CEP:WHAT: Partial escape analysis over the Sea of Nodes (standard 5.6): allocation sites
//   are Alloc node ids; path-sensitive escape sets E(a) are per-block dense bitsets
//   propagated backward over the CFG; escape edges flow through stores, calls, returns,
//   and explicit EscapePoint nodes; an allocation whose escape set never intersects a path
//   from its own block is scalar-replaced by rewiring loads to the single stored value via
//   replace_uses; an allocation that escapes only on SOME paths gets the design "Phase 3:
//   materialization on cold paths" transform - the Alloc and its field Store re-block to
//   the common dominator of the escape events (the divergence point), a Materialize
//   marker node is appended there, and hot-path loads that no escape event can have
//   preceded are rewired to the scalar, so the non-escaping prefix performs no heap
//   allocation at all.
// CEP:WHY: Escape-state optimization must be deterministic because self-hosting hashes
//   compiler outputs (38.10, 5.6); the bitset formulation keeps the fixpoint exact, and
//   the materialization transform is the defining partial-escape win of design.md 1969.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-105
// CEP:FAILURE: MiddleError::Budget when alloc sites or propagation rounds exceed the
//   named constants; MiddleError::Ir on structural failure; a final verify_son runs so a
//   transformed graph that violates any IR invariant fails closed instead of lowering;
//   never a panic.
// CEP:ASSUMES: conventions follow the flattening table: Store value-then-address; Load
//   [address]; Return data inputs per the control table; a replaceable allocation has
//   exactly one Store into it and no Phi consumers; scalar replacement rewires loads only
//   when the store dominates them; the materialization transform additionally requires a
//   single divergence block (the common dominator of every escape event) that the stored
//   value dominates, aborts conservatively on Phi or unknown consumers, and never fires
//   when the divergence block is the allocation's own block (no hot-path win). The
//   design-level PEA applications beyond this IR contract (macro-expansion arenas,
//   AOT-for-kernels shared-memory promotion) remain separately ticketed work and are not
//   claimed here.
// CEP:COST: O(passes * N * sites/64) provenance plus O(passes * blocks * sites/64) escape
//   closure plus O(events * depth) dominance walks; bounds named below.
// CEP:EVIDENCE: unit tests rv8_middle::pea::tests::* (escape stays, local replaced,
//   scalar return, partial-escape materialization, full-escape skip, phi abort, both
//   budgets) and the pipeline-level wiring test in rv8_driver::queries::tests.
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
/// CEP:WHY: Driver remarks and fixpoint detection need analyzed, replaced, and
///   materialized counts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: scalar_replaced counts allocations, not loads; materialized counts
///   allocations that received the Phase-3 cold-path materialization transform.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_middle::pea::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeaReport {
    /// Number of allocation sites analyzed.
    pub analyzed: usize,
    /// Number of allocations scalar-replaced (fully or on the non-escaping prefix).
    pub scalar_replaced: usize,
    /// Number of allocations that escaped somewhere and were materialized at the cold
    /// divergence point instead (design.md 1969-1976).
    pub materialized: usize,
}

/// CEP:WHAT: Walks the immediate-dominator chains to the deepest block that dominates
///   (or equals) every block in `ev_blocks` - the cold-path divergence point of design
///   1969.
/// CEP:WHY: The materialization transform needs ONE deterministic point that every
///   escape event flows through.
/// CEP:STATUS: complete
/// CEP:FAILURE: None (unreachable event block or budget) - the caller keeps the heap
///   object in that case (fail closed, conservative).
/// CEP:ASSUMES: ev_blocks is non-empty and in ascending block order.
/// CEP:COST: O(events * tree depth), bounded by MAX_DOMINANCE_WALK_STEPS.
/// CEP:EVIDENCE: the partial-escape materialization tests.
fn common_dominator(
    doms: &crate::analysis::Dominators,
    ev_blocks: &[usize],
) -> Option<rv8_ir::son::BlockId> {
    let mut cand = rv8_ir::son::BlockId(ev_blocks[0] as u32);
    let mut steps: u32 = 0;
    loop {
        let covers = ev_blocks.iter().all(|&b| {
            let bid = rv8_ir::son::BlockId(b as u32);
            bid == cand || doms.strictly_dominates(cand, bid)
        });
        if covers {
            return Some(cand);
        }
        match doms.idom.get(cand.0 as usize).copied().flatten() {
            Some(p) if p != cand => cand = p,
            _ => return None,
        }
        steps += 1;
        if steps > crate::analysis::MAX_DOMINANCE_WALK_STEPS {
            return None;
        }
    }
}

/// CEP:WHAT: Runs partial escape analysis and performs scalar replacement and cold-path
///   materialization in place.
/// CEP:WHY: The single entry point of rv8_middle::pea (standard 5.6).
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget and MiddleError::Ir as documented; replacement is
///   restricted to single-store, phi-free, dominance-safe allocations; materialization
///   additionally requires a single divergence block that the stored value dominates.
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
        return Ok(PeaReport { analyzed: 0, scalar_replaced: 0, materialized: 0 });
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

    // Direct escape events per site, recorded at the block of the escaping node.
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
    // Snapshot of the direct per-block events, taken before the backward closure:
    // evt[b] holds exactly the sites with an escape event located in block b. The
    // materialization transform needs the raw event locations (design 1969: the
    // divergence point is the common dominator of the escape events).
    let evt = esc.clone();
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
    // Forward closure of the direct events: fwd[b] says whether an escape event for a
    // site can have executed on SOME path reaching b. A load may be rewired to the
    // scalar only when no escape event (which may mutate the object through the leaked
    // pointer) can precede it.
    let mut fwd: Vec<Vec<u64>> = evt.clone();
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
            for &p in &cfg.preds[b] {
                let src = fwd[p.0 as usize].clone();
                if bs_union_into(&mut fwd[b], &src) {
                    changed = true;
                }
            }
        }
    }

    // Decision per site: full scalar replacement when nothing escapes; the Phase 3
    // materialization transform (design 1969-1976) when the site escapes on some paths
    // only; keep the heap object otherwise.
    let doms = crate::analysis::dominators(&cfg)?;
    let uses = crate::analysis::build_dataflow(sea);
    let mut scalar_replaced = 0usize;
    let mut materialized = 0usize;
    for (s, &site) in sites.iter().enumerate() {
        let site_block = sea.nodes[site as usize].block;
        let escapes = match site_block {
            Some(b) if (b.0 as usize) < blocks => {
                bs_test(&esc[b.0 as usize], s) || bs_test(&global_esc, s)
            }
            // Unplaced allocation: single-path view; any escape event anywhere keeps it.
            _ => bs_test(&global_esc, s) || esc.iter().any(|e| bs_test(e, s)),
        };
        // Classify consumers: the single field Store (address position), address-taken
        // Loads, escape observers (a pointer input in value position IS the escape
        // event: EscapePoint, calls, returns, stores of the pointer), and anything else
        // blocks the transform (conservative, deterministic).
        let site_id = rv8_ir::son::NodeId(site);
        let mut stores: Vec<u32> = Vec::new();
        let mut loads: Vec<u32> = Vec::new();
        let mut blocking = false;
        for &user in &uses.uses_of[site as usize] {
            let un = &sea.nodes[user.0 as usize];
            let kind = match un.opcode {
                Opcode::Store if un.inputs.len() >= 2 && un.inputs[1] == site_id => 0,
                Opcode::Load if !un.inputs.is_empty() && un.inputs[0] == site_id => 1,
                Opcode::Store if !un.inputs.is_empty() && un.inputs[0] == site_id => 2,
                Opcode::EscapePoint if !un.inputs.is_empty() && un.inputs[0] == site_id => 2,
                Opcode::Call { .. } | Opcode::Return if un.inputs.contains(&site_id) => 2,
                _ => 3,
            };
            match kind {
                0 => stores.push(user.0),
                1 => loads.push(user.0),
                // Escape observer: keeps reading the (possibly materialized) object; its
                // block is an event block, so it is always dominated by the divergence
                // point computed below.
                2 => {}
                _ => blocking = true,
            }
        }
        if blocking {
            continue;
        }
        if !escapes {
            // ---- Full scalar replacement: no escape event anywhere. ----
            if stores.len() != 1 || loads.is_empty() {
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
            continue;
        }
        // ---- Phase 3: materialization on cold paths (design 1969-1976). ----
        // Requires exactly one field store; multi-field shapes keep the heap object.
        if stores.len() != 1 {
            continue;
        }
        let store_at = stores[0];
        let value = sea.nodes[store_at as usize].inputs[0];
        if (value.0 as usize) >= n {
            continue;
        }
        // The divergence point is the common dominator of every escape event.
        let ev_blocks: Vec<usize> = (0..blocks).filter(|&b| bs_test(&evt[b], s)).collect();
        if ev_blocks.is_empty() {
            // Unplaced or global-only escape: keep the heap object.
            continue;
        }
        let c = match common_dominator(&doms, &ev_blocks) {
            Some(c) => c,
            None => continue,
        };
        // No hot-path win when the divergence point is the allocation's own block.
        if site_block == Some(c) {
            continue;
        }
        // The stored value must be available at the divergence point.
        let value_block = sea.nodes[value.0 as usize].block;
        let value_ok = match value_block {
            Some(vb) => vb == c || doms.strictly_dominates(vb, c),
            None => false,
        };
        if !value_ok {
            continue;
        }
        // Loads: rewire the ones no escape event can have preceded (the leaked pointer
        // cannot have been mutated before them); keep the ones at or past the divergence
        // point (they read the materialized object); abort conservatively otherwise.
        // Rewires are independently sound (they only demand value dominance), so an
        // abort after some rewires keeps those rewires and skips the re-blocking.
        let mut rewired = false;
        let mut abort = false;
        let mut rewired_loads: Vec<u32> = Vec::new();
        for &l in &loads {
            let load_block = sea.nodes[l as usize].block;
            let no_prior_event = match load_block {
                Some(b) if (b.0 as usize) < blocks => !bs_test(&fwd[b.0 as usize], s),
                _ => false,
            };
            let type_ok = sea.nodes[value.0 as usize].ty == sea.nodes[l as usize].ty;
            let value_dominates = match (value_block, load_block) {
                (Some(db), Some(ub)) => {
                    if db == ub {
                        value.0 < l
                    } else {
                        doms.strictly_dominates(db, ub)
                    }
                }
                _ => value.0 < l,
            };
            if no_prior_event && type_ok && value_dominates {
                sea.replace_uses(rv8_ir::son::NodeId(l), value);
                rewired = true;
                // The load itself is now dead (its users read the scalar) but still
                // references the allocation; it moves to the divergence block together
                // with the object so every reference stays dominance-correct until DCE
                // sweeps it.
                rewired_loads.push(l);
                continue;
            }
            let under_divergence = match load_block {
                Some(ub) => ub == c || doms.strictly_dominates(c, ub),
                None => false,
            };
            if !under_divergence {
                abort = true;
                break;
            }
        }
        if abort {
            continue;
        }
        // Move the allocation and its field store to the divergence point and mark the
        // materialization with the barrier node (borrowck treats Materialize like
        // EscapePoint: object pointer in input 0; DCE keeps both; son_to_hetir maps it
        // to MetaOp::Materialize).
        sea.nodes[site as usize].block = Some(c);
        sea.nodes[store_at as usize].block = Some(c);
        for &l in &rewired_loads {
            sea.nodes[l as usize].block = Some(c);
        }
        let mid = sea.add_node(Opcode::Materialize, IrType::Unit, vec![site_id]);
        sea.nodes[mid.0 as usize].block = Some(c);
        materialized += 1;
        if rewired {
            scalar_replaced += 1;
        }
    }
    sea.verify_structure()?;
    crate::verifier::verify_son(sea)?;
    Ok(PeaReport { analyzed, scalar_replaced, materialized })
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
        assert_eq!(report.materialized, 0);
        assert_eq!(sea.node(ret).expect("ret").inputs, vec![c5, n0]);
    }

    /// CEP:WHAT: The Phase 3 materialization: an allocation that escapes only in a block
    ///   two levels below its own re-blocks (with its field store) to the common
    ///   dominator of the escape events, gains a Materialize marker there, and the
    ///   hot-path load before the divergence reads the scalar; the hot exit path never
    ///   executes the Alloc.
    /// CEP:WHY: design.md 1969-1976 is the defining partial-escape transform.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn partial_escape_materializes_on_cold_path() {
        let mut sea = SeaOfNodes::new();
        // Block 0: entry region.
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        // Block 1: allocation, field store, hot load, branch.
        let r1 = push(&mut sea, Opcode::Region, IrType::Unit, vec![n0], 1);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 1);
        let c7 = push(&mut sea, Opcode::Const(7), IrType::I32, vec![], 1);
        let st = push(&mut sea, Opcode::Store, IrType::Unit, vec![c7, a], 1);
        let ld = push(&mut sea, Opcode::Load, IrType::I32, vec![a], 1);
        let ad = push(&mut sea, Opcode::Add, IrType::I32, vec![ld, c7], 1);
        let one = push(&mut sea, Opcode::Const(1), IrType::Bool, vec![], 1);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![one, r1], 1);
        // Block 2: pass-through toward the cold arm (keeps the divergence one level
        // below the allocation block so the transform has a win to make).
        let r2 = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 2);
        let r3 = push(&mut sea, Opcode::Region, IrType::Unit, vec![r2], 3);
        // Block 3: the only escape event.
        let ep = push(&mut sea, Opcode::EscapePoint, IrType::Unit, vec![a], 3);
        let ret_cold = push(&mut sea, Opcode::Return, IrType::Unit, vec![r3], 3);
        // Block 4: hot exit, uses the loaded value.
        let r4 = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 4);
        let ret_hot = push(&mut sea, Opcode::Return, IrType::Unit, vec![ad, r4], 4);
        let _ = (st, ep, ret_cold, ret_hot);

        let first = run_pea(&mut sea).expect("pea");
        assert_eq!(first.analyzed, 1);
        assert_eq!(first.materialized, 1, "partial escape is materialized");
        assert_eq!(first.scalar_replaced, 1, "the hot load is rewired");
        // The allocation and its store moved to the divergence block: with a single
        // event block, the common dominator is that block itself (3), and moving the
        // object there still takes it off the hot exit path 1 -> 4.
        assert_eq!(sea.node(a).expect("alloc").block, Some(BlockId(3)));
        let materialize = sea
            .nodes
            .iter()
            .find(|n| n.opcode == Opcode::Materialize)
            .expect("materialize node inserted");
        assert_eq!(materialize.block, Some(BlockId(3)));
        assert_eq!(materialize.inputs, vec![a]);
        // The hot load reads the scalar now.
        assert_eq!(sea.node(ad).expect("add").inputs, vec![c7, c7]);
        assert!(sea.verify_structure().is_ok());

        // Determinism: rebuilding the graph and re-running produces the same report and
        // the same canonical hash.
        let h1 = sea.canonical_hash();
        let mut sea2 = SeaOfNodes::new();
        let n0 = push(&mut sea2, Opcode::Region, IrType::Unit, vec![], 0);
        let r1 = push(&mut sea2, Opcode::Region, IrType::Unit, vec![n0], 1);
        let a2 = push(&mut sea2, Opcode::Alloc, IrType::Ptr, vec![], 1);
        let c7b = push(&mut sea2, Opcode::Const(7), IrType::I32, vec![], 1);
        let st2 = push(&mut sea2, Opcode::Store, IrType::Unit, vec![c7b, a2], 1);
        let ld2 = push(&mut sea2, Opcode::Load, IrType::I32, vec![a2], 1);
        let ad2 = push(&mut sea2, Opcode::Add, IrType::I32, vec![ld2, c7b], 1);
        let one2 = push(&mut sea2, Opcode::Const(1), IrType::Bool, vec![], 1);
        let br2 = push(&mut sea2, Opcode::Branch, IrType::Unit, vec![one2, r1], 1);
        let r2b = push(&mut sea2, Opcode::Region, IrType::Unit, vec![br2], 2);
        let r3b = push(&mut sea2, Opcode::Region, IrType::Unit, vec![r2b], 3);
        let ep2 = push(&mut sea2, Opcode::EscapePoint, IrType::Unit, vec![a2], 3);
        let rc2 = push(&mut sea2, Opcode::Return, IrType::Unit, vec![r3b], 3);
        let r4b = push(&mut sea2, Opcode::Region, IrType::Unit, vec![br2], 4);
        let rh2 = push(&mut sea2, Opcode::Return, IrType::Unit, vec![ad2, r4b], 4);
        let _ = (st2, ep2, rc2, rh2);
        let second = run_pea(&mut sea2).expect("pea 2");
        assert_eq!(first, second);
        assert_eq!(h1, sea2.canonical_hash());
    }

    /// CEP:WHAT: An escape event in the allocation's own block offers no divergence win,
    ///   so the heap object is kept exactly as before and no Materialize appears.
    /// CEP:WHY: The transform must not churn graphs it cannot improve.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn full_escape_in_alloc_block_keeps_heap_object() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        let ep = push(&mut sea, Opcode::EscapePoint, IrType::Unit, vec![a], 0);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![n0], 0);
        let _ = (ep, ret);
        let report = run_pea(&mut sea).expect("pea");
        assert_eq!(report.analyzed, 1);
        assert_eq!(report.scalar_replaced, 0);
        assert_eq!(report.materialized, 0, "no divergence win, no transform");
        assert_eq!(sea.node(a).expect("alloc").block, Some(BlockId(0)));
        assert!(sea.nodes.iter().all(|n| n.opcode != Opcode::Materialize));
    }

    /// CEP:WHAT: A Phi consumer of the pointer blocks the materialization transform
    ///   (conservative abort; the heap object is kept).
    /// CEP:WHY: Phi merges are outside the single-divergence contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn phi_consumer_blocks_transform() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let one = push(&mut sea, Opcode::Const(1), IrType::Bool, vec![], 0);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![one, n0], 0);
        // Arm 1 holds the allocation; arm 2 holds a second pointer; block 3 merges
        // both arms and hosts the phi (one input per predecessor, each defined on its
        // own path).
        let r1 = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 1);
        let a = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 1);
        let r2 = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 2);
        let a2 = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 2);
        let r3 = push(&mut sea, Opcode::Region, IrType::Unit, vec![r1, r2], 3);
        let ph = push(&mut sea, Opcode::Phi, IrType::Ptr, vec![a, a2], 3);
        let ep = push(&mut sea, Opcode::EscapePoint, IrType::Unit, vec![ph], 3);
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![r3], 3);
        let _ = (ph, ep, ret);
        let report = run_pea(&mut sea).expect("pea");
        assert_eq!(report.materialized, 0, "phi blocks the transform");
        assert_eq!(sea.node(a).expect("alloc").block, Some(BlockId(1)));
    }

    /// CEP:WHAT: More allocation sites than MAX_ALLOC_SITES fail closed with the named
    ///   budget error.
    /// CEP:WHY: The site bitset width must never grow unbounded.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn alloc_site_budget_trips() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let _ = n0;
        for _ in 0..(MAX_ALLOC_SITES + 1) {
            push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        }
        match run_pea(&mut sea) {
            Err(MiddleError::Budget { .. }) => {}
            other => panic!("expected Budget, got {other:?}"),
        }
    }

    /// CEP:WHAT: A provenance chain longer than MAX_PEA_PROP_PASSES fails closed with
    ///   the named budget error (the fixpoint converges one hop per round).
    /// CEP:WHY: The propagation round budget is the fail-closed bound for phi cycles.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn propagation_budget_trips() {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let seed = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        let mut prev_alloc = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
        // Seed the head: the seed pointer is stored into the first link, then each link
        // loads the previous link and stores that value into the next. Provenance
        // advances exactly one hop per fixpoint round, so a chain longer than the round
        // budget must fail closed.
        push(&mut sea, Opcode::Store, IrType::Unit, vec![seed, prev_alloc], 0);
        for _ in 0..(MAX_PEA_PROP_PASSES as usize + 64) {
            let ld = push(&mut sea, Opcode::Load, IrType::Ptr, vec![prev_alloc], 0);
            let next = push(&mut sea, Opcode::Alloc, IrType::Ptr, vec![], 0);
            push(&mut sea, Opcode::Store, IrType::Unit, vec![ld, next], 0);
            prev_alloc = next;
        }
        let ret = push(&mut sea, Opcode::Return, IrType::Unit, vec![n0], 0);
        let _ = ret;
        match run_pea(&mut sea) {
            Err(MiddleError::Budget { budget, .. }) => {
                assert_eq!(budget, "MAX_PEA_PROP_PASSES")
            }
            other => panic!("expected Budget, got {other:?}"),
        }
    }
}
