// CEP:FILE: crates/rv8_backend/src/regalloc/mod.rs
// CEP:WHAT: The shared register allocator core (one reggolloc): public entry points
//   `allocate` (the shipped x86-64 target) and `allocate_with` (any target described
//   by an rv8_isl TargetDescription), the published cost constants, the frame plan,
//   and the orchestration that derives ranges, barriers, and per-class solves from
//   the target description.
// CEP:WHY: Every target shares exactly one allocator core; a target is an ISL
//   description, not a code path (STANDARDS law 7, user law: the ISL carries target
//   descriptions and each target shares exactly one reggolloc).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: BackendError from every stage; infeasibility is impossible (every
//   timeline can idle). Library code never panics.
// CEP:ASSUMES: LIR invariants 4.3.1-4.3.4; the plan's lane order defines RegId
//   mapping; at most six register arguments.
// CEP:COST: dominated by the per-class solves (see the submodule CEP:COST lines).
// CEP:EVIDENCE: both brute-force corpora, the determinism/pressure/precoloring tests,
//   the RV8-137 regression batch, and the desc cross-check tests.
pub mod desc;
pub mod network;
pub mod ranges;
pub mod rewrite;
pub mod solve;

use crate::error::BackendError;
use rv8_ir::lir::{LirFunction, RegClassId, RegisterAllocation};

pub use desc::{CostTable, Plan};
pub use rewrite::{rewrite, FramePlan};

/// CEP:WHAT: Infinite-capacity marker (u64::MAX) published as part of the allocator's
/// API contract.
/// CEP:WHY: STANDARDS 5.1's feasibility story is now structural in the timeline
/// network (an idling register timeline always carries flow), so no solver edge uses
/// this value any more; the constant stays for downstream callers and diagnostics so
/// the published surface does not churn.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: u64::MAX is never a real capacity requirement.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: mcf_matches_bruteforce_optimum_corpus finds a feasible flow without it.
pub const CAP_INFINITE: u64 = u64::MAX;

/// CEP:WHAT: Fixed-point scale for the f64 costs published in lir::RegisterClass
/// (1.0 cost unit == 100 network cost points).
/// CEP:WHY: Min-cost flow needs integers; scaling keeps the published table honest.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: class costs are small positive decimals.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: MOVE_COST / SPILL_COST derive from it.
pub const COST_SCALE: i64 = 100;

/// CEP:WHAT: Cost of one def-connection or use-connection edge (gpr.move_cost scaled):
/// a register-routed commodity pays 2 * MOVE_COST total.
/// CEP:WHY: STANDARDS 5.1: "def/use connections cost MOVE_COST".
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: gpr_class().move_cost == 1.0.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: brute-force corpus prices assignments with the identical table.
pub const MOVE_COST: i64 = COST_SCALE;

/// CEP:WHAT: Cost of routing a commodity through the spill edge (gpr.spill_cost scaled).
/// CEP:WHY: Dominates 2 * MOVE_COST + any save cost, so spilling happens only under real
/// pressure.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: gpr_class().spill_cost == 100.0.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: mcf_spills_under_pressure_and_verifies.
pub const SPILL_COST: i64 = 100 * COST_SCALE;

/// CEP:WHAT: Extra cost on the def connection into a callee-saved register (models the
/// save/restore pair in prologue/epilogue).
/// CEP:WHY: Callee-saved registers are available but not free; the objective must see it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: symmetric save/restore cost, charged once per range.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: pressure test routes through callee-saved regs under call barriers.
pub const SAVE_RESTORE_COST: i64 = 20 * COST_SCALE;

/// CEP:WHAT: Cost of the emergency spill edge available to pre-colored (pinned) ranges.
/// CEP:WHY: Pinned ranges have fixed register edges; without an escape, a pinned range
/// whose lane is blocked (its ABI register is an argument-move destination at a call it
/// crosses) could not route. The escape keeps feasibility absolute; the high cost makes
/// it a last resort. A spilled pinned value is still correct code (entry store from its
/// ABI register, loads at uses; implemented in rewrite).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: mcf_respects_abi_precoloring.
pub const PINNED_SPILL_COST: i64 = 8 * SPILL_COST;

/// CEP:WHAT: Maximum flow-network nodes per class solve.
/// CEP:WHY: Named budget (38.12).
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in solve_class.
pub const MAX_MCF_NODES: usize = 1 << 20;

/// CEP:WHAT: Maximum flow-network half-edge pairs per class solve.
/// CEP:WHY: Named budget (38.12).
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in solve_class.
pub const MAX_MCF_EDGES: usize = 1 << 22;

/// CEP:WHAT: Maximum negative-cycle cancellations in the optimality backstop.
/// CEP:WHY: Named budget (38.12) for the Bellman-Ford cleanse that guards the SSP
/// invariant (the invariant should make it a no-op; the budget bounds it regardless).
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (honest bail; never silent suboptimality).
/// CEP:ASSUMES: integer costs strictly decrease per cancellation.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: corpus and pressure solves need zero cancellations after SSP insertion.
pub const MAX_MCF_CYCLE_CANCELLATIONS: usize = 10_000;

/// CEP:WHAT: Maximum Bellman-Ford relaxation attempts across one class solve.
/// CEP:WHY: BF is O(N*E) per refresh; this bounds total compile time.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in bellman_ford.
pub const MAX_MCF_BF_STEPS: u64 = 2_000_000_000;

/// CEP:WHAT: The rewrite result: ABI-conformant MachineFrame plus every emission fact
/// (slot offsets, stack adjustment, saved-register order, red-zone decision).
/// CEP:WHY: Prologue/epilogue bytes, spill addressing, and frame metadata must come from
/// one calculation (5.3.3); this is that calculation's output.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: slot addressing is rsp-relative after the prologue.
/// CEP:COST: POD plus small vectors.
/// CEP:EVIDENCE: spill_path_inserts_loads_and_stores; encode ABI byte tests.
/// CEP:WHAT: Allocates registers for a whole function against a target description:
///   builds live ranges over the linearized program points, solves each register
///   class as a min-cost multi-commodity flow over the plan's lanes and costs, and
///   returns the RegisterAllocation (RegId per class mapping through the plan's lane
///   order; spilled values get dense slot ids in value-id order; dead value ids get
///   neither). This is the ONE allocator core every target shares.
/// CEP:WHY: STANDARDS 5.1 public entry point, parameterized by ISL target
///   descriptions (user law: the ISL carries target descriptions).
/// CEP:STATUS: complete
/// CEP:FAILURE: see build_ranges and solve_class; infeasibility is impossible.
/// CEP:ASSUMES: the plan's first class hosts the ABI roles; LIR invariants 4.3.1-4.3.4.
/// CEP:COST: dominated by the class solves (module CEP:COST).
/// CEP:EVIDENCE: both brute-force corpora, the RV8-137 batch, and the toy-target
///   pressure test that proves two descriptions share this one core.
pub fn allocate_with(
    plan: &Plan,
    lir: &LirFunction,
) -> Result<RegisterAllocation, BackendError> {
    let n = lir.values.len();
    let (ranges, points) = ranges::build_ranges(plan, lir)?;

    let mut outcomes: Vec<Outcome> = vec![Outcome::Spill; n];
    let mut has_range = vec![false; n];
    for r in &ranges {
        has_range[r.value.0 as usize] = true;
    }

    let class_ids = [RegClassId::Gpr, RegClassId::Fpr];
    for (class_idx, &class) in class_ids.iter().enumerate() {
        if ranges.iter().any(|r| r.class == RegClassId::Vector) {
            return Err(BackendError::UnsupportedPattern {
                detail: "vector register class allocation not implemented".into(),
            });
        }
        let class_plan = &plan.classes[class_idx];
        let class_ranges: Vec<LiveRange> =
            ranges.iter().copied().filter(|r| r.class == class).collect();
        if class_ranges.is_empty() {
            continue;
        }
        let barriers = ranges::build_barriers(plan, lir, points, class_idx);
        let solved =
            solve::solve_class(class_plan, &class_ranges, points, &barriers, &plan.costs)?;
        for (range, outcome) in class_ranges.iter().zip(solved) {
            outcomes[range.value.0 as usize] = outcome;
        }
    }

    let mut alloc = RegisterAllocation::new(n);
    let mut slot: u32 = 0;
    for v in 0..n {
        if !has_range[v] {
            continue; // dead id: no register, no slot
        }
        match outcomes[v] {
            Outcome::Reg(ridx) => alloc.assignments[v] = Some(RegId(ridx as u32)),
            Outcome::Spill => {
                alloc.spill_slots[v] = Some(slot);
                slot += 1;
            }
        }
    }
    Ok(alloc)
}

/// CEP:WHAT: Allocates registers with the shipped x86-64 target description.
/// CEP:WHY: The established public entry point; the plan is derived from the ISL
///   description shipped in rv8_isl.
/// CEP:STATUS: complete
/// CEP:FAILURE: see allocate_with.
/// CEP:ASSUMES: the shipped description parses (cross-checked by tests).
/// CEP:COST: one ISL parse per call (cold, documented); use allocate_with with a
///   cached Plan in hot drivers.
/// CEP:EVIDENCE: the full workspace test suite.
pub fn allocate(lir: &LirFunction) -> Result<RegisterAllocation, BackendError> {
    allocate_with(&desc::x86_64_plan(), lir)
}

use rv8_ir::lir::RegId;
use ranges::LiveRange;
use solve::Outcome;
