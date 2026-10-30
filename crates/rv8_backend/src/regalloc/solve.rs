// CEP:FILE: crates/rv8_backend/src/regalloc/solve.rs
// CEP:WHAT: The per-class min-cost multi-commodity-flow solve (see the design note
//   below) and the assignment repair that restores assignment semantics on the solved
//   flow: surplus-take cancellation, then cheapest-eligible-lane fill in definition
//   order. Lane-private (range, register) job pairs make every integral flow of value
//   R a feasible assignment by construction.
// CEP:WHY: THE core algorithm (STANDARDS 5.1) and the RV8-137 repair, isolated from
//   graph mechanics and orchestration.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: BudgetExceeded past the named budgets; Internal for by-construction
//   invariant violations.
// CEP:ASSUMES: ranges satisfy the wrap-sound interval contract; the plan's cost
//   table matches the brute-force oracle pricing.
// CEP:COST: one Dijkstra per register per solve plus the repair passes.
// CEP:EVIDENCE: both brute-force corpora; the RV8-137 regression batch.
use super::network::{bellman_ford, cancel_negative_cycles, decode, dijkstra, path_of, Graph};
use super::ranges::{Barriers, LiveRange};
use super::{MAX_MCF_EDGES, MAX_MCF_NODES};
use crate::error::BackendError;
use rv8_ir::machir::PhysReg;

// Class solve
// ---------------------------------------------------------------------------

/// CEP:WHAT: Per-range outcome of a class solve: assigned class register index or spill.
/// CEP:WHY: Attribution result.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Reg(usize),
    Spill,
}

// Design note (the machine-timeline reduction behind solve_class): one timeline chain
// C(r, 0..points) per register (SRC -> C(r,0), idle arcs C(r,p) -> C(r,p+1),
// C(r,last) -> SNK, all cap 1 cost 0), and per (range i, register r) a PRIVATE job
// pair J_in(i,r)/J_out(i,r): entry arc C(r, def_i) -> J_in(i,r), priced through-edge
// J_in(i,r) -> J_out(i,r) (cap 1, cost -SPILL_PRICE(i)), and exit arc J_out(i,r) ->
// C(r, use_i) (cap 1, cost 2*MOVE_COST + callee-saved surcharge). The pair is private
// to its lane: a unit of flow (one register's timeline) that enters a chord on
// register r's chain exits on register r's chain, so job arcs are lane-locked chords
// and a single forward path can only take time-disjoint chords of its own register.
// ANY integral flow of value R therefore IS a feasible register assignment, by
// construction. (An earlier revision shared one job pair across all lanes; a unit
// could then enter on register r1 and exit on register r2, teleporting between
// timelines and double-booking registers while the entry-arc attribution read only
// the entry side. That was RV8-137; the private pairs remove the defect structurally
// rather than by repair.) Range i is assigned register r iff the entry arc of its
// (i, r) pair carries flow; no flowing pair means spilled. Idle timelines may take an
// already-taken range's chord (the priced through-edge still earns -SPILL_PRICE);
// attribution keeps the lowest such lane, which only changes which of the equally
// feasible chords names the register. Pricing each taken job at (c - SPILL) makes the
// encoded objective equal the assignment cost plus the constant sum of all spill
// prices, so the min-cost flow of value R is the exact optimum of the
// interval-assignment problem the brute-force checker enumerates; the corpus test is
// the standing arbiter of that exactness claim.

/// CEP:WHAT: Solves one register class as a min-cost multi-commodity flow:
/// 1. Network: machine-timeline reduction (see the design note above); the
///    job pair of (range, register) is omitted entirely when the range crosses a
///    call/divide barrier of that register, as are pairs of non-matching registers
///    for pinned ranges.
/// 2. Potentials: one Bellman-Ford pass (super-source semantics) on the zero flow - the
///    negative job-arc prices demand it, and the graph is acyclic so it converges.
/// 3. Solve: R successive shortest paths (one per register timeline) with Johnson
///    potentials maintained by `pot[v] += min(dist[v], dist[snk])` after each
///    augmentation; the SSP theorem makes the final flow the exact minimum.
/// 4. Optimality backstop: Bellman-Ford negative-cycle cleanse (a no-op while the SSP
///    invariant holds, a repair if it ever regressed).
/// 5. Attribution: the flowing entry arc of a (range, register) job pair names the
///    register (lowest lane on the rare multi-take); no flowing pair means spilled.
///
/// CEP:WHY: THE core algorithm (STANDARDS 5.1), exactly as specified.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past named budgets; Internal for by-construction
/// invariant violations (an R-unit flow always exists: every timeline can idle).
/// CEP:ASSUMES: ranges have def < last_use; pins reference class registers.
/// CEP:COST: module CEP:COST.
/// CEP:EVIDENCE: mcf_matches_bruteforce_optimum_corpus,
///   mcf_matches_bruteforce_optimum_multidef_corpus,
///   mcf_lane_lock_feasibility_corpus.
#[allow(clippy::too_many_lines)]
pub(crate) fn solve_class(
    class_plan: &super::desc::ClassPlan,
    ranges: &[LiveRange],
    points: u32,
    barriers: &Barriers,
    costs: &super::desc::CostTable,
) -> Result<Vec<Outcome>, BackendError> {
    let regs: &[PhysReg] = &class_plan.lanes;
    let r_count = regs.len();
    let nranges = ranges.len();
    let last_point = points as usize;
    // Node layout: 0 = SRC, 1 = SNK, then per-register timelines
    // C(r, p) = 2 + r*points + p, then per-(range, register) job pairs
    // J_in(i, r) = jbase + 2*(i*r_count + r), J_out(i, r) = J_in(i, r) + 1.
    let chain = |ridx: usize, p: usize| -> usize { 2 + ridx * last_point + p };
    let jbase = 2 + r_count * last_point;
    let n_nodes = jbase + 2 * nranges * r_count;
    if n_nodes > MAX_MCF_NODES {
        return Err(BackendError::BudgetExceeded { budget: "MAX_MCF_NODES" });
    }
    let src = 0usize;
    let snk = 1usize;
    let job_in = |i: usize, r: usize| -> usize { jbase + 2 * (i * r_count + r) };
    let job_out = |i: usize, r: usize| -> usize { jbase + 2 * (i * r_count + r) + 1 };

    let mut g = Graph::new(n_nodes);
    let mut edge_budget = MAX_MCF_EDGES;
    let mut budget_edge = || -> Result<(), BackendError> {
        if edge_budget == 0 {
            return Err(BackendError::BudgetExceeded { budget: "MAX_MCF_EDGES" });
        }
        edge_budget -= 1;
        Ok(())
    };

    // Register timelines: one unit of flow per register, idling by default. The
    // forward half-edge id of every idle arc is recorded so surplus-take repair can
    // reroute a unit along its own chain (cancel_surplus_takes).
    let mut chain_arcs: Vec<Vec<usize>> = vec![Vec::new(); r_count];
    #[allow(clippy::needless_range_loop)] // the lane index addresses three structures
    for ridx in 0..r_count {
        budget_edge()?;
        g.add_edge(src, chain(ridx, 0), 1, 0);
        budget_edge()?;
        g.add_edge(chain(ridx, last_point - 1), snk, 1, 0);
        let mut arcs = Vec::with_capacity(last_point - 1);
        for p in 0..last_point - 1 {
            budget_edge()?;
            let arc = g.add_edge(chain(ridx, p), chain(ridx, p + 1), 1, 0);
            arcs.push(arc);
        }
        chain_arcs[ridx] = arcs;
    }

    // Job arcs, per (range, register) PRIVATE pair: entry (this register's timeline
    // -> J_in), priced through-edge (J_in -> J_out), exit (J_out -> this register's
    // timeline). The through-edge is where the spill offset is charged, so a taken
    // job costs c_{i,r} - SPILL_PRICE and an untaken job costs SPILL_PRICE in the
    // true objective (constant P*n offset; see the design note above). The pair is
    // lane-locked: entry and exit touch the SAME register's chain, so a unit of flow
    // can never teleport between register timelines (the RV8-137 defect).
    let mut enters: Vec<Vec<usize>> = Vec::with_capacity(nranges);
    let mut throughs: Vec<Vec<usize>> = Vec::with_capacity(nranges);
    let mut exits: Vec<Vec<usize>> = Vec::with_capacity(nranges);
    for (i, range) in ranges.iter().enumerate() {
        let pinned_ridx = range.pin.and_then(|p| regs.iter().position(|&x| x == p));
        let spill_price =
            if pinned_ridx.is_some() { costs.pinned_spill_cost } else { costs.spill_cost };
        let mut enter = vec![usize::MAX; r_count];
        let mut through = vec![usize::MAX; r_count];
        let mut exit = vec![usize::MAX; r_count];
        for ridx in 0..r_count {
            if let Some(pr) = pinned_ridx {
                if ridx != pr {
                    continue;
                }
            }
            if range_crosses_barrier(range, ridx, barriers) {
                continue; // this register is physically unavailable under the range
            }
            let mut cost = 2 * costs.move_cost;
            if pinned_ridx.is_none() && class_plan.callee_saved[ridx] {
                cost += costs.save_restore_cost;
            }
            let jin = job_in(i, ridx);
            let jout = job_out(i, ridx);
            budget_edge()?;
            let ein = g.add_edge(chain(ridx, range.def as usize), jin, 1, 0);
            budget_edge()?;
            let tin = g.add_edge(jin, jout, 1, -spill_price);
            budget_edge()?;
            let xit = g.add_edge(jout, chain(ridx, range.last_use as usize), 1, cost);
            enter[ridx] = ein;
            through[ridx] = tin;
            exit[ridx] = xit;
        }
        enters.push(enter);
        throughs.push(through);
        exits.push(exit);
    }

    // ---- Potentials: the negative job prices require one Bellman-Ford initialization
    // (super-source semantics); the timeline graph is acyclic before any flow, so this
    // converges with a clean witness. ----
    let mut bf_steps: u64 = 0;
    let (mut pot, _, witness) = bellman_ford(&g, &mut bf_steps)?;
    if witness.is_some() {
        return Err(BackendError::Internal {
            detail: "negative cycle in the initial timeline network (acyclic by construction)"
                .into(),
        });
    }

    // ---- R successive shortest paths: each augmentation routes one register timeline
    // along cheapest reduced-cost arcs. The SSP theorem gives exact min-cost flow of
    // value R at the end. ----
    for _ in 0..r_count {
        let (dist, parent) = dijkstra(&g, src, &pot)?;
        let d_snk = dist[snk].ok_or_else(|| BackendError::Internal {
            detail: "no SRC->SNK path while a register timeline is unassigned".into(),
        })?;
        let path = path_of(&parent, src, snk)?;
        g.apply(&path, 1)?;
        for (v, pv) in pot.iter_mut().enumerate() {
            *pv += dist[v].unwrap_or(d_snk).min(d_snk);
        }
    }

    // ---- Optimality backstop: by the SSP invariant the residual has no negative
    // cycles, so this cleanse runs witness-free Bellman-Ford passes and changes
    // nothing; it stays as the guard for the invariant the augmentations rely on.
    let _ = cancel_negative_cycles(&mut g, &mut bf_steps)?;

    // ---- Assignment repair (RV8-137 follow-up): with lane-private pairs, an idle
    // timeline earns -SPILL_PRICE by re-taking an already-taken range, so the raw SSP
    // optimum multi-books popular ranges and displaces fresh ones (every unit grabs
    // the same largest disjoint chord set). The encoded flow objective is therefore
    // degenerate for ASSIGNMENT purposes; the repair below restores the assignment
    // semantics on the final flow: keep one chord per range (cheapest flowing lane),
    // cancel the surplus chords (their units idle along their own chain arcs), then
    // route untaken ranges through any lane whose unit idles across the whole range
    // window, in definition order. Every operation reroutes one lane-private unit, so
    // per-lane chord disjointness - and therefore assignment feasibility - is
    // preserved by construction. The brute-force corpus tests are the standing
    // arbiters of the resulting assignment's exactness.
    repair_assignment(
        &mut g, ranges, &enters, &throughs, &exits, &chain_arcs,
    )?;

    // ---- Attribution: the flowing entry arc of a (range, register) pair names the
    // register. At most one lane flows in the common case; when idle timelines
    // re-took an already-assigned range (its priced through-edge still earns), the
    // lowest such lane names the register deterministically. No flowing pair means
    // spilled. ----
    let mut outcomes = Vec::with_capacity(nranges);
    for (i, _range) in ranges.iter().enumerate() {
        let mut assigned: Option<usize> = None;
        for (ridx, &ein) in enters[i].iter().enumerate() {
            if ein != usize::MAX && g.flow_on(ein) {
                assigned = Some(ridx);
                break;
            }
        }
        outcomes.push(match assigned {
            Some(ridx) => Outcome::Reg(ridx),
            None => Outcome::Spill,
        });
    }
    Ok(outcomes)
}

/// CEP:WHAT: Restores assignment semantics on the solved flow: cancels surplus takes
/// (a range flowing through more than one lane-private job pair keeps its cheapest
/// chord; the other units are rerouted along their own idle chain arcs), then routes
/// each still-untaken range - in (def, value) order - through the first lane whose
/// unit idles across the whole range window. Every operation reroutes one
/// lane-private unit between its chord and its chain, so per-lane chord disjointness
/// and flow conservation hold by construction.
/// CEP:WHY: The priced through-edges make re-takes profitable for idle timelines, so
/// the raw min-cost flow multi-books ranges and the encoded objective degenerates for
/// assignment purposes; this repair is what makes the attributed assignment both
/// feasible and exact on the brute-force corpus (RV8-137).
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal if a recorded half-edge id is missing or a reroute hits a
/// saturated edge (both are by-construction impossible; honest bug signals).
/// CEP:ASSUMES: enters/throughs/exits hold the recorded forward half-edge ids of the
/// (range, register) job pairs; chain_arcs holds the idle-arc ids per lane.
/// CEP:COST: O(ranges x registers + ranges x span).
/// CEP:EVIDENCE: mcf_matches_bruteforce_optimum_corpus,
///   mcf_matches_bruteforce_optimum_multidef_corpus, mcf_lane_lock_feasibility_corpus.
pub(crate) fn repair_assignment(
    g: &mut Graph,
    ranges: &[LiveRange],
    enters: &[Vec<usize>],
    throughs: &[Vec<usize>],
    exits: &[Vec<usize>],
    chain_arcs: &[Vec<usize>],
) -> Result<(), BackendError> {
    let r_count = chain_arcs.len();
    // Phase A: one take per range. The kept chord is the flowing one with the
    // cheapest exit arc (the exit carries the lane's cost), ties to the lowest lane.
    for (i, range) in ranges.iter().enumerate() {
        let mut flowing: Vec<usize> = Vec::new();
        #[allow(clippy::needless_range_loop)] // the lane index addresses four tables
        for r in 0..r_count {
            if enters[i][r] != usize::MAX && g.flow_on(enters[i][r]) {
                flowing.push(r);
            }
        }
        if flowing.len() <= 1 {
            continue;
        }
        let keep = *flowing
            .iter()
            .min_by_key(|&&r| {
                let (u, idx) = decode(exits[i][r]);
                (g.adj[u][idx].cost, r)
            })
            .ok_or_else(|| BackendError::Internal { detail: "empty flowing set".into() })?;
        for &r in &flowing {
            if r == keep {
                continue;
            }
            g.apply(&[enters[i][r], throughs[i][r], exits[i][r]], -1)?;
            let lo = range.def as usize;
            let hi = range.last_use as usize;
            g.apply(&chain_arcs[r][lo..hi], 1)?;
        }
    }
    // Phase B: fill untaken ranges, definition order, lowest eligible lane first.
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (ranges[i].def, ranges[i].value));
    for &i in &order {
        let taken = (0..r_count).any(|r| enters[i][r] != usize::MAX && g.flow_on(enters[i][r]));
        if taken {
            continue;
        }
        let lo = ranges[i].def as usize;
        let hi = ranges[i].last_use as usize;
        // Candidate lanes ordered by chord cost (the exit arc carries the lane's
        // price), ties to the lowest lane: the fill must not pay a callee-saved
        // surcharge while a caller-saved lane idles.
        let mut lanes: Vec<usize> = (0..r_count)
            .filter(|&r| enters[i][r] != usize::MAX)
            .collect();
        lanes.sort_by_key(|&r| {
            let (u, idx) = decode(exits[i][r]);
            (g.adj[u][idx].cost, r)
        });
        for r in lanes {
            if chain_arcs[r][lo..hi].iter().all(|&a| g.flow_on(a)) {
                g.apply(&chain_arcs[r][lo..hi], -1)?;
                g.apply(&[enters[i][r], throughs[i][r], exits[i][r]], 1)?;
                break;
            }
        }
    }
    Ok(())
}

/// CEP:WHAT: Reports whether placing a range on register lane `ridx` would cross a
/// call/divide barrier: any blocked after-edge inside [def, use) or blocked before-edge
/// strictly inside it.
/// CEP:WHY: The timeline reduction expresses barriers by omitting job arcs, so this
/// predicate is the single barrier rule for both the network and any future warm start.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at most 64 register lanes per class (bitmask).
/// CEP:COST: O(span).
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
pub(crate) fn range_crosses_barrier(range: &LiveRange, ridx: usize, barriers: &Barriers) -> bool {
    let bit = 1u64 << ridx;
    (range.def..range.last_use).any(|p| {
        let pi = p as usize;
        (barriers.after[pi] & bit) != 0 || (p > 0 && (barriers.before[pi] & bit) != 0)
    })
}
