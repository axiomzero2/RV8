// CEP:FILE: crates/rv8_middle/src/borrowck.rs
// CEP:WHAT: The graph-CSP borrow checker over the Sea of Nodes (standard 5.4): pointer
//   provenance by bounded fixpoint, a borrow graph with mutable and immutable edges from
//   stores and loads, region inference by iterative Tarjan SCC condensation with
//   reverse-topological bitset propagation, validation by bitset AND intersection
//   (mutable-mutable alias overlap, use-after-move), and drop-graph cycle detection by
//   iterative three-color DFS.
// CEP:WHY: Memory-safety verification on the MIR graph; bitset intersection is the
//   documented validation mechanism and every traversal is iterative on untrusted input.
// CEP:CLASS: HPC-0
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-104
// CEP:FAILURE: MiddleError::Budget when regions, propagation rounds, or DFS steps exceed
//   the named constants; MiddleError::Ir for structural failures. Safety verdicts are the
//   returned BorrowReport::conflicts vector (empty = accepted, deterministic order).
// CEP:ASSUMES: the node conventions are Alloc: no inputs (Ptr result defining an object);
//   Store: value-then-address; Load: [address]; Drop: object; EscapePoint: object;
//   Materialize: object; Call/Return/Branch data inputs per the flattening table;
//   allocations and pointer roots are Ptr-typed; provenance through unknown stores is
//   tracked, but loads are given their own root region plus stored-site union; move order
//   is node-id order (documented corpus simplification, not epoch-based lifetimes).
// CEP:COST: O(passes * N * R/64) provenance, O(R + E) Tarjan, O(comps * R/64) closure,
//   O(R + E) DFS; all bounds are named constants.
// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::* (3 accepted, 3 rejected, each
//   run twice for determinism).
// CEP:SECURITY: no recursion; Tarjan and DFS use explicit stacks with step budgets.
// CEP:DETERMINISM: deterministic; conflicts sorted by (kind, node, other), all sets are
//   dense bitsets or ordered structures.

use crate::analysis::{bs_set, bs_test, bs_union_into, check_budget};
use crate::error::MiddleError;
use rv8_ir::son::{NodeId, Opcode, IrType, SeaOfNodes};
use std::collections::BTreeSet;

/// CEP:WHAT: Maximum object regions (allocation sites plus external pointer roots).
/// CEP:WHY: Named budget (38.12) bounding the R x R adjacency bitsets of the CSP.
/// CEP:STATUS: complete
/// CEP:FAILURE: check returns MiddleError::Budget above the limit.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::accepts_independent_allocs.
pub const MAX_BORROW_REGIONS: usize = 4_096;

/// CEP:WHAT: Maximum provenance fixpoint rounds before failing closed.
/// CEP:WHY: Named budget (38.12); phi cycles converge in a few rounds on real graphs.
/// CEP:STATUS: complete
/// CEP:FAILURE: check returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::accepts_independent_allocs.
pub const MAX_BORROW_PROP_PASSES: u32 = 256;

/// CEP:WHAT: Maximum three-color DFS steps for drop-graph cycle detection.
/// CEP:WHY: Named budget (38.12) bounding the explicit-stack traversal.
/// CEP:STATUS: complete
/// CEP:FAILURE: check returns MiddleError::Budget when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::rejects_drop_cycle.
pub const MAX_BORROW_DFS_STEPS: u64 = 1 << 20;

/// CEP:WHAT: One borrow-safety conflict: kind and the two offending nodes.
/// CEP:WHY: Deterministic reject/accept requires structured, comparable verdicts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: conflicts are sorted by (kind, node, other) before reporting.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Conflict {
    /// Which safety rule fired.
    pub kind: ConflictKind,
    /// Primary offending node (region root, drop site, or cycle member).
    pub node: NodeId,
    /// Secondary offending node (the aliasing region or the later use).
    pub other: NodeId,
}

/// CEP:WHAT: Conflict kinds, one per validation rule; declaration order is sort order.
/// CEP:WHY: Negative tests assert exact kinds; ordering keeps reports deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictKind {
    /// Two mutable-access regions are transitively mutable-aliased (bitset AND overlap).
    MutMutAlias,
    /// A region is used after a Drop of it (node-order move semantics).
    UseAfterMove,
    /// The drop graph restricted to dropped regions contains a cycle.
    DropCycle,
}

/// CEP:WHAT: The borrow-check verdict for one graph: region count and all conflicts.
/// CEP:WHY: Accept/reject is `conflicts.is_empty()`; Err is reserved for budget and
///   structural failures, keeping the safety verdict data-bearing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: conflicts are deterministically ordered.
/// CEP:COST: POD plus vector.
/// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BorrowReport {
    /// Number of object regions analyzed (allocation sites plus external roots).
    pub regions: usize,
    /// All conflicts, sorted by (kind, node, other).
    pub conflicts: Vec<Conflict>,
}

impl BorrowReport {
    /// CEP:WHAT: Reports whether the graph is accepted (no conflicts).
    /// CEP:WHY: Fail-closed acceptance predicate for pipeline glue.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
    pub fn is_accepted(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// CEP:WHAT: One provenance/move/use fact attached to a region.
/// CEP:WHY: Move-then-use validation scans per-region ordered facts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: facts are appended in ascending node order.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::rejects_use_after_move.
#[derive(Clone, Copy, Debug)]
struct Event {
    at: NodeId,
    is_move: bool,
}

/// CEP:WHAT: Internal analysis state: regions, provenance bitsets, adjacency, events.
/// CEP:WHY: One owner keeps the borrow-graph construction phases borrow-clean.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction fails closed via MiddleError on budget trips.
/// CEP:ASSUMES: node count already within MAX_MIDDLE_NODES.
/// CEP:COST: O(R * R/64) adjacency plus O(N * R/64) provenance.
/// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
struct BorrowGraph {
    /// Region root nodes, ascending by node id.
    roots: Vec<NodeId>,
    /// Region count.
    regions: usize,
    /// Words per region bitset.
    words: usize,
    /// Provenance bitset per node (Ptr nodes only; roots keep their single bit).
    prov: Vec<Vec<u64>>,
    /// Union of stored-value provenance per region.
    stored: Vec<Vec<u64>>,
    /// Mutable (store-created) contents adjacency per region.
    mut_adj: Vec<Vec<u64>>,
    /// Immutable (load-created) contents adjacency per region.
    imm_adj: Vec<Vec<u64>>,
    /// Regions written by at least one store.
    mut_access: Vec<bool>,
    /// Move/use events per region in ascending node order.
    events: Vec<Vec<Event>>,
}

impl BorrowGraph {
    /// CEP:WHAT: Discovers regions and initializes provenance roots.
    /// CEP:WHY: Allocations create regions-objects; external pointer roots (Parameter,
    ///   Call results) and loads get their own regions so unknown values stay tracked.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: MiddleError::Budget above MAX_BORROW_REGIONS.
    /// CEP:ASSUMES: Ptr-typed roots only.
    /// CEP:COST: O(N).
    /// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::accepts_independent_allocs.
    fn new(sea: &SeaOfNodes) -> Result<BorrowGraph, MiddleError> {
        let mut roots: Vec<NodeId> = Vec::new();
        for (u, node) in sea.nodes.iter().enumerate() {
            if node.ty == IrType::Ptr {
                let is_root = matches!(
                    node.opcode,
                    Opcode::Alloc | Opcode::Parameter(_) | Opcode::Load | Opcode::Call { .. }
                );
                if is_root {
                    roots.push(NodeId(u as u32));
                }
            }
        }
        if roots.len() > MAX_BORROW_REGIONS {
            return Err(MiddleError::Budget {
                budget: "MAX_BORROW_REGIONS",
                limit: MAX_BORROW_REGIONS as u64,
                actual: roots.len() as u64,
            });
        }
        let regions = roots.len();
        let words = regions.div_ceil(64);
        let mut prov = vec![vec![0u64; words]; sea.nodes.len()];
        for (r, &root) in roots.iter().enumerate() {
            bs_set(&mut prov[root.0 as usize], r);
        }
        Ok(BorrowGraph {
            roots,
            regions,
            words,
            prov,
            stored: vec![vec![0u64; words]; regions],
            mut_adj: vec![vec![0u64; words]; regions],
            imm_adj: vec![vec![0u64; words]; regions],
            mut_access: vec![false; regions],
            events: vec![Vec::new(); regions],
        })
    }

    /// CEP:WHAT: Runs the bounded provenance fixpoint over phi/load/stores.
    /// CEP:WHY: Loop phis create provenance cycles; the fixpoint converges to the least
    ///   solution and the round budget fails closed on non-convergence.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: MiddleError::Budget above MAX_BORROW_PROP_PASSES.
    /// CEP:ASSUMES: structure verified; dangling ids skipped defensively.
    /// CEP:COST: O(passes * N * words).
    /// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::accepts_shared_read_alias.
    fn propagate(&mut self, sea: &SeaOfNodes) -> Result<(), MiddleError> {
        let n = sea.nodes.len();
        let mut rounds: u32 = 0;
        let mut changed = true;
        while changed {
            changed = false;
            rounds += 1;
            if rounds > MAX_BORROW_PROP_PASSES {
                return Err(MiddleError::Budget {
                    budget: "MAX_BORROW_PROP_PASSES",
                    limit: MAX_BORROW_PROP_PASSES as u64,
                    actual: rounds as u64,
                });
            }
            // stored[r] = union of provenance of values stored into r.
            let mut new_stored = vec![vec![0u64; self.words]; self.regions];
            for node in &sea.nodes {
                if node.opcode == Opcode::Store && node.inputs.len() >= 2 {
                    let value = node.inputs[0];
                    let addr = node.inputs[1];
                    if (value.0 as usize) < n && (addr.0 as usize) < n {
                        for (r, row) in new_stored.iter_mut().enumerate() {
                            if bs_test(&self.prov[addr.0 as usize], r) {
                                bs_union_into(row, &self.prov[value.0 as usize]);
                            }
                        }
                    }
                }
            }
            if new_stored != self.stored {
                self.stored = new_stored;
                changed = true;
            }
            // Non-fixed provenance: loads read stored sites; phis and other pure pointer
            // operations union their pointer inputs.
            for (u, node) in sea.nodes.iter().enumerate() {
                if node.ty != IrType::Ptr {
                    continue;
                }
                let mut next = self.prov[u].clone();
                match node.opcode {
                    Opcode::Load => {
                        if let Some(&addr) = node.inputs.first() {
                            if (addr.0 as usize) < n {
                                for r in 0..self.regions {
                                    if bs_test(&self.prov[addr.0 as usize], r) {
                                        bs_union_into(&mut next, &self.stored[r]);
                                    }
                                }
                            }
                        }
                    }
                    Opcode::Alloc | Opcode::Parameter(_) | Opcode::Call { .. } => {}
                    _ => {
                        for &d in &node.inputs {
                            if (d.0 as usize) < n && sea.nodes[d.0 as usize].ty == IrType::Ptr {
                                bs_union_into(&mut next, &self.prov[d.0 as usize]);
                            }
                        }
                    }
                }
                if next != self.prov[u] {
                    self.prov[u] = next;
                    changed = true;
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Records borrow edges, mutable-access flags, and move/use events.
    /// CEP:WHY: Edges carry the mutability flag from node and type info; events give the
    ///   node-ordered move history per region.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (dangling ids skipped; structure pre-verified).
    /// CEP:ASSUMES: Store input order is [value, address]; Drop/EscapePoint/Materialize
    ///   carry the object as their first input.
    /// CEP:COST: O(N * words).
    /// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
    fn record_accesses(&mut self, sea: &SeaOfNodes) {
        let n = sea.nodes.len();
        let push_event = |graph: &mut BorrowGraph, prov: &[u64], at: usize, is_move: bool| {
            for r in 0..graph.regions {
                if bs_test(prov, r) {
                    graph.events[r].push(Event { at: NodeId(at as u32), is_move });
                }
            }
        };
        for (u, node) in sea.nodes.iter().enumerate() {
            let prov_at = |g: &BorrowGraph, id: NodeId| -> Vec<u64> {
                if (id.0 as usize) < n {
                    g.prov[id.0 as usize].clone()
                } else {
                    vec![0u64; g.words]
                }
            };
            match node.opcode {
                Opcode::Store if node.inputs.len() >= 2 => {
                    let value = node.inputs[0];
                    let addr = node.inputs[1];
                    let addr_prov = prov_at(self, addr);
                    for r in 0..self.regions {
                        if bs_test(&addr_prov, r) {
                            self.mut_access[r] = true;
                        }
                    }
                    if (value.0 as usize) < n && sea.nodes[value.0 as usize].ty == IrType::Ptr {
                        let value_prov = prov_at(self, value);
                        for a in 0..self.regions {
                            if bs_test(&addr_prov, a) {
                                bs_union_into(&mut self.mut_adj[a], &value_prov);
                            }
                        }
                    }
                    push_event(self, &addr_prov, u, false);
                    if (value.0 as usize) < n && sea.nodes[value.0 as usize].ty == IrType::Ptr {
                        let value_prov = prov_at(self, value);
                        push_event(self, &value_prov, u, false);
                    }
                }
                Opcode::Load if !node.inputs.is_empty() => {
                    let addr = node.inputs[0];
                    let addr_prov = prov_at(self, addr);
                    push_event(self, &addr_prov, u, false);
                    if node.ty == IrType::Ptr {
                        // Immutable borrow edges: reading r's contents borrows every site
                        // stored into r.
                        for a in 0..self.regions {
                            if bs_test(&addr_prov, a) {
                                bs_union_into(&mut self.imm_adj[a], &self.stored[a]);
                            }
                        }
                    }
                }
                Opcode::Drop if !node.inputs.is_empty() => {
                    let obj = node.inputs[0];
                    let obj_prov = prov_at(self, obj);
                    push_event(self, &obj_prov, u, true);
                }
                Opcode::EscapePoint | Opcode::Materialize if !node.inputs.is_empty() => {
                    let obj = node.inputs[0];
                    let obj_prov = prov_at(self, obj);
                    push_event(self, &obj_prov, u, false);
                }
                Opcode::Call { .. } => {
                    for &arg in &node.inputs {
                        if (arg.0 as usize) < n && sea.nodes[arg.0 as usize].ty == IrType::Ptr {
                            let arg_prov = prov_at(self, arg);
                            push_event(self, &arg_prov, u, false);
                        }
                    }
                }
                Opcode::Phi => {
                    for &d in &node.inputs {
                        if (d.0 as usize) < n && sea.nodes[d.0 as usize].ty == IrType::Ptr {
                            let d_prov = prov_at(self, d);
                            push_event(self, &d_prov, u, false);
                        }
                    }
                }
                Opcode::Return if node.inputs.len() >= 2 => {
                    let value = node.inputs[0];
                    if (value.0 as usize) < n && sea.nodes[value.0 as usize].ty == IrType::Ptr {
                        let value_prov = prov_at(self, value);
                        push_event(self, &value_prov, u, false);
                    }
                }
                _ => {}
            }
        }
    }
}

/// CEP:WHAT: Iterative Tarjan SCC over an adjacency list; returns the component id per
///   node and the component count (components numbered in reverse topological order).
/// CEP:WHY: Region inference per standard 5.4; explicit stacks keep it 38.13-safe.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: adjacency lists are complete over 0..len.
/// CEP:COST: O(R + E).
/// CEP:EVIDENCE: unit test rv8_middle::borrowck::tests::rejects_drop_cycle.
fn tarjan_scc(adj: &[Vec<u32>]) -> (Vec<u32>, u32) {
    let n = adj.len();
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp = vec![u32::MAX; n];
    let mut next_index: u32 = 0;
    let mut next_comp: u32 = 0;
    let mut work: Vec<(usize, usize)> = Vec::new();
    for s in 0..n {
        if index[s] != u32::MAX {
            continue;
        }
        index[s] = next_index;
        low[s] = next_index;
        next_index += 1;
        stack.push(s);
        on_stack[s] = true;
        work.push((s, 0));
        while let Some(top) = work.last_mut() {
            let v = top.0;
            if top.1 < adj[v].len() {
                let w = adj[v][top.1] as usize;
                top.1 += 1;
                if index[w] == u32::MAX {
                    index[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(parent) = work.last_mut() {
                    low[parent.0] = low[parent.0].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        comp[w] = next_comp;
                        if w == v {
                            break;
                        }
                    }
                    next_comp += 1;
                }
            }
        }
    }
    (comp, next_comp)
}

/// CEP:WHAT: Expands one row of a dense bitset into ascending element indices.
/// CEP:WHY: Deterministic edge enumeration from bitset adjacency.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(words + popcount).
/// CEP:EVIDENCE: exercised by all borrowck tests.
fn bits_of(bs: &[u64]) -> Vec<usize> {
    let mut out = Vec::new();
    for (w, &word) in bs.iter().enumerate() {
        let mut word = word;
        while word != 0 {
            let bit = word.trailing_zeros() as usize;
            out.push(w * 64 + bit);
            word &= word - 1;
        }
    }
    out
}

/// CEP:WHAT: Checks one graph and returns its borrow report.
/// CEP:WHY: The single entry point of rv8_middle::borrowck (standard 5.4).
/// CEP:STATUS: complete
/// CEP:FAILURE: MiddleError::Budget or MiddleError::Ir as documented; the safety verdict
///   itself is BorrowReport::conflicts (empty = accept).
/// CEP:ASSUMES: see module header for node conventions.
/// CEP:COST: documented in the module header.
/// CEP:EVIDENCE: unit tests rv8_middle::borrowck::tests::*.
pub fn check(sea: &SeaOfNodes) -> Result<BorrowReport, MiddleError> {
    sea.verify_structure()?;
    check_budget(sea.nodes.len())?;
    let mut graph = BorrowGraph::new(sea)?;
    graph.propagate(sea)?;
    graph.record_accesses(sea);
    let r = graph.regions;
    if r == 0 {
        return Ok(BorrowReport { regions: 0, conflicts: Vec::new() });
    }

    // Combined contents adjacency (mutable and immutable edges) as index lists.
    let mut adj_all: Vec<Vec<u32>> = vec![Vec::new(); r];
    for (a, row) in adj_all.iter_mut().enumerate() {
        let mut edges: BTreeSet<u32> = BTreeSet::new();
        for e in bits_of(&graph.mut_adj[a]) {
            edges.insert(e as u32);
        }
        for e in bits_of(&graph.imm_adj[a]) {
            edges.insert(e as u32);
        }
        *row = edges.into_iter().collect();
    }

    // Region inference: Tarjan SCC condensation (reverse topological numbering).
    let (comp, ncomp) = tarjan_scc(&adj_all);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); ncomp as usize];
    for (region, &c) in comp.iter().enumerate() {
        members[c as usize].push(region);
    }

    // Reverse-topological bitset propagation over the condensation: successor components
    // carry smaller ids, so ascending component order is a valid propagation order.
    let mut contents_reach: Vec<Vec<u64>> = vec![vec![0u64; graph.words]; ncomp as usize];
    let mut mut_reach: Vec<Vec<u64>> = vec![vec![0u64; graph.words]; ncomp as usize];
    for c in 0..ncomp as usize {
        for &m in &members[c] {
            bs_set(&mut contents_reach[c], m);
            bs_set(&mut mut_reach[c], m);
        }
        for &m in &members[c] {
            for e in bits_of(&graph.mut_adj[m]) {
                let sc = comp[e] as usize;
                let src = mut_reach[sc].clone();
                bs_union_into(&mut mut_reach[c], &src);
                let src = contents_reach[sc].clone();
                bs_union_into(&mut contents_reach[c], &src);
            }
            for e in bits_of(&graph.imm_adj[m]) {
                let sc = comp[e] as usize;
                let src = contents_reach[sc].clone();
                bs_union_into(&mut contents_reach[c], &src);
            }
        }
    }

    // Validation rule 1: mutable-mutable alias overlap by bitset AND intersection.
    let mut conflicts: Vec<Conflict> = Vec::new();
    let acc_m = &graph.mut_access;
    let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for a in 0..r {
        if !acc_m[a] {
            continue;
        }
        for s in bits_of(&mut_reach[comp[a] as usize]) {
            if s != a && acc_m[s] {
                pairs.insert((a.min(s), a.max(s)));
            }
        }
    }
    for (a, s) in &pairs {
        conflicts.push(Conflict {
            kind: ConflictKind::MutMutAlias,
            node: graph.roots[*a],
            other: graph.roots[*s],
        });
    }

    // Validation rule 2: use after move, per region in node order.
    for region in 0..r {
        let events = &graph.events[region];
        let mut first_move: Option<NodeId> = None;
        for event in events {
            if event.is_move {
                first_move = Some(match first_move {
                    Some(m) if m.0 <= event.at.0 => m,
                    _ => event.at,
                });
            } else if let Some(m) = first_move {
                if event.at.0 > m.0 {
                    conflicts.push(Conflict {
                        kind: ConflictKind::UseAfterMove,
                        node: graph.roots[region],
                        other: event.at,
                    });
                    break;
                }
            }
        }
    }

    // Validation rule 3: drop-graph cycles, restricted to regions that can reach a drop,
    // detected by iterative three-color DFS.
    let mut relevant = vec![false; r];
    let mut radj: Vec<Vec<usize>> = vec![Vec::new(); r];
    for a in 0..r {
        for e in bits_of(&graph.mut_adj[a]) {
            radj[e].push(a);
        }
        for e in bits_of(&graph.imm_adj[a]) {
            radj[e].push(a);
        }
    }
    let mut queue: Vec<usize> = (0..r).filter(|&x| graph.events[x].iter().any(|e| e.is_move)).collect();
    while let Some(x) = queue.pop() {
        if relevant[x] {
            continue;
        }
        relevant[x] = true;
        for &p in &radj[x] {
            if !relevant[p] {
                queue.push(p);
            }
        }
    }
    let mut color = vec![0u32; r]; // 0 white, 1 grey, 2 black
    let mut steps: u64 = 0;
    'dfs: for start in 0..r {
        if !relevant[start] || color[start] != 0 {
            continue;
        }
        color[start] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(top) = stack.last_mut() {
            let (v, _) = *top;
            // Advance to the next relevant outgoing edge.
            let mut next: Option<usize> = None;
            while top.1 < adj_all[v].len() {
                let w = adj_all[v][top.1] as usize;
                top.1 += 1;
                if relevant[w] {
                    next = Some(w);
                    break;
                }
            }
            steps += 1;
            if steps > MAX_BORROW_DFS_STEPS {
                return Err(MiddleError::Budget {
                    budget: "MAX_BORROW_DFS_STEPS",
                    limit: MAX_BORROW_DFS_STEPS,
                    actual: steps,
                });
            }
            match next {
                Some(w) if color[w] == 1 => {
                    conflicts.push(Conflict {
                        kind: ConflictKind::DropCycle,
                        node: graph.roots[w],
                        other: graph.roots[v],
                    });
                    break 'dfs;
                }
                Some(w) if color[w] == 0 => {
                    color[w] = 1;
                    stack.push((w, 0));
                }
                Some(_) => {}
                None => {
                    color[v] = 2;
                    stack.pop();
                }
            }
        }
    }

    conflicts.sort();
    Ok(BorrowReport { regions: r, conflicts })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Appends a node with a type.
    /// CEP:WHY: Test fixture helper; borrowck needs no block assignments.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn n(sea: &mut SeaOfNodes, op: Opcode, ty: IrType, inputs: Vec<NodeId>) -> NodeId {
        sea.add_node(op, ty, inputs)
    }

    /// CEP:WHAT: Two independent allocations each written once with a scalar, read, and
    /// returned through scalars: accepted.
    /// CEP:WHY: Baseline accept for the region machinery.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn accepts_independent_allocs() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let b = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let one = n(&mut sea, Opcode::Const(1), IrType::I32, vec![]);
        let two = n(&mut sea, Opcode::Const(2), IrType::I32, vec![]);
        let sa = n(&mut sea, Opcode::Store, IrType::Unit, vec![one, a]);
        let sb = n(&mut sea, Opcode::Store, IrType::Unit, vec![two, b]);
        let la = n(&mut sea, Opcode::Load, IrType::I32, vec![a]);
        let lb = n(&mut sea, Opcode::Load, IrType::I32, vec![b]);
        let add = n(&mut sea, Opcode::Add, IrType::I32, vec![la, lb]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![add, sa, sb]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(report.is_accepted(), "conflicts: {:?}", report.conflicts);
            assert_eq!(report.regions, 2);
        }
    }

    /// CEP:WHAT: An object holding a pointer to another object that is never itself
    ///   mutably accessed is accepted (one-sided alias).
    /// CEP:WHY: Mut-mut validation fires only on two mutably-accessed aliased regions.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn accepts_shared_read_alias() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let b = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        // b holds a pointer to a; a is never stored to directly.
        let sb = n(&mut sea, Opcode::Store, IrType::Unit, vec![a, b]);
        let la = n(&mut sea, Opcode::Load, IrType::Ptr, vec![b]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![la, sb]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(report.is_accepted(), "conflicts: {:?}", report.conflicts);
            // Regions: both allocs plus the Ptr-typed load root.
            assert_eq!(report.regions, 3);
        }
    }

    /// CEP:WHAT: A drop with no subsequent use of the object is accepted.
    /// CEP:WHY: Move tracking must accept the legal move pattern.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn accepts_move_without_reuse() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let d = n(&mut sea, Opcode::Drop, IrType::Unit, vec![a]);
        let zero = n(&mut sea, Opcode::Const(0), IrType::I32, vec![]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![zero, d]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(report.is_accepted(), "conflicts: {:?}", report.conflicts);
        }
    }

    /// CEP:WHAT: Two mutably-accessed regions where one mutably holds the other are
    ///   rejected with MutMutAlias (bitset AND overlap), deterministically.
    /// CEP:WHY: One rejection per validation rule, per task requirements.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_mutable_alias_overlap() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let b = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        // b mutably holds a; both b and a are mutably accessed.
        let s1 = n(&mut sea, Opcode::Store, IrType::Unit, vec![a, b]);
        let one = n(&mut sea, Opcode::Const(1), IrType::I32, vec![]);
        let s2 = n(&mut sea, Opcode::Store, IrType::Unit, vec![one, a]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![s1, s2]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(!report.is_accepted());
            assert_eq!(report.conflicts.len(), 1, "one deduplicated pair: {:?}", report.conflicts);
            assert_eq!(report.conflicts[0].kind, ConflictKind::MutMutAlias);
            assert_eq!(report.conflicts[0].node, a);
            assert_eq!(report.conflicts[0].other, b);
        }
    }

    /// CEP:WHAT: Loading from an object after dropping it is rejected with UseAfterMove,
    ///   deterministically.
    /// CEP:WHY: One rejection per validation rule, per task requirements.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_use_after_move() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let d = n(&mut sea, Opcode::Drop, IrType::Unit, vec![a]);
        let l = n(&mut sea, Opcode::Load, IrType::I32, vec![a]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![l, d]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(!report.is_accepted());
            assert_eq!(report.conflicts.len(), 1, "{:?}", report.conflicts);
            assert_eq!(report.conflicts[0].kind, ConflictKind::UseAfterMove);
            assert_eq!(report.conflicts[0].node, a);
            assert_eq!(report.conflicts[0].other, l);
        }
    }

    /// CEP:WHAT: Two objects holding pointers to each other, one of which is dropped, are
    ///   rejected with DropCycle via the three-color DFS, deterministically.
    /// CEP:WHY: One rejection per validation rule, per task requirements.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn rejects_drop_cycle() {
        let mut sea = SeaOfNodes::new();
        let a = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let b = n(&mut sea, Opcode::Alloc, IrType::Ptr, vec![]);
        let s1 = n(&mut sea, Opcode::Store, IrType::Unit, vec![a, b]); // b holds a
        let s2 = n(&mut sea, Opcode::Store, IrType::Unit, vec![b, a]); // a holds b
        let d = n(&mut sea, Opcode::Drop, IrType::Unit, vec![a]);
        let ret = n(&mut sea, Opcode::Return, IrType::Unit, vec![s1, s2, d]);
        let _ = ret;
        for _ in 0..2 {
            let report = check(&sea).expect("check");
            assert!(!report.is_accepted());
            assert!(
                report.conflicts.iter().any(|c| c.kind == ConflictKind::DropCycle),
                "conflicts: {:?}",
                report.conflicts
            );
        }
    }
}
