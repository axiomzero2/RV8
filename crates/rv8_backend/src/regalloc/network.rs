// CEP:FILE: crates/rv8_backend/src/regalloc/network.rs
// CEP:WHAT: The residual flow-network primitives of the MCF allocator: half-edge
//   adjacency, flow application and cancellation, Bellman-Ford potentials with a
//   negative-cycle witness, cycle extraction and cancellation, reduced-cost Dijkstra,
//   and parent-chain path reconstruction. All traversals are iterative and every
//   budget is a named constant in the module root.
// CEP:WHY: The solver kernel shared by the class solves; isolating it keeps the
//   orchestration (solve_class) auditable separately from the graph mechanics.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: every primitive returns Result; Internal marks by-construction
//   invariant violations (compiler bugs), BudgetExceeded the named budgets.
// CEP:ASSUMES: half-edge pairs are always created together by Graph::add_edge; costs
//   are integers (strict monotone decrease per cycle cancellation).
// CEP:COST: Dijkstra O(E log N) per augmentation; Bellman-Ford O(N*E) per pass.
// CEP:EVIDENCE: the brute-force optimality corpora and every allocation test.
use super::{MAX_MCF_BF_STEPS, MAX_MCF_CYCLE_CANCELLATIONS};
use crate::error::BackendError;

// Flow network
// ---------------------------------------------------------------------------

/// CEP:WHAT: One residual half-edge: target, cost (negative for backward half-edges),
/// remaining residual capacity, and the index of its paired reverse half-edge.
/// CEP:WHY: Standard residual-graph representation for min-cost flow.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: pairs are always added together by Graph::add_edge.
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HalfEdge {
    pub(crate) to: usize,
    pub(crate) cost: i64,
    pub(crate) resid: u64,
    pub(crate) rev: usize,
}

/// CEP:WHAT: Residual graph: adjacency of half-edges; forward half-edges carry +cost,
/// backward ones -cost; resid > 0 means traversable.
/// CEP:WHY: Shared by warm start, Bellman-Ford, Dijkstra, and cycle cancellation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(nodes + edges).
/// CEP:EVIDENCE: internal.
#[derive(Clone, Debug, Default)]
pub(crate) struct Graph {
    pub(crate) adj: Vec<Vec<HalfEdge>>,
}

/// CEP:WHAT: Global half-edge id: (node << 32) | adjacency index.
/// CEP:WHY: Paths and predecessor links store flat ids.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: adjacency index < 2^32.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
pub(crate) fn encode(u: usize, idx: usize) -> usize {
    (u << 32) | idx
}

/// CEP:WHAT: Decodes a global half-edge id into (node, adjacency index).
/// CEP:WHY: Counterpart of encode.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: id came from encode.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
pub(crate) fn decode(he: usize) -> (usize, usize) {
    (he >> 32, he & 0xFFFF_FFFF)
}

impl Graph {
    /// CEP:WHAT: Creates a graph with n nodes.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n).
    /// CEP:EVIDENCE: internal.
    pub(crate) fn new(n: usize) -> Graph {
        Graph { adj: vec![Vec::new(); n] }
    }

    /// CEP:WHAT: Adds a capacity/cost edge and its reverse half-edge; returns the global
    /// id of the forward half-edge.
    /// CEP:WHY: Single edge-construction path keeps pairing consistent.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) amortized.
    /// CEP:EVIDENCE: internal.
    pub(crate) fn add_edge(&mut self, u: usize, v: usize, cap: u64, cost: i64) -> usize {
        let fu = self.adj[u].len();
        let fv = self.adj[v].len();
        self.adj[u].push(HalfEdge { to: v, cost, resid: cap, rev: fv });
        self.adj[v].push(HalfEdge { to: u, cost: -cost, resid: 0, rev: fu });
        encode(u, fu)
    }

    /// CEP:WHAT: Applies +1 (route) or -1 (cancel) units of flow along a half-edge path.
    /// CEP:WHY: All flow mutations funnel through here.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Internal when a finite-capacity edge would go negative (impossible
    /// by construction; honest bug signal).
    /// CEP:ASSUMES: path edges had residual capacity when routed.
    /// CEP:COST: O(path).
    /// CEP:EVIDENCE: internal.
    pub(crate) fn apply(&mut self, path: &[usize], delta: i64) -> Result<(), BackendError> {
        for &he in path {
            let (u, idx) = decode(he);
            let rev = self.adj[u][idx].rev;
            let to = self.adj[u][idx].to;
            if delta > 0 {
                if self.adj[u][idx].resid == 0 {
                    return Err(BackendError::Internal { detail: format!(
                        "routing through saturated half-edge {}->{}", u, to) });
                }
                self.adj[u][idx].resid -= 1;
                self.adj[to][rev].resid += 1;
            } else {
                if self.adj[to][rev].resid == 0 {
                    return Err(BackendError::Internal { detail: format!(
                        "canceling flow absent on {}->{}", to, u) });
                }
                self.adj[to][rev].resid -= 1;
                self.adj[u][idx].resid += 1;
            }
        }
        Ok(())
    }

    /// CEP:WHAT: True when one unit of flow crosses the forward half-edge (capacity-1
    /// edges carry flow iff their residual hit 0).
    /// CEP:WHY: Attribution reads final flows, not recorded paths, so cycle
    /// cancellation cannot desynchronize labels.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: edge capacity == 1.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: internal.
    pub(crate) fn flow_on(&self, he: usize) -> bool {
        let (u, idx) = decode(he);
        self.adj[u][idx].resid == 0
    }
}

/// CEP:WHAT: Return bundle of bellman_ford: distances, predecessor links, and a
///   negative-cycle witness.
/// CEP:WHY: A named alias keeps the signatures readable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
pub(crate) type BellmanFordResult = (Vec<i64>, Vec<Option<usize>>, Option<usize>);

/// CEP:WHAT: Bellman-Ford with super-source semantics (all distances start at 0):
/// returns distances, predecessor half-edge links, and a negative-cycle witness node
/// (Some only when a relaxation still fires on the final sweep).
/// CEP:WHY: STANDARDS 5.1 mandates potential initialization by one Bellman-Ford pass;
/// super-source initialization handles warm-start residuals with unreachable nodes and
/// doubles as the negative-cycle detector for the cancellation pass.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past MAX_MCF_BF_STEPS.
/// CEP:ASSUMES: none
/// CEP:COST: O(N*E) worst case with early exit.
/// CEP:EVIDENCE: corpus optimality (valid potentials are Dijkstra's precondition).
pub(crate) fn bellman_ford(
    g: &Graph,
    steps: &mut u64,
) -> Result<BellmanFordResult, BackendError> {
    let n = g.adj.len();
    let mut dist = vec![0i64; n];
    let mut pred: Vec<Option<usize>> = vec![None; n];
    let mut witness: Option<usize> = None;
    for iteration in 0..n {
        let mut changed = false;
        for u in 0..n {
            for (idx, e) in g.adj[u].iter().enumerate() {
                *steps += 1;
                if *steps > MAX_MCF_BF_STEPS {
                    return Err(BackendError::BudgetExceeded { budget: "MAX_MCF_BF_STEPS" });
                }
                if e.resid == 0 {
                    continue;
                }
                let nd = dist[u].saturating_add(e.cost);
                if nd < dist[e.to] {
                    dist[e.to] = nd;
                    pred[e.to] = Some(encode(u, idx));
                    changed = true;
                    if iteration == n - 1 {
                        witness = Some(e.to);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    Ok((dist, pred, witness))
}

/// CEP:WHAT: Walks predecessor links from a negative-cycle witness onto the cycle and
/// returns its half-edges.
/// CEP:WHY: Cycle cancellation needs the cycle's edges.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal if the walk fails to close (pred invariant broken).
/// CEP:ASSUMES: witness is on or reachable-from a negative cycle via pred links.
/// CEP:COST: O(N).
/// CEP:EVIDENCE: pressure test exercises deep solves; corpus stays cancellation-free.
fn extract_cycle(
    g: &Graph,
    pred: &[Option<usize>],
    witness: usize,
) -> Result<Vec<usize>, BackendError> {
    let n = g.adj.len();
    // Walk back n steps: guaranteed to land on a cycle when a negative cycle exists.
    let mut x = witness;
    for _ in 0..n {
        match pred[x] {
            Some(e) => {
                let (u, _) = decode(e);
                x = u;
            }
            None => return Err(BackendError::Internal { detail: "cycle walk hit a root".into() }),
        }
    }
    let mut cycle = Vec::new();
    let mut cur = x;
    for _ in 0..=n {
        match pred[cur] {
            Some(e) => {
                cycle.push(e);
                let (u, _) = decode(e);
                cur = u;
                if cur == x {
                    return Ok(cycle);
                }
            }
            None => break,
        }
    }
    Err(BackendError::Internal { detail: "negative cycle did not close".into() })
}

/// CEP:WHAT: Cancels residual negative cycles until the flow is min-cost (none remain),
/// under the MAX_MCF_CYCLE_CANCELLATIONS budget, and returns Bellman-Ford potentials from
/// the final witness-free pass (valid Johnson potentials by construction).
/// CEP:WHY: A warm start (or the residual left after canceling one commodity's path) can
/// contain negative cycles; potentials are only valid on a cycle-free residual, so the
/// cleanse must run before every reduced-cost Dijkstra. The cycle-canceling optimality
/// theorem then guarantees the cleaned flow of value v is THE min-cost flow of value v,
/// which is what makes the subsequent SSP augmentation optimal.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past MAX_MCF_CYCLE_CANCELLATIONS; Internal for pred-chain
/// violations (compiler bugs). Each cancellation strictly decreases the integer cost, so
/// the loop terminates.
/// CEP:ASSUMES: integer costs (strict decrease per cancellation).
/// CEP:COST: O(cancellations x N x E).
/// CEP:EVIDENCE: corpus optimality; pressure test (which exposes a negative cycle in the
/// warm residual and passes only when the cleanse runs).
pub(crate) fn cancel_negative_cycles(
    g: &mut Graph,
    steps: &mut u64,
) -> Result<(Vec<i64>, Vec<Option<usize>>), BackendError> {
    for _ in 0..MAX_MCF_CYCLE_CANCELLATIONS {
        let (dist, pred, witness) = bellman_ford(g, steps)?;
        match witness {
            None => return Ok((dist, pred)),
            Some(w) => {
                let cycle = extract_cycle(g, &pred, w)?;
                g.apply(&cycle, 1)?;
            }
        }
    }
    Err(BackendError::BudgetExceeded { budget: "MAX_MCF_CYCLE_CANCELLATIONS" })
}

/// CEP:WHAT: Return bundle of dijkstra: optional distances and parent links.
/// CEP:WHY: A named alias keeps the signatures readable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
pub(crate) type DijkstraResult = (Vec<Option<i64>>, Vec<Option<usize>>);

/// CEP:WHAT: Dijkstra over residual half-edges with Johnson reduced costs
/// (`c + pot[u] - pot[v]`); returns distances and parent links. The heap orders by
/// (distance, node id), so ties pop the lowest node id first: deterministic.
/// CEP:WHY: STANDARDS 5.1: successive shortest paths use reduced-cost Dijkstra with
/// deterministic tie-breaking.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal on a negative reduced cost (invalid potentials - compiler bug).
/// CEP:ASSUMES: pot satisfies the Johnson condition on every residual half-edge.
/// CEP:COST: O(E log N).
/// CEP:EVIDENCE: corpus optimality.
pub(crate) fn dijkstra(
    g: &Graph,
    src: usize,
    pot: &[i64],
) -> Result<DijkstraResult, BackendError> {
    let n = g.adj.len();
    let mut dist: Vec<Option<i64>> = vec![None; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut heap = std::collections::BinaryHeap::new();
    dist[src] = Some(0);
    heap.push(std::cmp::Reverse((0i64, src)));
    while let Some(std::cmp::Reverse((d, u))) = heap.pop() {
        if dist[u].is_some() && dist[u] < Some(d) {
            continue; // stale heap entry
        }
        for (idx, e) in g.adj[u].iter().enumerate() {
            if e.resid == 0 {
                continue;
            }
            let red = e.cost + pot[u] - pot[e.to];
            if red < 0 {
                return Err(BackendError::Internal { detail: format!(
                    "negative reduced cost {} on half-edge {}->{} (invalid potentials)",
                    red, u, e.to) });
            }
            let nd = d + red;
            if dist[e.to].is_none_or(|cur| nd < cur) {
                dist[e.to] = Some(nd);
                parent[e.to] = Some(encode(u, idx));
                heap.push(std::cmp::Reverse((nd, e.to)));
            }
        }
    }
    Ok((dist, parent))
}

/// CEP:WHAT: Reconstructs the half-edge path from parent links.
/// CEP:WHY: Path recording per commodity.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal on a broken parent chain.
/// CEP:ASSUMES: parent links form a tree toward src.
/// CEP:COST: O(path).
/// CEP:EVIDENCE: internal.
pub(crate) fn path_of(parent: &[Option<usize>], src: usize, snk: usize) -> Result<Vec<usize>, BackendError> {
    let mut path = Vec::new();
    let mut cur = snk;
    let bound = parent.len() + 1;
    let mut guard = 0usize;
    while cur != src {
        guard += 1;
        if guard > bound {
            return Err(BackendError::Internal { detail: "parent chain did not reach src".into() });
        }
        match parent[cur] {
            Some(e) => {
                path.push(e);
                let (u, _) = decode(e);
                cur = u;
            }
            None => return Err(BackendError::Internal { detail: "path parent missing".into() }),
        }
    }
    path.reverse();
    Ok(path)
}
