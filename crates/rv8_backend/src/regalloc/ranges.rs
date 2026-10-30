// CEP:FILE: crates/rv8_backend/src/regalloc/ranges.rs
// CEP:WHAT: LIR facts for allocation: linearized program points, live ranges under
//   the conservative wrap-sound rule (CFG back-edge liveness plus multi-def values,
//   STANDARDS convention pin 3), and the call/divide barrier masks per program point.
// CEP:WHY: The network is built from these facts; keeping derivation in one module
//   makes the interval contract auditable against the brute-force oracles.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: IllegalLir for linearized-liveness violations and dangling ids;
//   BudgetExceeded past MAX_LIVENESS_ROUNDS.
// CEP:ASSUMES: block vector order is layout order; at most 64 lanes per class.
// CEP:COST: O(instructions + values) per pass.
// CEP:EVIDENCE: the RV8-137 regression batch and both brute-force corpora.
use super::desc::Plan;
use crate::error::BackendError;
use rv8_ir::lir::{LirFunction, MachineOpcode, RegClassId, ValueId};
use rv8_ir::machir::PhysReg;

// ---------------------------------------------------------------------------
// LIR facts shared by allocate_with and rewrite()
// ---------------------------------------------------------------------------

/// CEP:WHAT: One live range: value id, linearized definition point, last-use point,
/// register class, and optional ABI pin.
/// CEP:WHY: The commodity of the MCF network; points are instruction ordinals across
/// blocks in layout order with point 0 = function entry.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: def <= last_use (checked in build_ranges); for multi-def (phi) values
/// last_use is the final program point (the conservative wrap-sound interval).
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveRange {
    pub(crate) value: ValueId,
    pub(crate) def: u32,
    pub(crate) last_use: u32,
    pub(crate) class: RegClassId,
    pub(crate) pin: Option<PhysReg>,
}

/// CEP:WHAT: Maximum backward-liveness fixpoint rounds per function.
/// CEP:WHY: Named budget (38.12); the dataflow converges in O(block depth) rounds.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in build_ranges.
pub const MAX_LIVENESS_ROUNDS: usize = 10_000;

/// CEP:WHAT: Computes the linearized program points and live ranges for a function.
/// CEP:WHY: The network is built over these points; shared facts for allocate/rewrite.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalLir for double definitions of single-def values, dangling ids,
/// and uses that precede their definition in layout order (the linearized-liveness
/// contract).
/// CEP:ASSUMES: point 0 is function entry; the instruction at global ordinal g is
/// point g+1. Multi-def values (phi edge copies, LIR 4.3.1) take the conservative
/// wrap-sound interval [first_def, last_point]: each interior def-write happens at a
/// point the lane must stay reserved for, and the value is live across the back-edge
/// to uses that sit EARLIER in layout order (loop headers), which no half-open
/// linear interval ending at a use can express. Single-def values never wrap: the
/// linearized-liveness contract rejects use-before-def, so [def, last_use] covers
/// their whole live set.
/// CEP:COST: O(instructions + values).
/// CEP:EVIDENCE: internal; behavior pinned by all regalloc tests, including the
/// RV8-137 regression batch and the multi-def optimality corpus.
pub(crate) fn build_ranges(
    plan: &Plan,
    lir: &LirFunction,
) -> Result<(Vec<LiveRange>, u32), BackendError> {
    let n = lir.values.len();
    let mut total_instrs: u32 = 0;
    for b in &lir.blocks {
        total_instrs += b.instructions.len() as u32;
    }
    let points: u32 = 1 + total_instrs;

    let mut def_point: Vec<Option<u32>> = vec![None; n];
    let mut last_def: Vec<u32> = vec![0; n];
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
                // written once per incoming path. The FIRST definition anchors the
                // range; every def is a spill store site; interior rewrites are
                // ordinary writes within the routed register.
                if def_point[v].is_none() {
                    def_point[v] = Some(point);
                }
                last_def[v] = point;
            }
        }
    }

    // Parameters (never defined, used somewhere) in value-id order pin to ABI arg regs.
    //
    // First: CFG liveness (block-level gen/kill, backward fixpoint) to detect values
    // live across BACK-EDGES. Linear layout order linearizes liveness only for
    // forward flows: a value that is live-out of a block with a layout-earlier
    // successor is re-read after the wrap, so no half-open linear interval ending at
    // its last layout use protects it (a single-def loop constant is re-read at the
    // header of every iteration after the latch rewrote its register - exactly the
    // zero-constant clobber on RV8-137). Such values take the conservative wrap-sound
    // interval [def, last_point]. Multi-def (phi) values always qualify (their uses
    // sit in the wrap-target block), and the explicit multi_def rule below stays as
    // defense in depth.
    let block_index_of_id: Vec<usize> = {
        let mut m = vec![usize::MAX; lir.blocks.len()];
        for (bi, b) in lir.blocks.iter().enumerate() {
            if (b.id.0 as usize) < m.len() {
                m[b.id.0 as usize] = bi;
            }
        }
        m
    };
    let mut gen: Vec<Vec<bool>> = vec![vec![false; n]; lir.blocks.len()];
    let mut kill: Vec<Vec<bool>> = vec![vec![false; n]; lir.blocks.len()];
    {
        for (bi, block) in lir.blocks.iter().enumerate() {
            for instr in &block.instructions {
                for &op in &instr.operands {
                    let v = op.0 as usize;
                    if !kill[bi][v] {
                        gen[bi][v] = true;
                    }
                }
                if let Some(r) = instr.result {
                    kill[bi][r.0 as usize] = true;
                }
            }
        }
    }
    let mut live_out: Vec<Vec<bool>> = vec![vec![false; n]; lir.blocks.len()];
    let mut live_in: Vec<Vec<bool>> = gen.clone();
    let mut converged = false;
    for _round in 0..MAX_LIVENESS_ROUNDS {
        let mut changed = false;
        for bi in (0..lir.blocks.len()).rev() {
            // live_out = union of live_in over CFG successors.
            for &s in &lir.blocks[bi].successors {
                let si = match block_index_of_id.get(s.0 as usize) {
                    Some(&i) if i != usize::MAX => i,
                    _ => continue,
                };
                for v in 0..n {
                    if live_in[si][v] && !live_out[bi][v] {
                        live_out[bi][v] = true;
                        changed = true;
                    }
                }
            }
            // live_in = gen | (live_out & !kill).
            for v in 0..n {
                let nv = gen[bi][v] || (live_out[bi][v] && !kill[bi][v]);
                if nv != live_in[bi][v] {
                    live_in[bi][v] = nv;
                    changed = true;
                }
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err(BackendError::BudgetExceeded { budget: "MAX_LIVENESS_ROUNDS" });
    }
    // A block index whose CFG successor is earlier in layout order carries a back-edge.
    let mut wraps: Vec<bool> = vec![false; n];
    for (bi, block) in lir.blocks.iter().enumerate() {
        for &s in &block.successors {
            let si = match block_index_of_id.get(s.0 as usize) {
                Some(&i) if i != usize::MAX => i,
                _ => continue,
            };
            if si <= bi {
                for v in 0..n {
                    if live_out[bi][v] {
                        wraps[v] = true;
                    }
                }
            }
        }
    }

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
                if param_index >= plan.abi_args.len() {
                    return Err(BackendError::UnsupportedPattern { detail: format!(
                        "more than {} parameters; stack arguments are not implemented",
                        plan.abi_args.len()) });
                }
                if lir.values[v].reg_class != RegClassId::Gpr {
                    return Err(BackendError::UnsupportedPattern { detail: format!(
                        "parameter {} is not GPR class", v) });
                }
                let pin = plan.abi_args[param_index];
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
        // layout order is an error. A multi-def value takes the wrap-sound interval
        // [first_def, last_point]: its register is reserved from the first edge copy
        // to the end of the layout so no interior rewrite is clobbered and no single-
        // def value steals the lane across the loop back-edge (RV8-137).
        let multi_def = last_def[v] > def;
        let range_last = if multi_def || wraps[v] {
            points - 1
        } else if last_use[v] <= def {
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
    // range (documented in isel: the ABI move writes RAX). A value that already
    // carries a pin (a parameter pre-colored to its ABI argument register) keeps it:
    // isel emits the register-to-register ABI move, so overwriting the entry pin
    // would lose the incoming value entirely.
    for block in &lir.blocks {
        for instr in &block.instructions {
            if instr.opcode == MachineOpcode::Ret {
                if let Some(&v) = instr.operands.first() {
                    for range in ranges.iter_mut() {
                        if range.value == v && range.pin.is_none() {
                            range.pin = Some(plan.abi_return);
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
pub(crate) struct Barriers {
    pub(crate) after: Vec<u64>,
    pub(crate) before: Vec<u64>,
}

/// CEP:WHAT: Builds the barrier masks for one register class from the LIR opcode stream.
/// CEP:WHY: See Barriers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: call argument count <= 6 (isel enforces); regs outside the class map to
/// no bit.
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
pub(crate) fn build_barriers(
    plan: &Plan,
    lir: &LirFunction,
    points: u32,
    class_idx: usize,
) -> Barriers {
    let lanes = &plan.classes[class_idx].lanes;
    let is_gpr = class_idx == 0;
    let bit_of = |r: PhysReg| -> u64 {
        lanes
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
                    // Calls clobber the class's caller-saved lanes (from the ISL
                    // description: every non-callee-saved lane).
                    for (i, saved) in plan.classes[class_idx].callee_saved.iter().enumerate() {
                        if !saved {
                            b.after[p] |= 1u64 << i;
                        }
                    }
                    if is_gpr {
                        // Argument moves write arg registers in the window before the
                        // call instruction (isel emits them at this LIR point).
                        let nargs = instr.operands.len().min(plan.abi_args.len());
                        if p > 0 {
                            for &r in plan.abi_args.iter().take(nargs) {
                                b.before[p] |= bit_of(r);
                            }
                        }
                    }
                }
                MachineOpcode::Cqo | MachineOpcode::IdivReg if is_gpr => {
                    let mut mask = 0u64;
                    for (i, d) in plan.classes[0].div_clobber.iter().enumerate() {
                        if *d {
                            mask |= 1u64 << i;
                        }
                    }
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
