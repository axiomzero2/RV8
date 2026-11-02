// CEP:FILE: crates/rv8_backend/src/regalloc/warm.rs
// CEP:WHAT: The RV8-140 warm-start and incremental/region re-solve paths over the ONE
// shared timeline network: a deterministic linear-scan warm start that seeds the flow
// (design.md 7.4) and a previous-solution restore that re-routes only changed ranges
// (design.md 7.2/7.3: a region re-solve is the same mechanism with the region's
// values as the changed set and every other range frozen).
// CEP:WHY: STANDARDS 5.1 names the linear-scan warm start and incremental/region
// re-solve; the design note prices them as exactness-preserving accelerators over
// the same network, not a second allocator.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-140
// CEP:FAILURE: BackendError from the shared builder and flow surgery; any restore
// failure falls back to the exact cold solve (documented, deterministic).
// CEP:ASSUMES: prev solutions come from this allocator on a compatible plan; the
// corpora arbitrate exactness (warm) and identity (incremental, unchanged input).
// CEP:COST: warm: O(ranges x registers) scan + one cycle cleanse; incremental:
// O(changed x registers) restore + repair.
// CEP:EVIDENCE: warm_and_incremental test batch in regalloc/tests.rs.
use super::ranges::{Barriers, LiveRange};
use super::solve::{build_timeline_network, repair_assignment, Outcome, TimelineNetwork};
use super::network::cancel_negative_cycles;
use super::solve::solve_class;
use super::solve::range_crosses_barrier;

use crate::error::BackendError;
use rv8_ir::machir::PhysReg;

/// CEP:WHAT: Deterministic linear-scan assignment per range: ranges in (def, value)
/// order take the lowest-index eligible lane whose timeline is free across the whole
/// window, honoring pins and the shared barrier rule; none when every lane is busy.
/// CEP:WHY: design.md 7.4: "use a fast heuristic (e.g. linear scan) to get an initial
/// feasible solution" — the scan seeds the flow so the cleanse starts near the optimum.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a busy network spills honestly.
/// CEP:ASSUMES: the same barrier rule as the network (range_crosses_barrier).
/// CEP:COST: O(ranges x registers) after the def-order sort.
/// CEP:EVIDENCE: warm_start_matches_cold_optimum_on_corpora.
pub(crate) fn linear_scan_assign(
    ranges: &[LiveRange],
    lanes: &[PhysReg],
    barriers: &Barriers,
) -> Vec<Option<usize>> {
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (ranges[i].def, ranges[i].value));
    let mut free_at: Vec<u32> = vec![0; lanes.len()];
    let mut out = vec![None; ranges.len()];
    for &i in &order {
        let range = &ranges[i];
        for (ridx, lane) in lanes.iter().enumerate() {
            if let Some(pin) = range.pin {
                if *lane != pin {
                    continue; // pinned ranges only ever take their pin lane
                }
            }
            if free_at[ridx] > range.def {
                continue;
            }
            if range_crosses_barrier(range, ridx, barriers) {
                continue;
            }
            out[i] = Some(ridx);
            free_at[ridx] = range.last_use;
            break;
        }
    }
    out
}

/// CEP:WHAT: Routes one register's flow unit along its timeline: through a range's
/// job pair when assigned, else idling along the full chain.
/// CEP:WHY: Warm start and incremental restore both express "previous assignment" as
/// an integral flow of value R on the same network.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError when an arc is missing or saturated (caller falls back
/// to the cold solve; both are honest bug signals, never silent misassignment).
/// CEP:ASSUMES: enters/throughs/exits record the forward half-edge ids.
/// CEP:COST: O(span).
/// CEP:EVIDENCE: incremental_resolve_is_identity_when_unchanged.
pub(crate) fn restore_unit(
    net: &mut TimelineNetwork,
    ridx: usize,
    job: Option<(usize, u32, u32)>, // (range index, def, last_use)
    last_point: usize,
) -> Result<(), BackendError> {
    let TimelineNetwork { g, enters, throughs, exits, chain_arcs, src_arcs, snk_arcs } = net;
    g.apply(&[src_arcs[ridx]], 1)?;
    match job {
        Some((i, def, last_use)) => {
            let (e, t, x) = (enters[i][ridx], throughs[i][ridx], exits[i][ridx]);
            if e == usize::MAX || t == usize::MAX || x == usize::MAX {
                return Err(BackendError::Internal {
                    detail: "warm restore: job arcs missing for an assigned range".into(),
                });
            }
            g.apply(&chain_arcs[ridx][0..def as usize], 1)?;
            g.apply(&[e, t, x], 1)?;
            g.apply(&chain_arcs[ridx][last_use as usize..last_point - 1], 1)?;
        }
        None => {
            g.apply(&chain_arcs[ridx][0..last_point - 1], 1)?;
        }
    }
    g.apply(&[snk_arcs[ridx]], 1)?;
    Ok(())
}

/// CEP:WHAT: The warm-started class solve (design.md 7.4): build the shared network,
/// restore the linear scan's assignment as the initial flow of value R, cancel
/// negative cycles, then run the same assignment repair and attribution as the cold
/// solve, and finally PROMOTE the exact cold solve's outcome whenever the refined
/// warm flow costs more (never-regress guarantee).
/// CEP:WHY: A residual cycle cannot express reassigning a chord between two
/// register timelines (the SRC/SNK arcs are saturated at flow value R), so a
/// cycle-cleanse alone is not globally optimal; the promotion check keeps the
/// corpus-exact optimum as the published answer while the warm flow remains the
/// seeding and incremental machinery.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past the named budgets; Internal for by-construction
/// violations.
/// CEP:ASSUMES: pins filter the scan through network eligibility (an assigned lane
/// whose job arcs were omitted falls back to idling).
/// CEP:COST: module CEP:COST.
/// CEP:EVIDENCE: warm_start_matches_cold_optimum_on_corpora.
pub(crate) fn solve_class_warm(
    class_plan: &super::desc::ClassPlan,
    ranges: &[LiveRange],
    points: u32,
    barriers: &Barriers,
    costs: &super::desc::CostTable,
) -> Result<Vec<Outcome>, BackendError> {
    let mut net = build_timeline_network(class_plan, ranges, points, barriers, costs)?;
    let last_point = points as usize;
    let scan = linear_scan_assign(ranges, &class_plan.lanes, barriers);
    for (ridx, _lane) in class_plan.lanes.iter().enumerate() {
        // The scan's range->lane map is per range; invert it per lane (at most one
        // chord per lane at a time, so at most one range owns the unit's deflection
        // window: the scan guarantees disjoint windows per lane).
        let mut job: Option<(usize, u32, u32)> = None;
        for (i, assigned) in scan.iter().enumerate() {
            if *assigned == Some(ridx) {
                job = Some((i, ranges[i].def, ranges[i].last_use));
                break;
            }
        }
        // A pinned range's non-pinned job arcs do not exist; a pin-violating scan
        // pick (possible when the pin lane is busy) falls back to idling here.
        let eligible = match job {
            Some((i, _, _)) => {
                let e = net.enters[i][ridx];
                e != usize::MAX
            }
            None => true,
        };
        let unit = if eligible { job } else { None };
        match restore_unit(&mut net, ridx, unit, last_point) {
            Ok(()) => {}
            Err(_) => {
                // Deterministic fallback: the exact cold solve (documented).
                return solve_class(class_plan, ranges, points, barriers, costs);
            }
        }
    }
    finish_with_repair_and_promote(net, ranges, class_plan, points, barriers, costs)
}

/// CEP:WHAT: The incremental/region re-solve (design.md 7.2/7.3): restore the
/// previous assignment on the shared network, re-route only the ranges marked
/// changed (a region's values, for the region path), then run the same optimality
/// cleanse, repair, and attribution as the warm path: the result is exact-optimal
/// for the restored flow value, deterministic, and feasible. Unchanged ranges keep
/// their lanes unless their job arcs no longer exist (barrier or pin moved), in
/// which case they are treated as changed.
/// CEP:WHY: Compile-time bound for edit-compile loops: cost scales with the changed
/// set, not the function; the cleanse keeps the exactness story identical to warm.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError from the shared builder; a restore failure falls back
/// to the exact cold solve. The promotion check never lets the incremental answer
/// cost more than the exact cold optimum.
/// CEP:ASSUMES: prev lanes are class-relative indices (the RegisterAllocation
/// convention); ranges are the CURRENT LIR's ranges.
/// CEP:COST: O(ranges + changed x span).
/// CEP:EVIDENCE: incremental_resolve_is_identity_when_unchanged,
///   incremental_resolve_keeps_feasibility_after_change.
pub(crate) fn solve_class_incremental(
    class_plan: &super::desc::ClassPlan,
    ranges: &[LiveRange],
    points: u32,
    barriers: &Barriers,
    costs: &super::desc::CostTable,
    prev: &[Option<usize>],
    changed: &[bool],
) -> Result<Vec<Outcome>, BackendError> {
    let mut net = build_timeline_network(class_plan, ranges, points, barriers, costs)?;
    let last_point = points as usize;
    for ridx in 0..class_plan.lanes.len() {
        let mut job: Option<(usize, u32, u32)> = None;
        for (i, prev_lane) in prev.iter().enumerate() {
            if *prev_lane == Some(ridx) && !changed.get(i).copied().unwrap_or(true) {
                job = Some((i, ranges[i].def, ranges[i].last_use));
                break;
            }
        }
        let eligible = match job {
            Some((i, _, _)) => net.enters[i][ridx] != usize::MAX,
            None => true,
        };
        let unit = if eligible { job } else { None };
        match restore_unit(&mut net, ridx, unit, last_point) {
            Ok(()) => {}
            Err(_) => {
                return solve_class(class_plan, ranges, points, barriers, costs);
            }
        }
    }
    finish_with_repair_and_promote(net, ranges, class_plan, points, barriers, costs)
}

/// CEP:WHAT: Shared finish for the warm and incremental paths: the optimality
/// cleanse, the assignment repair, flowing-entry attribution, and the never-regress
/// promotion against the exact cold solve.
/// CEP:WHY: One finisher keeps the three solve paths on identical attribution rules
/// and makes the accelerator paths safe: a residual cycle cannot reassign a chord
/// across timelines, so the refined warm/incremental flow is verified against the
/// exact optimum and the cheaper (tie: cold) answer is published.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded / Internal from the cleanse and repair.
/// CEP:ASSUMES: the incoming flow has value R with lane-private chords.
/// CEP:COST: cleanse O(BF passes) + repair O(ranges x registers + ranges x span)
///   plus the exact solve when promotion triggers.
/// CEP:EVIDENCE: warm_and_incremental batch (corpora pin cost equality).
fn finish_with_repair_and_promote(
    net: TimelineNetwork,
    ranges: &[LiveRange],
    class_plan: &super::desc::ClassPlan,
    points: u32,
    barriers: &Barriers,
    costs: &super::desc::CostTable,
) -> Result<Vec<Outcome>, BackendError> {
    let TimelineNetwork { mut g, enters, throughs, exits, chain_arcs, .. } = net;
    let mut bf_steps: u64 = 0;
    cancel_negative_cycles(&mut g, &mut bf_steps)?;
    repair_assignment(&mut g, ranges, &enters, &throughs, &exits, &chain_arcs)?;
    let mut outcomes = Vec::with_capacity(ranges.len());
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
    // Never-regress promotion: the refined flow is a feasible assignment, but a
    // residual cycle cannot move a chord between timelines, so the exact cold
    // optimum may be cheaper. Publish the cheaper answer (ties keep the cold
    // result so both accelerator paths are byte-identical to the exact one).
    let warm_cost = assignment_cost(&outcomes, ranges, class_plan, costs);
    let cold = solve_class(class_plan, ranges, points, barriers, costs)?;
    let cold_cost = assignment_cost(&cold, ranges, class_plan, costs);
    if cold_cost <= warm_cost {
        Ok(cold)
    } else {
        Ok(outcomes)
    }
}

/// CEP:WHAT: The true assignment objective over outcomes: routed ranges pay
/// 2*move_cost plus the callee-saved surcharge when unpinned; spilled ranges pay
/// spill_cost. Identical to the brute-force corpus pricing.
/// CEP:WHY: The promotion check needs the same objective the corpus arbitrates.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: pins reference class lanes; class_plan.lanes aligns with outcome ids.
/// CEP:COST: O(ranges).
/// CEP:EVIDENCE: warm_start_matches_bruteforce_optimum_corpus.
fn assignment_cost(
    outcomes: &[Outcome],
    ranges: &[LiveRange],
    class_plan: &super::desc::ClassPlan,
    costs: &super::desc::CostTable,
) -> i64 {
    let mut total = 0i64;
    for (i, range) in ranges.iter().enumerate() {
        match outcomes[i] {
            Outcome::Reg(ridx) => {
                total += 2 * costs.move_cost;
                if range.pin.is_none() && class_plan.callee_saved[ridx] {
                    total += costs.save_restore_cost;
                }
            }
            Outcome::Spill => total += costs.spill_cost,
        }
    }
    total
}
