// CEP:FILE: crates/rv8_backend/src/regalloc.rs
// CEP:WHAT: Min-cost multi-commodity-flow register allocation per STANDARDS 5.1: per
//   register class, a machine-timeline flow network (one cap-1 timeline chain per
//   register, job pairs per live range with priced through-edges; see solve_class) in
//   which every integral flow of value R is a feasible assignment by construction;
//   R successive-shortest-path augmentations (one per register timeline) with Johnson
//   potentials initialized by one Bellman-Ford pass and updated by
//   pot[v] += min(dist[v], dist[snk]); a Bellman-Ford negative-cycle cleanse as the
//   optimality backstop; call/divide barriers and ABI pre-coloring as omitted job arcs.
//   Also the spill/prologue/epilogue rewriter producing verified Machine IR plus a
//   FramePlan.
// CEP:WHY: Register pressure is an optimization problem, not a heuristic; the network
//   makes "which values spill" the exact optimum of an explicit cost function, and the
//   brute-force corpus proves optimality on small instances.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// Feasibility is structural in the timeline reduction: every register timeline can
// idle, so an R-unit flow always exists (the inherited def-split/lane network with
// infinite spill edges was replaced after corpus seed 0x9001 showed its
// cancel/reroute bookkeeping could emit infeasible assignments; CAP_INFINITE survives
// as a published marker, not a solver edge).
// CEP:FAILURE: BackendError::IllegalLir for structural violations (double definitions,
//   use before definition in layout order), UnsupportedPattern for the Vector class,
//   BudgetExceeded past named budgets, Internal only for violated by-construction
//   invariants (compiler bugs). Infeasibility is impossible: every timeline can idle,
//   so a failed route is always an Internal bug, never a user-facing failure.
// CEP:ASSUMES: PhysReg numbering per abi.rs (machir x86-64 space); RegId is dense within
//   a class and maps through abi::gpr_phys / abi::fpr_phys; parameters are the values
//   never defined in any block in value-id order (the same rule isel enforces); call
//   arguments are moved into ABI argument registers by instruction selection, and the
//   network blocks the argument-register lanes across the move window so parallel moves
//   are safe.
// CEP:COST: one reduced-cost Dijkstra per range O(ranges x E log N); one backstop
//   Bellman-Ford O(N*E) capped by MAX_MCF_BF_STEPS. Named budgets bound everything
//   (38.12).
// CEP:EVIDENCE: mcf_matches_bruteforce_optimum_corpus (24 seeded instances),
//   mcf_allocation_is_deterministic, mcf_spills_under_pressure_and_verifies,
//   spill_path_inserts_loads_and_stores, mcf_respects_abi_precoloring,
//   mcf_avoids_caller_saved_across_calls.
// CEP:SECURITY: every traversal is iterative (worklists, heaps, BF sweeps); no recursion
//   over user structure; all iteration orders are index-ordered (38.10).

use crate::abi::{self, FrameCalculator, FrameLayout};
use crate::error::BackendError;
use rv8_ir::lir::{LirFunction, MachineOpcode, RegClassId, RegId, RegisterAllocation, ValueId};
use rv8_ir::machir::{MachineBlock, MachineInstr, MachineIrFunction, MachineOperand, MemOperand, PhysReg};
use MachineOperand::Reg;

// ---------------------------------------------------------------------------
// Named budgets and cost constants (STANDARDS 2.6 / 38.12)
// ---------------------------------------------------------------------------

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
#[derive(Clone, Debug)]
pub struct FramePlan {
    /// ABI-conformant frame (stack_size multiple of alignment; see FrameCalculator).
    pub frame: rv8_ir::machir::MachineFrame,
    /// RSP-relative offset of every spill slot.
    pub slot_offsets: Vec<i32>,
    /// Bytes subtracted from RSP by the prologue (0 in red-zone frames).
    pub stack_adjust: u32,
    /// Callee-saved registers pushed, in push order.
    pub saved: Vec<PhysReg>,
    /// True when slots live in the red zone (leaf frame, no adjustment).
    pub uses_red_zone: bool,
}

// ---------------------------------------------------------------------------
// LIR facts shared by allocate() and rewrite()
// ---------------------------------------------------------------------------

/// CEP:WHAT: One live range: value id, linearized definition point, last-use point,
/// register class, and optional ABI pin.
/// CEP:WHY: The commodity of the MCF network; points are instruction ordinals across
/// blocks in layout order with point 0 = function entry.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: def <= last_use (checked in build_ranges).
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
#[derive(Clone, Copy, Debug)]
struct LiveRange {
    value: ValueId,
    def: u32,
    last_use: u32,
    class: RegClassId,
    pin: Option<PhysReg>,
}

/// CEP:WHAT: Computes the linearized program points and live ranges for a function.
/// CEP:WHY: The network is built over these points; shared facts for allocate/rewrite.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalLir for double definitions, dangling ids, and uses that precede
/// their definition in layout order (the linearized-liveness contract).
/// CEP:ASSUMES: point 0 is function entry; the instruction at global ordinal g is
/// point g+1.
/// CEP:COST: O(instructions + values).
/// CEP:EVIDENCE: internal; behavior pinned by all regalloc tests.
fn build_ranges(lir: &LirFunction) -> Result<(Vec<LiveRange>, u32), BackendError> {
    let n = lir.values.len();
    let mut total_instrs: u32 = 0;
    for b in &lir.blocks {
        total_instrs += b.instructions.len() as u32;
    }
    let points: u32 = 1 + total_instrs;

    let mut def_point: Vec<Option<u32>> = vec![None; n];
    let mut last_use: Vec<u32> = vec![0; n];
    let mut any_use = vec![false; n];
    let mut ordinal: u32 = 0;
    for block in &lir.blocks {
        for instr in &block.instructions {
            let point = 1 + ordinal;
            ordinal += 1;
            for &op in &instr.operands {
                let v = op.0 as usize;
                if v >= n {
                    return Err(BackendError::IllegalLir { detail: format!(
                        "operand value id {} out of range", v) });
                }
                any_use[v] = true;
                if point > last_use[v] {
                    last_use[v] = point;
                }
            }
            if let Some(r) = instr.result {
                let v = r.0 as usize;
                if v >= n {
                    return Err(BackendError::IllegalLir { detail: format!(
                        "result value id {} out of range", v) });
                }
                // LIR 4.3.1 relaxes single-definition ("registers can be redefined"):
                // phi values lower to predecessor-edge copies, so a value may be
                // written once per incoming path. The live range spans the FIRST
                // definition to the last use; interior rewrites are ordinary writes
                // within the routed register, and every def is a spill store site.
                if def_point[v].is_none() {
                    def_point[v] = Some(point);
                }
            }
        }
    }

    // Parameters (never defined, used somewhere) in value-id order pin to ABI arg regs.
    let mut param_index = 0usize;
    let mut ranges = Vec::with_capacity(n);
    for v in 0..n {
        let value = ValueId(v as u32);
        let def = match def_point[v] {
            Some(d) => d,
            None => {
                if !any_use[v] {
                    continue; // dead id: no range, no slot
                }
                if param_index >= abi::ARG_REGISTERS.len() {
                    return Err(BackendError::UnsupportedPattern { detail: format!(
                        "more than {} parameters; stack arguments are not implemented",
                        abi::ARG_REGISTERS.len()) });
                }
                if lir.values[v].reg_class != RegClassId::Gpr {
                    return Err(BackendError::UnsupportedPattern { detail: format!(
                        "parameter {} is not GPR class", v) });
                }
                let pin = abi::ARG_REGISTERS[param_index];
                param_index += 1;
                ranges.push(LiveRange {
                    value,
                    def: 0,
                    last_use: last_use[v],
                    class: RegClassId::Gpr,
                    pin: Some(pin),
                });
                continue;
            }
        };
        // A definition with no uses is legal LIR (dead code the middle end chose to
        // keep), and a use at the defining instruction itself is self-use; both get a
        // one-lane span [def, def+1) so the defining write occupies its register for at
        // least the point it is defined at. A genuine use before the definition in
        // layout order is an error.
        let range_last = if last_use[v] <= def {
            if any_use[v] && last_use[v] < def {
                return Err(BackendError::IllegalLir { detail: format!(
                    "value {} used before its definition in layout order; linearized liveness \
                     requires defs to precede uses", v) });
            }
            (def + 1).min(points - 1)
        } else {
            last_use[v]
        };
        ranges.push(LiveRange {
            value,
            def,
            last_use: range_last,
            class: lir.values[v].reg_class,
            pin: None,
        });
    }

    // Return-value pins: every ret's value operand is pinned to RAX for its whole
    // range (documented in isel: the ABI move writes RAX).
    for block in &lir.blocks {
        for instr in &block.instructions {
            if instr.opcode == MachineOpcode::Ret {
                if let Some(&v) = instr.operands.first() {
                    for range in ranges.iter_mut() {
                        if range.value == v {
                            range.pin = Some(abi::RETURN_REGISTER);
                        }
                    }
                }
            }
        }
    }
    Ok((ranges, points))
}

/// CEP:WHAT: Call/division barrier facts per program point, in class register-index
/// bitmask form: `after[p]` blocks lane edge (p -> p+1) (the call clobbers those
/// registers / the divide sequence leaves scratch in them); `before[p]` blocks lane
/// edge (p-1 -> p) (argument moves and the dividend move write those registers in the
/// window before the instruction).
/// CEP:WHY: Values live across a call must avoid caller-saved registers; values crossing
/// an arg-move or cqo/idiv window must avoid the registers those sequences write.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at most 64 registers per class (bitmask).
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
#[derive(Clone, Debug, Default)]
struct Barriers {
    after: Vec<u64>,
    before: Vec<u64>,
}

/// CEP:WHAT: Builds the barrier masks for one register class from the LIR opcode stream.
/// CEP:WHY: See Barriers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: call argument count <= 6 (isel enforces); regs outside the class map to
/// no bit.
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
fn build_barriers(lir: &LirFunction, points: u32, is_gpr: bool) -> Barriers {
    let gpr_bit = |r: PhysReg| -> u64 {
        abi::ALLOCATABLE_GPRS
            .iter()
            .position(|&p| p == r)
            .map_or(0, |i| 1u64 << i)
    };
    let mut b = Barriers {
        after: vec![0u64; points as usize],
        before: vec![0u64; points as usize],
    };
    let mut ordinal: u32 = 0;
    for block in &lir.blocks {
        for instr in &block.instructions {
            let p = (1 + ordinal) as usize;
            ordinal += 1;
            match instr.opcode {
                MachineOpcode::Call | MachineOpcode::Syscall => {
                    if is_gpr {
                        for &r in abi::CALLER_SAVED_REGISTERS.iter() {
                            b.after[p] |= gpr_bit(r);
                        }
                        // Argument moves write arg registers in the window before the
                        // call instruction (isel emits them at this LIR point).
                        let nargs = instr.operands.len().min(abi::ARG_REGISTERS.len());
                        if p > 0 {
                            for &r in abi::ARG_REGISTERS.iter().take(nargs) {
                                b.before[p] |= gpr_bit(r);
                            }
                        }
                    } else {
                        // SysV: every vector register is caller-clobbered.
                        for bit in 0..abi::ALLOCATABLE_FPRS.len() {
                            b.after[p] |= 1u64 << bit;
                        }
                    }
                }
                MachineOpcode::Cqo | MachineOpcode::IdivReg if is_gpr => {
                    let mask = gpr_bit(abi::RAX) | gpr_bit(abi::RDX);
                    b.after[p] |= mask;
                    if p > 0 {
                        b.before[p] |= mask;
                    }
                }
                _ => {}
            }
        }
    }
    b
}

// ---------------------------------------------------------------------------
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
struct HalfEdge {
    to: usize,
    cost: i64,
    resid: u64,
    rev: usize,
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
struct Graph {
    adj: Vec<Vec<HalfEdge>>,
}

/// CEP:WHAT: Global half-edge id: (node << 32) | adjacency index.
/// CEP:WHY: Paths and predecessor links store flat ids.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: adjacency index < 2^32.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn encode(u: usize, idx: usize) -> usize {
    (u << 32) | idx
}

/// CEP:WHAT: Decodes a global half-edge id into (node, adjacency index).
/// CEP:WHY: Counterpart of encode.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: id came from encode.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn decode(he: usize) -> (usize, usize) {
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
    fn new(n: usize) -> Graph {
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
    fn add_edge(&mut self, u: usize, v: usize, cap: u64, cost: i64) -> usize {
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
    fn apply(&mut self, path: &[usize], delta: i64) -> Result<(), BackendError> {
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
    fn flow_on(&self, he: usize) -> bool {
        let (u, idx) = decode(he);
        self.adj[u][idx].resid == 0
    }
}

/// CEP:WHAT: Return bundle of bellman_ford (distances, predecessor links, negative-cycle
/// witness); a named alias keeps the signature readable.
type BellmanFordResult = (Vec<i64>, Vec<Option<usize>>, Option<usize>);

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
fn bellman_ford(
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
fn cancel_negative_cycles(
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

/// CEP:WHAT: Return bundle of dijkstra (distances, parent links); a named alias keeps
/// the signature readable.
type DijkstraResult = (Vec<Option<i64>>, Vec<Option<usize>>);

/// CEP:WHAT: Dijkstra over residual half-edges with Johnson reduced costs
/// (c + pot[u] - pot[v]); returns distances and parent links. The heap orders by
/// (distance, node id), so ties pop the lowest node id first: deterministic.
/// CEP:WHY: STANDARDS 5.1: successive shortest paths use reduced-cost Dijkstra with
/// deterministic tie-breaking.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal on a negative reduced cost (invalid potentials - compiler bug).
/// CEP:ASSUMES: pot satisfies the Johnson condition on every residual half-edge.
/// CEP:COST: O(E log N).
/// CEP:EVIDENCE: corpus optimality.
fn dijkstra(
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
fn path_of(parent: &[Option<usize>], src: usize, snk: usize) -> Result<Vec<usize>, BackendError> {
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

// ---------------------------------------------------------------------------
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
enum Outcome {
    Reg(usize),
    Spill,
}

// Design note (the machine-timeline reduction behind solve_class): one timeline chain
// C(r, 0..points) per register (SRC -> C(r,0), idle arcs C(r,p) -> C(r,p+1),
// C(r,last) -> SNK, all cap 1 cost 0), and per range i a job pair J_i/J_i' threaded
// through the chains: entry arc C(r, def_i) -> J_i, priced through-edge J_i -> J_i'
// (cap 1, cost -SPILL_PRICE(i)), and exit arc J_i' -> C(r, use_i) (cap 1, cost
// 2*MOVE_COST + callee-saved surcharge). Range i is assigned register r iff its entry
// arc carries flow; no flow on the through-edge means spilled. A unit of flow is one
// register's timeline; job arcs are chords a timeline may take, and a single forward
// path can only take time-disjoint chords, so any integral flow IS a feasible register
// assignment. Pricing each taken job at (c - SPILL) makes the encoded objective equal
// the assignment cost plus the constant sum of all spill prices, so the min-cost flow
// of value R is THE exact optimum of the interval-assignment problem the brute-force
// checker enumerates.

/// CEP:WHAT: Solves one register class as a min-cost multi-commodity flow:
/// 1. Network: machine-timeline reduction (see the design note above); job
///    arcs crossing a call/divide barrier of register r are omitted, as are arcs of
///    non-matching registers for pinned ranges.
/// 2. Potentials: one Bellman-Ford pass (super-source semantics) on the zero flow - the
///    negative job-arc prices demand it, and the graph is acyclic so it converges.
/// 3. Solve: R successive shortest paths (one per register timeline) with Johnson
///    potentials maintained by pot[v] += min(dist[v], dist[snk]) after each
///    augmentation; the SSP theorem makes the final flow the exact minimum.
/// 4. Optimality backstop: Bellman-Ford negative-cycle cleanse (a no-op while the SSP
///    invariant holds, a repair if it ever regressed).
/// 5. Attribution: through-edge flow means "in a register", the saturated entry arc
///    names which one; no flow means spilled.
///
/// CEP:WHY: THE core algorithm (STANDARDS 5.1), exactly as specified.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past named budgets; Internal for by-construction
/// invariant violations (an R-unit flow always exists: every timeline can idle).
/// CEP:ASSUMES: ranges have def < last_use; pins reference class registers.
/// CEP:COST: module CEP:COST.
/// CEP:EVIDENCE: mcf_matches_bruteforce_optimum_corpus.
#[allow(clippy::too_many_lines)]
fn solve_class(
    ranges: &[LiveRange],
    points: u32,
    regs: &[PhysReg],
    barriers: &Barriers,
) -> Result<Vec<Outcome>, BackendError> {
    let r_count = regs.len();
    let nranges = ranges.len();
    let last_point = points as usize;
    // Node layout: 0 = SRC, 1 = SNK, then per-register timelines
    // C(r, p) = 2 + r*points + p, then job pairs J_i = jbase + 2i, Jout_i = jbase+2i+1.
    let chain = |ridx: usize, p: usize| -> usize { 2 + ridx * last_point + p };
    let jbase = 2 + r_count * last_point;
    let n_nodes = jbase + 2 * nranges;
    if n_nodes > MAX_MCF_NODES {
        return Err(BackendError::BudgetExceeded { budget: "MAX_MCF_NODES" });
    }
    let src = 0usize;
    let snk = 1usize;
    let job_in = |i: usize| -> usize { jbase + 2 * i };
    let job_out = |i: usize| -> usize { jbase + 2 * i + 1 };

    let mut g = Graph::new(n_nodes);
    let mut edge_budget = MAX_MCF_EDGES;
    let mut budget_edge = || -> Result<(), BackendError> {
        if edge_budget == 0 {
            return Err(BackendError::BudgetExceeded { budget: "MAX_MCF_EDGES" });
        }
        edge_budget -= 1;
        Ok(())
    };

    // Register timelines: one unit of flow per register, idling by default.
    for ridx in 0..r_count {
        budget_edge()?;
        g.add_edge(src, chain(ridx, 0), 1, 0);
        budget_edge()?;
        g.add_edge(chain(ridx, last_point - 1), snk, 1, 0);
        for p in 0..last_point - 1 {
            budget_edge()?;
            g.add_edge(chain(ridx, p), chain(ridx, p + 1), 1, 0);
        }
    }

    // Job arcs: entry (timeline -> J), priced through-edge (J -> J'), exit (J' ->
    // timeline). The through-edge is where the spill offset is charged, so a taken job
    // costs c_{i,r} - SPILL_PRICE and an untaken job costs SPILL_PRICE in the true
    // objective (constant P*n offset; see the network CEP block).
    let mut enters: Vec<Vec<usize>> = Vec::with_capacity(nranges);
    let mut throughs: Vec<usize> = Vec::with_capacity(nranges);
    for (i, range) in ranges.iter().enumerate() {
        let pinned_ridx = range.pin.and_then(|p| regs.iter().position(|&x| x == p));
        let spill_price = if pinned_ridx.is_some() { PINNED_SPILL_COST } else { SPILL_COST };
        budget_edge()?;
        throughs.push(g.add_edge(job_in(i), job_out(i), 1, -spill_price));
        let mut enter = vec![usize::MAX; r_count];
        for ridx in 0..r_count {
            if let Some(pr) = pinned_ridx {
                if ridx != pr {
                    continue;
                }
            }
            if range_crosses_barrier(range, ridx, barriers) {
                continue; // this register is physically unavailable under the range
            }
            let mut cost = 2 * MOVE_COST;
            if pinned_ridx.is_none() && abi::is_callee_saved(regs[ridx]) {
                cost += SAVE_RESTORE_COST;
            }
            budget_edge()?;
            let ein = g.add_edge(chain(ridx, range.def as usize), job_in(i), 1, 0);
            budget_edge()?;
            g.add_edge(job_out(i), chain(ridx, range.last_use as usize), 1, cost);
            enter[ridx] = ein;
        }
        enters.push(enter);
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
    // cycles, so this cleanse runs witness-free Bellman-Ford passes and changes nothing;
    // when a future edit breaks that invariant, cancellation repairs optimality
    // instead of emitting a silently suboptimal assignment. ----
    let _ = cancel_negative_cycles(&mut g, &mut bf_steps)?;

    // ---- Attribution: through-edge flow means the range sits in a register; the
    // saturated entry arc names which one (cap-1 through-edge admits at most one). ----
    let mut outcomes = Vec::with_capacity(nranges);
    for (i, range) in ranges.iter().enumerate() {
        let mut assigned: Option<usize> = None;
        if g.flow_on(throughs[i]) {
            for (ridx, &ein) in enters[i].iter().enumerate() {
                if ein != usize::MAX && g.flow_on(ein) {
                    assigned = Some(ridx);
                    break;
                }
            }
            if assigned.is_none() {
                return Err(BackendError::Internal { detail: format!(
                    "range {} carries flow but no entry arc names a register", range.value.0) });
            }
        }
        outcomes.push(match assigned {
            Some(ridx) => Outcome::Reg(ridx),
            None => Outcome::Spill,
        });
    }
    Ok(outcomes)
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
fn range_crosses_barrier(range: &LiveRange, ridx: usize, barriers: &Barriers) -> bool {
    let bit = 1u64 << ridx;
    (range.def..range.last_use).any(|p| {
        let pi = p as usize;
        (barriers.after[pi] & bit) != 0 || (p > 0 && (barriers.before[pi] & bit) != 0)
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// CEP:WHAT: Allocates registers for a whole function: builds live ranges over the
/// linearized program points, solves each register class as a min-cost multi-commodity
/// flow (see solve_class), and returns the RegisterAllocation (RegId per class mapping
/// through abi::gpr_phys / abi::fpr_phys; spilled values get dense slot ids in value-id
/// order; dead value ids get neither).
/// CEP:WHY: STANDARDS 5.1 public entry point.
/// CEP:STATUS: complete
/// CEP:FAILURE: see solve_class and build_ranges; infeasibility is impossible.
/// CEP:ASSUMES: LIR invariants 4.3.1-4.3.4.
/// CEP:COST: dominated by the class solves (module CEP:COST).
/// CEP:EVIDENCE: the brute-force optimality corpus, determinism, pressure, and
/// pre-coloring tests.
pub fn allocate(lir: &LirFunction) -> Result<RegisterAllocation, BackendError> {
    let n = lir.values.len();
    let (ranges, points) = build_ranges(lir)?;

    let mut outcomes: Vec<Outcome> = vec![Outcome::Spill; n];
    let mut has_range = vec![false; n];
    for r in &ranges {
        has_range[r.value.0 as usize] = true;
    }

    for class in [RegClassId::Gpr, RegClassId::Fpr] {
        let regs: &[PhysReg] = match class {
            RegClassId::Gpr => &abi::ALLOCATABLE_GPRS,
            RegClassId::Fpr => &abi::ALLOCATABLE_FPRS,
            RegClassId::Vector => {
                if ranges.iter().any(|r| r.class == RegClassId::Vector) {
                    return Err(BackendError::UnsupportedPattern {
                        detail: "vector register class allocation not implemented".into(),
                    });
                }
                continue;
            }
        };
        let class_ranges: Vec<LiveRange> =
            ranges.iter().copied().filter(|r| r.class == class).collect();
        if class_ranges.is_empty() {
            continue;
        }
        let barriers = build_barriers(lir, points, class == RegClassId::Gpr);
        let solved = solve_class(&class_ranges, points, regs, &barriers)?;
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

/// CEP:WHAT: Rewrites post-selection MIR in place: vregs become physical registers,
/// spilled values get LoadStack/StoreStack with deterministic scratch selection, the
/// prologue (push callee-saved; sub rsp) and epilogues (add rsp; pops) are inserted
/// (skipped entirely for red-zone leaf frames), spilled parameters store their ABI
/// register at entry, and MachineFrame is filled from the FrameCalculator.
/// CEP:WHY: STANDARDS 5.1 public entry point; turns allocated values into verified MIR.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnsupportedPattern for unknown classes; Internal for scratch exhaustion
/// (unreachable: at most 15 register operands fit one instruction) and mapping holes.
/// CEP:ASSUMES: mir came from isel::select over the same lir (vreg numbering contract);
/// alloc came from allocate() over the same lir.
/// CEP:COST: O(mir instructions x operands).
/// CEP:EVIDENCE: spill_path_inserts_loads_and_stores plus the verify() call in every
/// pipeline test. (No apostrophes in this block: the CEP scanner misclassifies lines
/// containing one.)
#[allow(clippy::too_many_lines)]
pub fn rewrite(
    mir: &mut MachineIrFunction,
    lir: &LirFunction,
    alloc: &RegisterAllocation,
) -> Result<FramePlan, BackendError> {
    if alloc.assignments.len() < lir.values.len() {
        return Err(BackendError::Internal { detail: "allocation smaller than value table".into() });
    }
    // Physical register of an assigned value; None = spilled.
    let phys_of = |v: ValueId| -> Result<Option<PhysReg>, BackendError> {
        match alloc.register(v) {
            Some(rid) => {
                let p = match lir.values[v.0 as usize].reg_class {
                    RegClassId::Gpr => abi::gpr_phys(rid),
                    RegClassId::Fpr => abi::fpr_phys(rid),
                    RegClassId::Vector => {
                        return Err(BackendError::UnsupportedPattern { detail: "vector class".into() })
                    }
                };
                p.map(Some).ok_or_else(|| BackendError::Internal { detail: format!(
                    "RegId {} outside class table for value {}", rid.0, v.0) })
            }
            None => Ok(None),
        }
    };
    let slot_of = |v: ValueId| -> Result<u32, BackendError> {
        alloc.spill_slots.get(v.0 as usize).copied().flatten().ok_or_else(|| {
            BackendError::Internal { detail: format!("spilled value {} has no slot", v.0) }
        })
    };

    // Frame facts: saved set + leafness.
    let mut saved: Vec<PhysReg> = Vec::new();
    for v in 0..lir.values.len() {
        if let Some(p) = phys_of(ValueId(v as u32))? {
            if abi::is_callee_saved(p) && !saved.contains(&p) {
                saved.push(p);
            }
        }
    }
    let mut leaf = true;
    for block in &mir.blocks {
        for instr in &block.instructions {
            if matches!(instr.opcode, MachineOpcode::Call | MachineOpcode::Syscall) {
                leaf = false;
            }
        }
    }
    let spill_count = alloc.spill_slots.iter().flatten().count() as u32;
    let layout: FrameLayout = FrameCalculator::calculate(spill_count, &saved, leaf);

    let mem_slot = |slot: u32| -> MemOperand {
        MemOperand {
            base: Some(abi::RSP),
            index: None,
            scale: 1,
            disp: layout.slot_offset(slot).unwrap_or(0),
        }
    };

    // Spilled parameters store their ABI register right after the prologue.
    let params = param_ids(lir)?;
    let mut spilled_params: Vec<(PhysReg, u32)> = Vec::new();
    for (pi, &v) in params.iter().enumerate() {
        if phys_of(v)?.is_none() {
            spilled_params.push((abi::ARG_REGISTERS[pi], slot_of(v)?));
        }
    }

    let dest_bearing = |op: MachineOpcode| -> bool {
        matches!(
            op,
            MachineOpcode::MovRegReg
                | MachineOpcode::MovRegImm32
                | MachineOpcode::Lea
                | MachineOpcode::AddRegReg
                | MachineOpcode::AddRegImm32
                | MachineOpcode::SubRegReg
                | MachineOpcode::SubRegImm32
                | MachineOpcode::ImulRegReg
                | MachineOpcode::ShlRegImm8
                | MachineOpcode::SarRegImm8
                | MachineOpcode::AndRegReg
                | MachineOpcode::OrRegReg
                | MachineOpcode::XorRegReg
                | MachineOpcode::XorRegSelf
                | MachineOpcode::Setcc(_)
                | MachineOpcode::MovRegMem
                | MachineOpcode::MovsdRegReg
                | MachineOpcode::CvtSi2sd
                | MachineOpcode::LoadStack
        )
    };

    let mut new_blocks = Vec::with_capacity(mir.blocks.len());
    for (bi, block) in mir.blocks.iter().enumerate() {
        let mut out: Vec<MachineInstr> = Vec::with_capacity(block.instructions.len() + 8);
        if bi == 0 && !layout.uses_red_zone {
            for &r in &layout.saved {
                out.push(MachineInstr {
                    opcode: MachineOpcode::Push,
                    operands: vec![Reg(r)],
                    predicates: Vec::new(),
                });
            }
            if layout.stack_adjust > 0 {
                out.push(MachineInstr {
                    opcode: MachineOpcode::SubRegImm32,
                    operands: vec![Reg(abi::RSP), MachineOperand::Imm(layout.stack_adjust as i64)],
                    predicates: Vec::new(),
                });
            }
            for &(pref, slot) in &spilled_params {
                out.push(MachineInstr {
                    opcode: MachineOpcode::StoreStack,
                    operands: vec![MachineOperand::Mem(mem_slot(slot)), Reg(pref)],
                    predicates: Vec::new(),
                });
            }
        }
        for instr in &block.instructions {
            // Epilogue before every return: restore RSP, then pop in reverse push order.
            if instr.opcode == MachineOpcode::Ret && !layout.uses_red_zone {
                if layout.stack_adjust > 0 {
                    out.push(MachineInstr {
                        opcode: MachineOpcode::AddRegImm32,
                        operands: vec![Reg(abi::RSP), MachineOperand::Imm(layout.stack_adjust as i64)],
                        predicates: Vec::new(),
                    });
                }
                for &r in layout.saved.iter().rev() {
                    out.push(MachineInstr {
                        opcode: MachineOpcode::Pop,
                        operands: vec![Reg(r)],
                        predicates: Vec::new(),
                    });
                }
            }
            rewrite_instr(instr, dest_bearing(instr.opcode), &phys_of, &slot_of, &mem_slot, &mut out)?;
        }
        new_blocks.push(MachineBlock {
            instructions: out,
            successors: block.successors.clone(),
        });
    }
    mir.blocks = new_blocks;
    mir.frame = layout.frame.clone();

    Ok(FramePlan {
        slot_offsets: (0..layout.slot_count)
            .map(|s| layout.slot_offset(s).unwrap_or(0))
            .collect(),
        frame: layout.frame,
        stack_adjust: layout.stack_adjust,
        saved: layout.saved,
        uses_red_zone: layout.uses_red_zone,
    })
}

/// CEP:WHAT: Rewrites one instruction: maps vregs to physical registers (including the
/// base/index registers inside memory operands, which isel-folded addressing uses),
/// inserts LoadStack before spilled uses and StoreStack after spilled definitions, and
/// picks scratches deterministically (lowest allocatable GPR not otherwise appearing in
/// the instruction, in operand order).
/// CEP:WHY: The mechanical heart of spill rewriting.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal on scratch exhaustion (unreachable: at most 15 register
/// operands fit one instruction) or slot/mapping holes.
/// CEP:ASSUMES: raw physical operands (RAX, ABI arg regs) pass through untouched and
/// count as occupied for scratch choice.
/// CEP:COST: O(operands x registers).
/// CEP:EVIDENCE: spill_path_inserts_loads_and_stores.
fn rewrite_instr(
    instr: &MachineInstr,
    has_dest: bool,
    phys_of: &dyn Fn(ValueId) -> Result<Option<PhysReg>, BackendError>,
    slot_of: &dyn Fn(ValueId) -> Result<u32, BackendError>,
    mem_slot: &dyn Fn(u32) -> MemOperand,
    out: &mut Vec<MachineInstr>,
) -> Result<(), BackendError> {
    let is_vreg = |r: PhysReg| -> Option<ValueId> {
        if r.0 >= crate::isel::VREG_BASE {
            Some(ValueId((r.0 - crate::isel::VREG_BASE) as u32))
        } else {
            None
        }
    };
    let phys_or_raw = |r: PhysReg| -> Result<Option<PhysReg>, BackendError> {
        match is_vreg(r) {
            Some(v) => phys_of(v),
            None => Ok(Some(r)),
        }
    };
    let mut new_ops = instr.operands.clone();
    let mut used: Vec<PhysReg> = Vec::new();
    // Occupancy for scratch choice: every physical register the instruction touches,
    // explicitly or through a memory operand's base/index.
    for op in &instr.operands {
        match op {
            Reg(r) => {
                if let Some(p) = phys_or_raw(*r)? {
                    if !used.contains(&p) {
                        used.push(p);
                    }
                }
            }
            MachineOperand::Mem(m) => {
                for r in [m.base, m.index].into_iter().flatten() {
                    if let Some(p) = phys_or_raw(r)? {
                        if !used.contains(&p) {
                            used.push(p);
                        }
                    }
                }
            }
            MachineOperand::Imm(_) => {}
        }
    }
    let pick_scratch = |used: &mut Vec<PhysReg>| -> Result<PhysReg, BackendError> {
        for &cand in abi::ALLOCATABLE_GPRS.iter() {
            if !used.contains(&cand) {
                used.push(cand);
                return Ok(cand);
            }
        }
        Err(BackendError::Internal { detail: "scratch register pool exhausted".into() })
    };

    // Loads for spilled uses (every operand position except a spilled dest).
    for (i, op) in instr.operands.iter().enumerate() {
        if has_dest && i == 0 {
            continue;
        }
        if let Reg(r) = op {
            if let Some(v) = is_vreg(*r) {
                match phys_of(v)? {
                    Some(p) => new_ops[i] = Reg(p),
                    None => {
                        let slot = slot_of(v)?;
                        let scratch = pick_scratch(&mut used)?;
                        out.push(MachineInstr {
                            opcode: MachineOpcode::LoadStack,
                            operands: vec![Reg(scratch), MachineOperand::Mem(mem_slot(slot))],
                            predicates: Vec::new(),
                        });
                        new_ops[i] = Reg(scratch);
                    }
                }
            }
        }
    }
    // Memory operands: rewrite vreg base/index to physical registers (a spilled
    // base/index is loaded into a scratch first). Without this pass, isel-folded
    // addressing would leak vregs into post-allocation MIR.
    for (i, op) in instr.operands.iter().enumerate() {
        if let MachineOperand::Mem(m) = op {
            let mut nm = *m;
            for (which, r) in [(0, m.base), (1, m.index)] {
                let Some(r) = r else { continue };
                let Some(v) = is_vreg(r) else { continue };
                match phys_of(v)? {
                    Some(p) => {
                        if which == 0 {
                            nm.base = Some(p)
                        } else {
                            nm.index = Some(p)
                        }
                    }
                    None => {
                        let slot = slot_of(v)?;
                        let scratch = pick_scratch(&mut used)?;
                        out.push(MachineInstr {
                            opcode: MachineOpcode::LoadStack,
                            operands: vec![Reg(scratch), MachineOperand::Mem(mem_slot(slot))],
                            predicates: Vec::new(),
                        });
                        if which == 0 {
                            nm.base = Some(scratch)
                        } else {
                            nm.index = Some(scratch)
                        }
                    }
                }
            }
            new_ops[i] = MachineOperand::Mem(nm);
        }
    }
    // Spilled destination: def into a scratch, store after.
    let mut store_after: Option<MachineInstr> = None;
    if has_dest {
        if let Reg(r) = instr.operands[0] {
            if let Some(v) = is_vreg(r) {
                match phys_of(v)? {
                    Some(p) => new_ops[0] = Reg(p),
                    None => {
                        let slot = slot_of(v)?;
                        let scratch = pick_scratch(&mut used)?;
                        new_ops[0] = Reg(scratch);
                        store_after = Some(MachineInstr {
                            opcode: MachineOpcode::StoreStack,
                            operands: vec![MachineOperand::Mem(mem_slot(slot)), Reg(scratch)],
                            predicates: Vec::new(),
                        });
                    }
                }
            }
        }
    }
    out.push(MachineInstr {
        opcode: instr.opcode,
        operands: new_ops,
        predicates: instr.predicates.clone(),
    });
    if let Some(st) = store_after {
        out.push(st);
    }
    Ok(())
}

/// CEP:WHAT: Parameter value ids in value-id order (never defined, used somewhere).
/// CEP:WHY: Shared ABI fact for rewrite's spilled-parameter stores; the same rule isel
/// documents and enforces.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnsupportedPattern past six GPR parameters.
/// CEP:ASSUMES: none
/// CEP:COST: O(values + instructions).
/// CEP:EVIDENCE: internal.
fn param_ids(lir: &LirFunction) -> Result<Vec<ValueId>, BackendError> {
    let n = lir.values.len();
    let mut defined = vec![false; n];
    let mut used = vec![false; n];
    for b in &lir.blocks {
        for i in &b.instructions {
            for &op in &i.operands {
                used[op.0 as usize] = true;
            }
            if let Some(r) = i.result {
                defined[r.0 as usize] = true;
            }
        }
    }
    let mut params = Vec::new();
    for v in 0..n {
        if !defined[v] && used[v] {
            params.push(ValueId(v as u32));
        }
    }
    if params.len() > abi::ARG_REGISTERS.len() {
        return Err(BackendError::UnsupportedPattern { detail: "too many parameters".into() });
    }
    Ok(params)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::lir::{LirBlock, LirInstruction};
    use rv8_ir::son::{BlockId, IrType};
    use rv8_core::rng::SplitMix64;


    /// CEP:WHAT: Registers one GPR value.
    /// CEP:WHY: Test brevity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn val(f: &mut LirFunction) -> ValueId {
        f.add_value(IrType::I64, RegClassId::Gpr)
    }

    /// CEP:WHAT: Plain LIR instruction constructor.
    /// CEP:WHY: Test brevity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn li(op: MachineOpcode, operands: Vec<ValueId>, result: Option<ValueId>, imm: i64) -> LirInstruction {
        LirInstruction { opcode: op, operands, result, immediate: imm, target_block: None }
    }

    /// CEP:WHAT: Cost of an assignment under the exact network table:
    /// register = 2*MOVE_COST (+SAVE_RESTORE_COST once when callee-saved, matching the
    /// network's single def-side surcharge), spill = SPILL_COST.
    /// CEP:WHY: The brute-force checker must price assignments identically to the network.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn assignment_cost(assign: &[Option<usize>]) -> i64 {
        assign
            .iter()
            .map(|a| match a {
                Some(r) => {
                    let save = if abi::is_callee_saved(abi::ALLOCATABLE_GPRS[*r]) {
                        SAVE_RESTORE_COST
                    } else {
                        0
                    };
                    2 * MOVE_COST + save
                }
                None => SPILL_COST,
            })
            .sum()
    }

    /// CEP:WHAT: Interference check under the exact lane model: two ranges on one
    /// register conflict iff their half-open lane intervals [def, use) intersect.
    /// CEP:WHY: The brute force must enforce the same capacity semantics as the lanes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: corpus instances contain no calls (no barrier rule to mirror).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn feasible(defs: &[u32], uses: &[u32], assign: &[Option<usize>], nregs: usize) -> bool {
        for a in 0..assign.len() {
            if let Some(ra) = assign[a] {
                if ra >= nregs {
                    return false;
                }
                for b in 0..assign.len() {
                    if b != a
                        && assign[b] == Some(ra)
                        && defs[a] < uses[b]
                        && defs[b] < uses[a]
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// CEP:WHAT: THE BRUTE-FORCE OPTIMALITY CORPUS: 24 randomized-but-seeded small
    /// interference instances (2-4 values, definition/use points drawn from SplitMix64
    /// with fixed seeds 0x9000..0x9018). For every instance, enumerate ALL 16^n
    /// assignments over the full 15-register GPR class including spill, take the minimum
    /// feasible cost under the identical cost table, and assert the MCF allocation's
    /// cost equals the brute-force optimum.
    /// CEP:WHY: STANDARDS 5.1 acceptance: "optimal for small instances against a
    /// brute-force checker" - non-negotiable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: corpus instances have no calls (no barrier rule to mirror) and no
    /// ABI pins (every corpus value is defined by an instruction).
    /// CEP:COST: test-only; 16^4 = 65536 assignments max per instance.
    /// CEP:EVIDENCE: this test IS the evidence.
    #[test]
    fn mcf_matches_bruteforce_optimum_corpus() {
        let nregs = abi::ALLOCATABLE_GPRS.len();
        let mut failures: Vec<String> = Vec::new();
        for seed in 0x9000u64..0x9000 + 24 {
            let mut rng = SplitMix64::seed_from_u64(seed);
            let n = 2 + (rng.next_u64() % 3) as usize; // 2..=4 values
            let span = 6u64;
            let mut defs = Vec::new();
            let mut uses = Vec::new();
            let mut f = LirFunction::new("corpus");
            let ids: Vec<ValueId> = (0..n).map(|_| val(&mut f)).collect();
            // Unique definition points (one def per program point, matching the
            // network's one-def-per-instruction shape); uses may collide on points.
            let mut used_points = std::collections::BTreeSet::new();
            for _ in 0..n {
                let mut d = (rng.next_u64() % span) as u32 + 1;
                while !used_points.insert(d) {
                    d = d % (span as u32) + 1;
                }
                let len = 1 + (rng.next_u64() % span) as u32;
                defs.push(d);
                uses.push(d + len);
            }
            let max_point = uses.iter().copied().max().unwrap_or(1);
            // One instruction per point: its def (fresh dead value when none) plus the
            // use operands attached to this point.
            let mut point_defs: Vec<Option<ValueId>> = vec![None; max_point as usize];
            let mut point_uses: Vec<Vec<ValueId>> = vec![Vec::new(); max_point as usize];
            let mut ord: Vec<usize> = (0..n).collect();
            ord.sort_by_key(|&i| (defs[i], ids[i]));
            for &i in &ord {
                point_defs[(defs[i] - 1) as usize] = Some(ids[i]);
            }
            for i in 0..n {
                point_uses[(uses[i] - 1) as usize].push(ids[i]);
            }
            let mut block = LirBlock::new(BlockId(0));
            for p in 0..max_point as usize {
                // Filler points carry no definition at all: a dead definition would
                // occupy a lane in the network yet be invisible to the brute-force
                // table, which would break the exactness comparison.
                let mut instr = li(MachineOpcode::MovRegImm32, vec![], point_defs[p], p as i64);
                instr.operands = point_uses[p].clone();
                block.push(instr);
            }
            block.push(li(MachineOpcode::Ret, vec![], None, 0));
            f.push_block(block);

            let alloc = allocate(&f)
                .unwrap_or_else(|e| panic!("seed {seed}: allocate failed: {e}"));

            // Brute force over all (nregs+1)^n assignments including spill.
            let mut best: Option<i64> = None;
            let total = (nregs + 1).pow(n as u32);
            for code in 0..total {
                let mut assign = vec![None; n];
                let mut c = code;
                for a in assign.iter_mut().take(n) {
                    let choice = c % (nregs + 1);
                    c /= nregs + 1;
                    *a = if choice == nregs { None } else { Some(choice) };
                }
                if feasible(&defs, &uses, &assign, nregs) {
                    let cost = assignment_cost(&assign);
                    best = Some(best.map_or(cost, |b: i64| b.min(cost)));
                }
            }
            let brute = best.expect("spill-only assignment is always feasible");

            let mcf_assign: Vec<Option<usize>> = (0..n)
                .map(|i| alloc.register(ids[i]).map(|r| r.0 as usize))
                .collect();
            assert!(
                feasible(&defs, &uses, &mcf_assign, nregs),
                "seed {seed}: MCF assignment infeasible: {mcf_assign:?}"
            );
            let mcf_cost = assignment_cost(&mcf_assign);
            if mcf_cost != brute {
                failures.push(format!(
                    "seed {seed}: mcf {mcf_cost} != brute {brute} (defs {defs:?} uses {uses:?} got {mcf_assign:?})"
                ));
            }
        }
        assert!(failures.is_empty(), "optimality failures:\n{}", failures.join("\n"));
    }

    /// CEP:WHAT: Allocation is deterministic: allocate twice (plus full rewrite twice)
    /// yields identical RegisterAllocations and identical MIR canonical hashes.
    /// CEP:WHY: 38.10/38.19 acceptance.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mcf_allocation_is_deterministic() {
        let build = || {
            let mut f = LirFunction::new("det");
            let a = val(&mut f);
            let b = val(&mut f);
            let c = val(&mut f);
            let d = val(&mut f);
            let e = val(&mut f);
            let mut blk0 = LirBlock::new(BlockId(0));
            blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(a), 1));
            blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(b), 2));
            blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(c), 3));
            blk0.push(li(MachineOpcode::AddRegReg, vec![a, b], Some(d), 0));
            blk0.push(li(MachineOpcode::AddRegReg, vec![c, d], Some(e), 0));
            blk0.push(li(MachineOpcode::Ret, vec![e], None, 0));
            f.push_block(blk0);
            f
        };
        let f1 = build();
        let f2 = build();
        let a1 = allocate(&f1).expect("alloc 1");
        let a2 = allocate(&f2).expect("alloc 2");
        assert_eq!(format!("{a1:?}"), format!("{a2:?}"));
        let m1 = crate::isel::select(&f1).and_then(|mut m| {
            rewrite(&mut m, &f1, &a1)?;
            Ok(m)
        }).expect("rewrite 1");
        let m2 = crate::isel::select(&f2).and_then(|mut m| {
            rewrite(&mut m, &f2, &a2)?;
            Ok(m)
        }).expect("rewrite 2");
        assert_eq!(m1.canonical_hash(), m2.canonical_hash());
        m1.verify().expect("rewritten MIR verifies");
    }

    /// CEP:WHAT: Over-constrained pressure: eight values live across a call, which
    /// blocks caller-saved lanes and leaves only six callee-saved registers; allocation
    /// succeeds via spills, rewritten MIR verifies, two runs identical.
    /// CEP:WHY: STANDARDS 5.1 acceptance: spills keep feasibility by construction.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mcf_spills_under_pressure_and_verifies() {
        let build = || {
            let mut f = LirFunction::new("pressure");
            let vs: Vec<ValueId> = (0..8).map(|_| val(&mut f)).collect();
            let mut blk0 = LirBlock::new(BlockId(0));
            for (i, &v) in vs.iter().enumerate() {
                blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), i as i64 + 1));
            }
            blk0.push(LirInstruction {
                opcode: MachineOpcode::Call,
                operands: vec![],
                result: None,
                immediate: 1,
                target_block: None,
            });
            let mut acc = vs[0];
            for &v in vs.iter().skip(1) {
                let s = val(&mut f);
                blk0.push(li(MachineOpcode::AddRegReg, vec![acc, v], Some(s), 0));
                acc = s;
            }
            blk0.push(li(MachineOpcode::Ret, vec![acc], None, 0));
            f.push_block(blk0);
            f
        };
        let f1 = build();
        let f2 = build();
        let a1 = allocate(&f1).expect("pressure allocation succeeds");
        let a2 = allocate(&f2).expect("second run");
        assert_eq!(format!("{a1:?}"), format!("{a2:?}"));
        let spilled = a1.spill_slots.iter().flatten().count();
        assert!(spilled > 0, "pressure must force spills");
        let mut mir = crate::isel::select(&f1).expect("isel");
        rewrite(&mut mir, &f1, &a1).expect("rewrite");
        mir.verify().expect("spill-rewritten MIR verifies");
        let text = mir.print();
        assert!(
            text.contains("loadstack") || text.contains("storestack"),
            "spill code must appear:\n{text}"
        );
    }

    /// CEP:WHAT: ABI pre-coloring: parameters land in their SysV argument registers and
    /// the returned value is pinned to RAX.
    /// CEP:WHY: STANDARDS 5.9 acceptance: pre-coloring constrains the allocator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mcf_respects_abi_precoloring() {
        let mut f = LirFunction::new("abi");
        let a = val(&mut f); // param 0 -> RDI
        let b = val(&mut f); // param 1 -> RSI
        let s = val(&mut f);
        let mut blk0 = LirBlock::new(BlockId(0));
        blk0.push(li(MachineOpcode::AddRegReg, vec![a, b], Some(s), 0));
        blk0.push(li(MachineOpcode::Ret, vec![s], None, 0));
        f.push_block(blk0);
        let alloc = allocate(&f).expect("alloc");
        let rdi_idx = abi::ALLOCATABLE_GPRS.iter().position(|&p| p == abi::RDI).unwrap() as u32;
        let rsi_idx = abi::ALLOCATABLE_GPRS.iter().position(|&p| p == abi::RSI).unwrap() as u32;
        assert_eq!(alloc.register(a), Some(RegId(rdi_idx)));
        assert_eq!(alloc.register(b), Some(RegId(rsi_idx)));
        assert_eq!(alloc.register(s), Some(RegId(0)), "return value pinned to RAX");
    }

    /// CEP:WHAT: A value live across a call is never allocated to a caller-saved
    /// register (or it is spilled): the lane barrier is enforced end to end.
    /// CEP:WHY: Correctness of calls, not just cost.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mcf_avoids_caller_saved_across_calls() {
        let mut f = LirFunction::new("across");
        let v = val(&mut f); // defined before the call, used after
        let w = val(&mut f);
        let mut blk0 = LirBlock::new(BlockId(0));
        blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), 5));
        blk0.push(LirInstruction {
            opcode: MachineOpcode::Call,
            operands: vec![],
            result: None,
            immediate: 3,
            target_block: None,
        });
        blk0.push(li(MachineOpcode::AddRegReg, vec![v, v], Some(w), 0));
        blk0.push(li(MachineOpcode::Ret, vec![w], None, 0));
        f.push_block(blk0);
        let alloc = allocate(&f).expect("alloc");
        if let Some(r) = alloc.register(v) {
            let p = abi::gpr_phys(r).expect("gpr");
            assert!(!abi::is_caller_saved(p), "v lives across a call in caller-saved {p:?}");
        }
        let mut mir = crate::isel::select(&f).expect("isel");
        rewrite(&mut mir, &f, &alloc).expect("rewrite");
        mir.verify().expect("verify");
    }

    /// CEP:WHAT: Spill path end to end: 16 simultaneously-live values (more than the 15
    /// allocatable GPRs) force spills; LoadStack/StoreStack appear with correct operand
    /// shapes, verify() stays green, two runs are identical, and the frame plan is
    /// 16-aligned.
    /// CEP:WHY: Required spill-path test.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn spill_path_inserts_loads_and_stores() {
        let build = || {
            let mut f = LirFunction::new("spills");
            let vs: Vec<ValueId> = (0..16).map(|_| val(&mut f)).collect();
            let mut blk0 = LirBlock::new(BlockId(0));
            for (i, &v) in vs.iter().enumerate() {
                blk0.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), i as i64));
            }
            let mut acc = vs[0];
            for &v in vs.iter().skip(1) {
                let s = val(&mut f);
                blk0.push(li(MachineOpcode::AddRegReg, vec![acc, v], Some(s), 0));
                acc = s;
            }
            blk0.push(li(MachineOpcode::Ret, vec![acc], None, 0));
            f.push_block(blk0);
            f
        };
        let f1 = build();
        let f2 = build();
        let a1 = allocate(&f1).expect("alloc 1");
        let a2 = allocate(&f2).expect("alloc 2");
        assert_eq!(format!("{a1:?}"), format!("{a2:?}"));
        assert!(a1.spill_slots.iter().flatten().count() > 0, "must spill");
        let mut m1 = crate::isel::select(&f1).expect("isel 1");
        let plan1 = rewrite(&mut m1, &f1, &a1).expect("rewrite 1");
        let mut m2 = crate::isel::select(&f2).expect("isel 2");
        let plan2 = rewrite(&mut m2, &f2, &a2).expect("rewrite 2");
        assert_eq!(m1.canonical_hash(), m2.canonical_hash());
        m1.verify().expect("verify 1");
        assert_eq!(plan1.frame.stack_size, plan2.frame.stack_size);
        assert_eq!(plan1.frame.alignment, 16);
        let mut loads = 0;
        let mut stores = 0;

        for b in &m1.blocks {
            for i in &b.instructions {
                match i.opcode {
                    MachineOpcode::LoadStack => {
                        loads += 1;
                        assert!(matches!(i.operands[0], Reg(_)));
                        assert!(matches!(i.operands[1], MachineOperand::Mem(_)));
                    }
                    MachineOpcode::StoreStack => {
                        stores += 1;
                        assert!(matches!(i.operands[0], MachineOperand::Mem(_)));
                        assert!(matches!(i.operands[1], Reg(_)));
                    }
                    _ => {}
                }
            }
        }
        assert!(loads > 0 && stores > 0, "both spill directions must appear");
    }
}
