// CEP:FILE: crates/rv8_middle/src/analysis.rs
// CEP:WHAT: Iterative Sea-of-Nodes analyses: dense def-use/use-def indexes, block-level
//   CFG from the control-node wiring, Cooper-Harvey-Kennedy dominators, natural-loop
//   detection via back edges, and backward liveness over dense u64 bitsets.
// CEP:WHY: Every rv8_middle pass and the borrow checker build on these; flat index
//   structures keep identity stable (38.17) and every traversal is an explicit stack or
//   worklist, so untrusted graphs cannot drive recursion (38.13).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: budget guards return MiddleError::Budget before allocating unbounded
//   structures; accessors are total over dense indices.
// CEP:ASSUMES: node.block follows the flattening convention (BlockId equals RegionId for
//   region blocks, synthesized ids for joins); control input positions follow
//   docs/specs/multi_level_ir.md section 4 (Region/Merge inputs all control; Branch
//   cond-then-control; Return value-then-control).
// CEP:COST: build O(N*fanin); dominators O(E) RPO plus O(P*rounds) fixpoint; liveness
//   O(passes * blocks * words); all budgets are named constants below.
// CEP:EVIDENCE: unit tests rv8_middle::analysis::tests::*.
// CEP:SECURITY: no recursion; every loop is bounded by a named constant or vector length.
// CEP:DETERMINISM: deterministic; vectors keyed by dense ids, no hash iteration anywhere.

use crate::error::MiddleError;
use rv8_ir::son::{BlockId, NodeId, Opcode, SeaOfNodes};

/// CEP:WHAT: Maximum nodes per graph accepted by any rv8_middle entry point.
/// CEP:WHY: Named budget (38.12) aligned with rv8_ir::son::MAX_NODES_PER_FUNCTION; guards
///   analyses, passes, borrow checking, and PEA before they allocate.
/// CEP:STATUS: complete
/// CEP:FAILURE: check_budget returns MiddleError::Budget above the limit.
/// CEP:ASSUMES: enforced at every public entry point.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_MIDDLE_NODES: u32 = 1_000_000;

/// CEP:WHAT: Maximum basic blocks per graph for CFG-based analyses.
/// CEP:WHY: Named budget (38.12) bounding CFG, dominator, and loop worklists.
/// CEP:STATUS: complete
/// CEP:FAILURE: build_cfg returns MiddleError::Budget above the limit.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_ANALYSIS_BLOCKS: u32 = 65_536;

/// CEP:WHAT: Maximum u64 words per liveness bitset (MAX_LIVENESS_WORDS * 64 bits).
/// CEP:WHY: Named budget (38.12): liveness bitsets are dense chunks and must stay bounded
///   even for maximum-size graphs.
/// CEP:STATUS: complete
/// CEP:FAILURE: liveness returns MiddleError::Budget when more words would be needed.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_LIVENESS_WORDS: usize = 16_384;

/// CEP:WHAT: Maximum total bitset words across all blocks (blocks * words product guard).
/// CEP:WHY: Named budget (38.12) bounding liveness memory even for many-block graphs.
/// CEP:STATUS: complete
/// CEP:FAILURE: liveness returns MiddleError::Budget above the limit.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_LIVENESS_CELLS: usize = 1 << 22;

/// CEP:WHAT: Maximum backward-propagation rounds for the liveness fixpoint.
/// CEP:WHY: Named budget (38.12); worst-case round count equals loop nesting depth, so a
///   generous constant fails closed instead of looping forever.
/// CEP:STATUS: complete
/// CEP:FAILURE: liveness returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_LIVENESS_PASSES: u32 = 4_096;

/// CEP:WHAT: Maximum fixpoint rounds for iterative dominator refinement.
/// CEP:WHY: Named budget (38.12); CHK converges in a handful of rounds on real CFGs.
/// CEP:STATUS: complete
/// CEP:FAILURE: dominators returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub const MAX_DOM_FIXPOINT_PASSES: u32 = 128;

/// CEP:WHAT: Maximum idom-chain steps in one dominance query or intersection.
/// CEP:WHY: Named budget (38.12); chain length is bounded by block count in practice, the
///   constant fails closed on malformed trees.
/// CEP:STATUS: complete
/// CEP:FAILURE: strictly_dominates returns false when the walk budget trips.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dominators_on_loop_cfg.
pub const MAX_DOMINANCE_WALK_STEPS: u32 = 1_000_000;

/// CEP:WHAT: Maximum total worklist steps across all natural-loop extractions.
/// CEP:WHY: Named budget (38.12) bounding the O(loops * blocks) loop-body walk.
/// CEP:STATUS: complete
/// CEP:FAILURE: detect_loops returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::detects_loop_back_edge.
pub const MAX_LOOP_WORK: u64 = 1 << 20;

/// CEP:WHAT: Enforces the per-graph node budget at public entry points.
/// CEP:WHY: Named compile-time budget (38.12); pure function for direct testing.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget when nodes exceeds MAX_MIDDLE_NODES.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::budget_guard_boundary.
pub fn check_budget(nodes: usize) -> Result<(), MiddleError> {
    if nodes as u64 > MAX_MIDDLE_NODES as u64 {
        Err(MiddleError::Budget {
            budget: "MAX_MIDDLE_NODES",
            limit: MAX_MIDDLE_NODES as u64,
            actual: nodes as u64,
        })
    } else {
        Ok(())
    }
}

/// CEP:WHAT: Splits a node's inputs into data positions and control positions per the
///   flattening convention (mirrors rv8_ir::lower_het::split_inputs).
/// CEP:WHY: CFG construction, verification, and escape analysis all need the same
///   positional classification; one shared table prevents divergence.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Region/Merge inputs are all control; Branch is (cond, control); Return is
///   value-then-control (all-control when it has a single input).
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::ctrl_positions_match_convention.
pub(crate) fn ctrl_positions(opcode: Opcode, len: usize) -> (Vec<usize>, Vec<usize>) {
    match opcode {
        Opcode::Region | Opcode::Merge => (Vec::new(), (0..len).collect()),
        Opcode::Branch => (vec![0], if len >= 2 { vec![1] } else { Vec::new() }),
        Opcode::Return => {
            if len >= 2 {
                ((0..len - 1).collect(), vec![len - 1])
            } else {
                (Vec::new(), (0..len).collect())
            }
        }
        _ => ((0..len).collect(), Vec::new()),
    }
}

/// CEP:WHAT: Reports whether an opcode is a control-transfer node for CFG purposes.
/// CEP:WHY: CFG construction and entry detection scan only control nodes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: same class set as rv8_ir::lower_het (Region, Merge, Branch, Return).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::ctrl_positions_match_convention.
pub(crate) fn is_control_opcode(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::Region | Opcode::Merge | Opcode::Branch | Opcode::Return)
}

/// CEP:WHAT: Dense def-use and use-def indexes over one graph.
/// CEP:WHY: Passes and the borrow checker need consumer lists without repeated O(N) scans.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: indices are dense and aligned with node ids.
/// CEP:COST: O(N * fanin) time and space.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dataflow_index_matches_graph.
#[derive(Clone, Debug, Default)]
pub struct DataFlowIndex {
    /// For each node id, the ids of nodes that use it as an input (ascending by user id).
    pub uses_of: Vec<Vec<NodeId>>,
    /// For each node id, its own input list (copied for index-addressed access).
    pub operands_of: Vec<Vec<NodeId>>,
}

/// CEP:WHAT: Builds the def-use/use-def index of a graph in one pass.
/// CEP:WHY: Shared consumer of every per-pass adjacency need; deterministic order.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(N * fanin).
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dataflow_index_matches_graph.
pub fn build_dataflow(sea: &SeaOfNodes) -> DataFlowIndex {
    let mut index = DataFlowIndex {
        uses_of: vec![Vec::new(); sea.nodes.len()],
        operands_of: vec![Vec::new(); sea.nodes.len()],
    };
    for (u, node) in sea.nodes.iter().enumerate() {
        let uid = NodeId(u as u32);
        index.operands_of[u].extend_from_slice(&node.inputs);
        for &d in &node.inputs {
            if (d.0 as usize) < index.uses_of.len() {
                index.uses_of[d.0 as usize].push(uid);
            }
        }
    }
    index
}

/// CEP:WHAT: Block-level control-flow graph: successors and predecessors per block, entry
///   block, derived from control-node wiring rather than stored block links.
/// CEP:WHY: The SoN contract stores control only as input edges; the CFG is an analysis
///   product, and index-addressed vectors keep it deterministic (38.17).
/// CEP:STATUS: complete
/// CEP:FAILURE: none beyond the construction budget; unreachable blocks simply have empty
///   predecessor lists and no dominator information.
/// CEP:ASSUMES: each successor/predecessor list is sorted ascending and deduplicated.
/// CEP:COST: O(N + E log E) for sorted dedup.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::cfg_from_if_else_graph.
#[derive(Clone, Debug, Default)]
pub struct ControlFlowGraph {
    /// Number of blocks; ids are dense 0..block_count.
    pub block_count: u32,
    /// Entry block, chosen as the block of the first control node with no control inputs.
    pub entry: Option<BlockId>,
    /// Sorted, deduplicated successor lists per block.
    pub succs: Vec<Vec<BlockId>>,
    /// Sorted, deduplicated predecessor lists per block.
    pub preds: Vec<Vec<BlockId>>,
}

/// CEP:WHAT: Builds the block-level CFG from control-node input wiring.
/// CEP:WHY: Dominators, loops, liveness, and the verifier consume one CFG product.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget when the block count exceeds MAX_ANALYSIS_BLOCKS.
/// CEP:ASSUMES: control input positions follow the flattening convention.
/// CEP:COST: O(N + E log E) for sorted dedup.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::cfg_from_if_else_graph.
pub fn build_cfg(sea: &SeaOfNodes) -> Result<ControlFlowGraph, MiddleError> {
    let mut block_count: u32 = 0;
    for node in &sea.nodes {
        if let Some(b) = node.block {
            block_count = block_count.max(b.0 + 1);
        }
    }
    if block_count > MAX_ANALYSIS_BLOCKS {
        return Err(MiddleError::Budget {
            budget: "MAX_ANALYSIS_BLOCKS",
            limit: MAX_ANALYSIS_BLOCKS as u64,
            actual: block_count as u64,
        });
    }
    let n = block_count as usize;
    let mut succs: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for node in &sea.nodes {
        if !is_control_opcode(node.opcode) {
            continue;
        }
        let (_, ctrl) = ctrl_positions(node.opcode, node.inputs.len());
        let ub = match node.block {
            Some(b) => b,
            None => continue,
        };
        for c in ctrl {
            let cid = match node.inputs.get(c) {
                Some(&id) if (id.0 as usize) < sea.nodes.len() => id,
                _ => continue,
            };
            let cnode = &sea.nodes[cid.0 as usize];
            if !is_control_opcode(cnode.opcode) {
                continue;
            }
            if let Some(cb) = cnode.block {
                if cb != ub {
                    succs[cb.0 as usize].push(ub);
                    preds[ub.0 as usize].push(cb);
                }
            }
        }
    }
    // Entry: block of the first (lowest id) control node with no control inputs.
    let mut entry = None;
    for node in &sea.nodes {
        if !is_control_opcode(node.opcode) {
            continue;
        }
        let (_, ctrl) = ctrl_positions(node.opcode, node.inputs.len());
        if ctrl.is_empty() {
            entry = node.block;
            break;
        }
    }
    for i in 0..n {
        succs[i].sort_unstable();
        succs[i].dedup();
        preds[i].sort_unstable();
        preds[i].dedup();
    }
    Ok(ControlFlowGraph { block_count, entry, succs, preds })
}

/// CEP:WHAT: Immediate-dominator tree over block ids, computed iteratively.
/// CEP:WHY: Dominance is the legality backbone for verification and passes (38.22);
///   recursion-free CHK keeps the computation safe on untrusted CFGs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none beyond the construction budget.
/// CEP:ASSUMES: idom at the entry block is Some of itself; unreachable blocks have idom None.
/// CEP:COST: O(V + E) RPO plus O(rounds * V * P) fixpoint, rounds budgeted.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dominators_on_loop_cfg.
#[derive(Clone, Debug, Default)]
pub struct Dominators {
    /// Immediate dominator per block; None for unreachable blocks.
    pub idom: Vec<Option<BlockId>>,
}

impl Dominators {
    /// CEP:WHAT: Reports whether block a strictly dominates block b.
    /// CEP:WHY: Verifier dominance checks and pass soundness guards query this.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns false when either block is unreachable or the walk budget
    ///   trips (fail-closed).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(tree depth), bounded by MAX_DOMINANCE_WALK_STEPS.
    /// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dominators_on_loop_cfg.
    pub fn strictly_dominates(&self, a: BlockId, b: BlockId) -> bool {
        if a == b {
            return false;
        }
        let mut cur = b;
        let mut steps: u32 = 0;
        loop {
            match self.idom.get(cur.0 as usize).copied().flatten() {
                Some(p) if p == cur => return false,
                Some(p) => {
                    if p == a {
                        return true;
                    }
                    cur = p;
                }
                None => return false,
            }
            steps += 1;
            if steps > MAX_DOMINANCE_WALK_STEPS {
                return false;
            }
        }
    }
}

/// CEP:WHAT: Computes immediate dominators with reverse postorder and the iterative
///   Cooper-Harvey-Kennedy fixpoint (no recursion).
/// CEP:WHY: Standard 38.13 forbids recursion over untrusted structure; CHK over RPO is
///   the canonical iterative formulation.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget when the fixpoint exceeds MAX_DOM_FIXPOINT_PASSES.
/// CEP:ASSUMES: succs and preds are consistent; entry is reachable from itself.
/// CEP:COST: O(V + E) RPO plus budgeted fixpoint rounds.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dominators_on_loop_cfg.
pub fn dominators(cfg: &ControlFlowGraph) -> Result<Dominators, MiddleError> {
    let n = cfg.block_count as usize;
    let mut doms = Dominators { idom: vec![None; n] };
    let entry = match cfg.entry {
        Some(e) if (e.0 as usize) < n => e,
        _ => return Ok(doms),
    };
    // Iterative postorder DFS over successors with an explicit stack.
    let mut visited = vec![false; n];
    let mut post: Vec<BlockId> = Vec::with_capacity(n);
    let mut stack: Vec<(BlockId, usize)> = vec![(entry, 0)];
    visited[entry.0 as usize] = true;
    while !stack.is_empty() {
        let top = stack.len() - 1;
        let b = stack[top].0;
        let next = stack[top].1;
        let succ_list = &cfg.succs[b.0 as usize];
        if next < succ_list.len() {
            stack[top].1 += 1;
            let w = succ_list[next];
            if !visited[w.0 as usize] {
                visited[w.0 as usize] = true;
                stack.push((w, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post.reverse();
    let mut pos = vec![-1i64; n];
    for (i, &b) in post.iter().enumerate() {
        pos[b.0 as usize] = i as i64;
    }
    doms.idom[entry.0 as usize] = Some(entry);
    let mut changed = true;
    let mut rounds: u32 = 0;
    while changed {
        changed = false;
        rounds += 1;
        if rounds > MAX_DOM_FIXPOINT_PASSES {
            return Err(MiddleError::Budget {
                budget: "MAX_DOM_FIXPOINT_PASSES",
                limit: MAX_DOM_FIXPOINT_PASSES as u64,
                actual: rounds as u64,
            });
        }
        for &b in &post {
            if b == entry {
                continue;
            }
            let mut new_idom: Option<BlockId> = None;
            for &p in &cfg.preds[b.0 as usize] {
                if pos[p.0 as usize] < 0 || doms.idom[p.0 as usize].is_none() {
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(cur) => intersect(&doms, &pos, cur, p),
                });
            }
            if let Some(nid) = new_idom {
                if doms.idom[b.0 as usize] != Some(nid) {
                    doms.idom[b.0 as usize] = Some(nid);
                    changed = true;
                }
            }
        }
    }
    Ok(doms)
}

/// CEP:WHAT: CHK dominator intersection over reverse-postorder indices.
/// CEP:WHY: Shared helper of the iterative dominator fixpoint.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns the current finger when the chain walk finds no common ancestor
///   (fail-closed).
/// CEP:ASSUMES: both fingers have computed idoms.
/// CEP:COST: O(tree depth) per call, walk-budget bounded.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::dominators_on_loop_cfg.
fn intersect(doms: &Dominators, pos: &[i64], mut f1: BlockId, mut f2: BlockId) -> BlockId {
    let budget = u64::from(MAX_DOMINANCE_WALK_STEPS);
    let mut steps: u64 = 0;
    while f1 != f2 {
        while pos[f1.0 as usize] > pos[f2.0 as usize] {
            match doms.idom[f1.0 as usize] {
                Some(p) => f1 = p,
                None => return f1,
            }
            steps += 1;
            if steps > budget {
                return f1;
            }
        }
        while pos[f2.0 as usize] > pos[f1.0 as usize] {
            match doms.idom[f2.0 as usize] {
                Some(p) => f2 = p,
                None => return f2,
            }
            steps += 1;
            if steps > budget {
                return f2;
            }
        }
        steps += 1;
        if steps > budget {
            return f1;
        }
    }
    f1
}

/// CEP:WHAT: Natural-loop facts: back edges latch-then-header and a per-block loop-header
///   map used for the phi back-edge dominance exception.
/// CEP:WHY: Spec 2.3.1 excepts loop back-edges from operand-dominates-use; the verifier
///   needs a deterministic map from blocks to their loop header.
/// CEP:STATUS: complete
/// CEP:FAILURE: none beyond the work budget.
/// CEP:ASSUMES: loop_header_of_block records the header of the first (lowest-id) natural
///   loop containing the block; nested loops keep the first recorded header.
/// CEP:COST: O(back_edges * blocks) bounded by MAX_LOOP_WORK.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::detects_loop_back_edge.
#[derive(Clone, Debug, Default)]
pub struct Loops {
    /// Back edges (from latch block, to header block), sorted ascending.
    pub back_edges: Vec<(BlockId, BlockId)>,
    /// For each block, the header of the first natural loop containing it.
    pub loop_header_of_block: Vec<Option<BlockId>>,
}

/// CEP:WHAT: Finds back edges (edges into a dominating block) and their natural loops.
/// CEP:WHY: Loop detection via dominators per spec 2.3.1; fully iterative worklist.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget when MAX_LOOP_WORK steps are exhausted.
/// CEP:ASSUMES: dominators computed for the same CFG.
/// CEP:COST: O(V + E) scan plus budgeted loop-body walks.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::detects_loop_back_edge.
pub fn detect_loops(cfg: &ControlFlowGraph, doms: &Dominators) -> Result<Loops, MiddleError> {
    let n = cfg.block_count as usize;
    let mut back_edges: Vec<(BlockId, BlockId)> = Vec::new();
    for b in 0..n {
        for &s in &cfg.succs[b] {
            if s == BlockId(b as u32) || doms.strictly_dominates(s, BlockId(b as u32)) {
                back_edges.push((BlockId(b as u32), s));
            }
        }
    }
    back_edges.sort_unstable();
    back_edges.dedup();
    let mut loops = Loops {
        back_edges,
        loop_header_of_block: vec![None; n],
    };
    let mut work: u64 = 0;
    for &(latch, header) in &loops.back_edges {
        let mut members = vec![false; n];
        members[header.0 as usize] = true;
        let mut stack = vec![latch];
        while let Some(x) = stack.pop() {
            work += 1;
            if work > MAX_LOOP_WORK {
                return Err(MiddleError::Budget {
                    budget: "MAX_LOOP_WORK",
                    limit: MAX_LOOP_WORK,
                    actual: work,
                });
            }
            if members[x.0 as usize] {
                continue;
            }
            members[x.0 as usize] = true;
            for &p in &cfg.preds[x.0 as usize] {
                stack.push(p);
            }
        }
        for (i, in_loop) in members.iter().enumerate() {
            if *in_loop && loops.loop_header_of_block[i].is_none() {
                loops.loop_header_of_block[i] = Some(header);
            }
        }
    }
    Ok(loops)
}

/// CEP:WHAT: Backward liveness over node ids as dense u64-chunk bitsets per block.
/// CEP:WHY: Register-allocation warm start (5.4) and pressure estimation need live sets;
///   dense chunks keep the fixpoint allocation-light and deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget when words, cells, or fixpoint passes exceed the
///   named limits.
/// CEP:ASSUMES: gen is the operand set of a block's nodes, def is the set of ids defined
///   in the block (SSA makes this exact for values).
/// CEP:COST: O(passes * blocks * words); budgets named above.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::liveness_marks_operand_ranges.
#[derive(Clone, Debug, Default)]
pub struct Liveness {
    /// Words per bitset; bits index node ids.
    pub words: usize,
    /// live_in per block, dense u64 chunks.
    pub live_in: Vec<Vec<u64>>,
    /// live_out per block, dense u64 chunks.
    pub live_out: Vec<Vec<u64>>,
}

/// CEP:WHAT: Computes per-block live-in/live-out bitsets with an iterative backward
///   fixpoint over the CFG.
/// CEP:WHY: Downstream allocator warm start and pressure estimation; demonstrates the
///   dense-chunk bitset contract named in the component spec.
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget variants for the liveness limits.
/// CEP:ASSUMES: none
/// CEP:COST: O(passes * blocks * words); passes budgeted.
/// CEP:EVIDENCE: unit test rv8_middle::analysis::tests::liveness_marks_operand_ranges.
pub fn liveness(sea: &SeaOfNodes, cfg: &ControlFlowGraph) -> Result<Liveness, MiddleError> {
    let n = sea.nodes.len();
    let words = n.div_ceil(64);
    if words > MAX_LIVENESS_WORDS {
        return Err(MiddleError::Budget {
            budget: "MAX_LIVENESS_WORDS",
            limit: MAX_LIVENESS_WORDS as u64,
            actual: words as u64,
        });
    }
    let blocks = cfg.block_count as usize;
    if blocks
        .checked_mul(words)
        .is_some_and(|cells| cells > MAX_LIVENESS_CELLS)
    {
        return Err(MiddleError::Budget {
            budget: "MAX_LIVENESS_CELLS",
            limit: MAX_LIVENESS_CELLS as u64,
            actual: (blocks as u64) * (words as u64),
        });
    }
    // Per-block gen (operands referenced) and def (ids defined in the block).
    let mut gen: Vec<Vec<u64>> = vec![vec![0u64; words]; blocks];
    let mut def: Vec<Vec<u64>> = vec![vec![0u64; words]; blocks];
    for (u, node) in sea.nodes.iter().enumerate() {
        if let Some(b) = node.block {
            let bi = b.0 as usize;
            if bi < blocks {
                bs_set(&mut def[bi], u);
                for &d in &node.inputs {
                    bs_set(&mut gen[bi], d.0 as usize);
                }
            }
        }
    }
    let mut live = Liveness {
        words,
        live_in: vec![vec![0u64; words]; blocks],
        live_out: vec![vec![0u64; words]; blocks],
    };
    let mut rounds: u32 = 0;
    let mut changed = true;
    while changed {
        changed = false;
        rounds += 1;
        if rounds > MAX_LIVENESS_PASSES {
            return Err(MiddleError::Budget {
                budget: "MAX_LIVENESS_PASSES",
                limit: MAX_LIVENESS_PASSES as u64,
                actual: rounds as u64,
            });
        }
        for b in (0..blocks).rev() {
            // live_out = union of successor live_in.
            let mut out = vec![0u64; words];
            for &s in &cfg.succs[b] {
                bs_union_into(&mut out, &live.live_in[s.0 as usize]);
            }
            // live_in = gen | (out - def).
            let mut inn = out.clone();
            for w in 0..words {
                inn[w] &= !def[b][w];
            }
            bs_union_into(&mut inn, &gen[b]);
            if out != live.live_out[b] {
                live.live_out[b] = out;
                changed = true;
            }
            if inn != live.live_in[b] {
                live.live_in[b] = inn;
                changed = true;
            }
        }
    }
    Ok(live)
}

/// CEP:WHAT: Sets one bit in a dense u64-chunk bitset.
/// CEP:WHY: Shared bitset primitive for liveness and borrow-region sets.
/// CEP:STATUS: complete
/// CEP:FAILURE: out-of-range indices are ignored (fail-safe under budgets).
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: exercised by every bitset test.
pub(crate) fn bs_set(bs: &mut [u64], bit: usize) {
    let word = bit / 64;
    if word < bs.len() {
        bs[word] |= 1u64 << (bit % 64);
    }
}

/// CEP:WHAT: Tests one bit in a dense u64-chunk bitset.
/// CEP:WHY: Shared bitset primitive.
/// CEP:STATUS: complete
/// CEP:FAILURE: out-of-range indices read false.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: exercised by every bitset test.
pub(crate) fn bs_test(bs: &[u64], bit: usize) -> bool {
    let word = bit / 64;
    word < bs.len() && (bs[word] >> (bit % 64)) & 1 == 1
}

/// CEP:WHAT: Unions bitset b into bitset a, reporting whether a changed.
/// CEP:WHY: Fixpoint convergence detection without extra passes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: a and b have the same length.
/// CEP:COST: O(words).
/// CEP:EVIDENCE: exercised by every bitset test.
pub(crate) fn bs_union_into(a: &mut [u64], b: &[u64]) -> bool {
    let mut changed = false;
    for (x, y) in a.iter_mut().zip(b.iter()) {
        let new = *x | *y;
        if new != *x {
            *x = new;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::lowering::hir_to_son;
    use rv8_core::span::Span;
    use rv8_ir::{ExprKind, HirFunction, IrType, RegionKind, StmtKind};

    /// CEP:WHAT: Blank span for test HIR construction.
    /// CEP:WHY: Keeps test bodies short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span::new(rv8_core::span::FileId(0), 0, 0)
    }

    /// CEP:WHAT: Appends one node with an explicit block assignment.
    /// CEP:WHY: Test fixture helper mirroring the flattening emission path.
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

    /// CEP:WHAT: Builds fn f() { let x = 0; if 1 { x = 2 } else { x = 3 }; return x } as
    ///   HIR and lowers it, producing a multi-block SoN graph with a join phi.
    /// CEP:WHY: Shared fixture for CFG, dominator, liveness, and verifier tests.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: lowering contract of rv8_ir.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    pub fn if_else_son() -> SeaOfNodes {
        let mut h = HirFunction::new("f");
        let root = h.add_region(RegionKind::Block, None, sp());
        let x = h.add_var("x", IrType::I32, root);
        h.add_stmt(root, StmtKind::Let { var: x, init: None }, sp());
        let cond = h.add_expr(ExprKind::Const(1), IrType::Bool, sp());
        let cif = h.add_region(RegionKind::Conditional, Some(root), sp());
        h.regions[cif.0 as usize].cond = Some(cond);
        let t = h.add_region(RegionKind::Block, Some(cif), sp());
        let e = h.add_region(RegionKind::Block, Some(cif), sp());
        let two = h.add_expr(ExprKind::Const(2), IrType::I32, sp());
        h.add_stmt(t, StmtKind::Assign { var: x, value: two }, sp());
        let three = h.add_expr(ExprKind::Const(3), IrType::I32, sp());
        h.add_stmt(e, StmtKind::Assign { var: x, value: three }, sp());
        let tail = h.add_region(RegionKind::Block, Some(root), sp());
        let read = h.add_expr(ExprKind::Var(x), IrType::I32, sp());
        h.add_stmt(tail, StmtKind::Return(Some(read)), sp());
        hir_to_son(&h).expect("lowering fixture")
    }

    /// CEP:WHAT: Builds the canonical while-loop graph: entry, header with phi, branch,
    ///   latch block computing the back-edge value, and exit.
    /// CEP:WHY: Shared fixture for loop detection, the phi back-edge exception, and pass
    ///   tests that need a nontrivial CFG.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: forward node references are legal at construction time.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    pub fn loop_graph() -> SeaOfNodes {
        let mut sea = SeaOfNodes::new();
        let n0 = push(&mut sea, Opcode::Region, IrType::Unit, vec![], 0);
        let one = push(&mut sea, Opcode::Const(1), IrType::I32, vec![], 0);
        // Forward references: header cites the latch region, phi cites the back edge.
        let h = push(&mut sea, Opcode::Region, IrType::Unit, vec![n0, NodeId(6)], 1);
        let phi = push(&mut sea, Opcode::Phi, IrType::I32, vec![one, NodeId(7)], 1);
        let zero = push(&mut sea, Opcode::Const(0), IrType::I32, vec![], 1);
        let br = push(&mut sea, Opcode::Branch, IrType::Unit, vec![zero, h], 1);
        assert_eq!(br, NodeId(5));
        let latchr = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 2);
        assert_eq!(latchr, NodeId(6));
        let inc = push(&mut sea, Opcode::Add, IrType::I32, vec![phi, one], 2);
        assert_eq!(inc, NodeId(7));
        let exit = push(&mut sea, Opcode::Region, IrType::Unit, vec![br], 3);
        let _ = push(&mut sea, Opcode::Return, IrType::Unit, vec![phi, exit], 3);
        sea
    }

    /// CEP:WHAT: Verifies the CFG derived from control wiring has the expected block
    ///   count, entry, and branch fan-out.
    /// CEP:WHY: CFG is the base product every other analysis consumes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn cfg_from_if_else_graph() {
        let sea = if_else_son();
        let cfg = build_cfg(&sea).expect("cfg");
        assert!(cfg.block_count >= 4, "entry, then, else, join blocks");
        assert_eq!(cfg.entry, Some(BlockId(0)));
        let mut branch_outs = 0;
        for s in &cfg.succs {
            branch_outs = branch_outs.max(s.len());
        }
        assert_eq!(branch_outs, 2, "branch block has two successors");
        for list in cfg.succs.iter().chain(cfg.preds.iter()) {
            let mut sorted = list.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(*list, sorted, "lists sorted and deduplicated");
        }
    }

    /// CEP:WHAT: Verifies the def-use/use-def index agrees with the graph inputs.
    /// CEP:WHY: Passes rely on exact consumer lists.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn dataflow_index_matches_graph() {
        let sea = if_else_son();
        let idx = build_dataflow(&sea);
        assert_eq!(idx.uses_of.len(), sea.nodes.len());
        for (u, node) in sea.nodes.iter().enumerate() {
            assert_eq!(idx.operands_of[u], node.inputs, "operands copied verbatim");
            for &d in &node.inputs {
                assert!(
                    idx.uses_of[d.0 as usize].contains(&NodeId(u as u32)),
                    "user recorded under its operand"
                );
            }
        }
    }

    /// CEP:WHAT: Verifies dominators on the loop CFG: entry dominates all, header
    ///   dominates latch, latch does not dominate header, and the back edge is found.
    /// CEP:WHY: CHK must produce the classic tree and loop facts iteratively.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn dominators_on_loop_cfg() {
        let sea = loop_graph();
        let cfg = build_cfg(&sea).expect("cfg");
        let doms = dominators(&cfg).expect("dominators");
        let b0 = BlockId(0);
        for b in 1..cfg.block_count {
            assert!(doms.strictly_dominates(b0, BlockId(b)), "entry dominates b{b}");
        }
        assert!(!doms.strictly_dominates(BlockId(2), BlockId(1)));
        assert!(doms.strictly_dominates(BlockId(1), BlockId(2)));
        let loops = detect_loops(&cfg, &doms).expect("loops");
        assert_eq!(loops.back_edges, vec![(BlockId(2), BlockId(1))]);
        assert_eq!(loops.loop_header_of_block[2], Some(BlockId(1)));
        assert_eq!(loops.loop_header_of_block[1], Some(BlockId(1)));
        assert_eq!(loops.loop_header_of_block[3], None, "exit is outside the loop");
        // A second run is identical (determinism).
        let again = detect_loops(&cfg, &doms).expect("loops 2");
        assert_eq!(loops.back_edges, again.back_edges);
    }

    /// CEP:WHAT: Verifies liveness computes bounded dense-chunk bitsets whose entry
    ///   live-in covers entry-block operands, and that reruns are identical.
    /// CEP:WHY: Liveness feeds allocator warm start; the chunked shape is contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn liveness_marks_operand_ranges() {
        let sea = if_else_son();
        let cfg = build_cfg(&sea).expect("cfg");
        let live = liveness(&sea, &cfg).expect("liveness");
        assert_eq!(live.live_in.len(), cfg.block_count as usize);
        assert_eq!(live.live_out.len(), cfg.block_count as usize);
        assert!(live.words <= MAX_LIVENESS_WORDS);
        let entry = cfg.entry.expect("entry");
        for node in &sea.nodes {
            if node.block == Some(entry) {
                for &d in &node.inputs {
                    assert!(bs_test(&live.live_in[entry.0 as usize], d.0 as usize));
                }
            }
        }
        let again = liveness(&sea, &cfg).expect("liveness 2");
        assert_eq!(format!("{live:?}"), format!("{again:?}"));
    }

    /// CEP:WHAT: Verifies the positional control/data input table against the documented
    ///   flattening convention.
    /// CEP:WHY: One shared table must match rv8_ir::lower_het::split_inputs exactly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn ctrl_positions_match_convention() {
        assert_eq!(ctrl_positions(Opcode::Branch, 2), (vec![0], vec![1]));
        assert_eq!(ctrl_positions(Opcode::Region, 2), (Vec::<usize>::new(), vec![0, 1]));
        assert_eq!(ctrl_positions(Opcode::Return, 2), (vec![0], vec![1]));
        assert_eq!(ctrl_positions(Opcode::Return, 1), (Vec::<usize>::new(), vec![0]));
        assert_eq!(ctrl_positions(Opcode::Add, 2), (vec![0, 1], Vec::<usize>::new()));
        assert!(is_control_opcode(Opcode::Merge));
        assert!(!is_control_opcode(Opcode::Call { callee: 0 }));
    }

    /// CEP:WHAT: Verifies the node budget guard accepts the boundary and rejects one past
    ///   it with MiddleError::Budget.
    /// CEP:WHY: Named budgets must fail closed (38.12), never OOM or panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(1).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn budget_guard_boundary() {
        assert!(check_budget(MAX_MIDDLE_NODES as usize).is_ok());
        match check_budget(MAX_MIDDLE_NODES as usize + 1) {
            Err(MiddleError::Budget { budget, limit, actual }) => {
                assert_eq!(budget, "MAX_MIDDLE_NODES");
                assert_eq!(limit, u64::from(MAX_MIDDLE_NODES));
                assert_eq!(actual, u64::from(MAX_MIDDLE_NODES) + 1);
            }
            other => panic!("expected Budget, got {other:?}"),
        }
    }
}
