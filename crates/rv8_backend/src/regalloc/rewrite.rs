// CEP:FILE: crates/rv8_backend/src/regalloc/rewrite.rs
// CEP:WHAT: The spill/prologue/epilogue rewriter: vregs become physical registers,
//   spilled values get LoadStack/StoreStack with deterministic scratch selection, the
//   prologue and epilogues are synthesized from the frame plan, and spilled parameters
//   store their ABI register at entry.
// CEP:WHY: Turns the allocation into verified Machine IR (STANDARDS 5.1 public
//   entry point); isolated from the solver so the emission contract is auditable.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: UnsupportedPattern for unknown classes; Internal for scratch
//   exhaustion (unreachable: at most 15 register operands fit one instruction).
// CEP:ASSUMES: mir came from isel::select over the same lir; alloc came from
//   allocate_with over the same lir.
// CEP:COST: O(mir instructions x operands).
// CEP:EVIDENCE: spill_path_inserts_loads_and_stores plus verify() in every pipeline
//   test and the RV8-137 regression batch.
use crate::abi::{self, FrameCalculator, FrameLayout};
use crate::error::BackendError;
use rv8_ir::lir::{LirFunction, MachineOpcode, RegClassId, RegisterAllocation, ValueId};
use rv8_ir::machir::{
    MachineBlock, MachineInstr, MachineIrFunction, MachineOperand, MemOperand, PhysReg,
};
use MachineOperand::Reg;

/// CEP:WHAT: The rewrite result: ABI-conformant MachineFrame plus every emission fact
///   (slot offsets, stack adjustment, saved-register order, red-zone decision).
/// CEP:WHY: Prologue/epilogue bytes, spill addressing, and frame metadata come from
///   one calculation; this is that calculation's output.
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

/// CEP:WHAT: Rewrites post-selection MIR in place: vregs become physical registers,
///   spilled values get LoadStack/StoreStack with deterministic scratch selection, the
///   prologue (push callee-saved; sub rsp) and epilogues (add rsp; pops) are inserted
///   (skipped entirely for red-zone leaf frames), spilled parameters store their ABI
///   register at entry, and MachineFrame is filled from the FrameCalculator.
/// CEP:WHY: STANDARDS 5.1 public entry point; turns allocated values into verified
///   Machine IR.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnsupportedPattern for unknown classes; Internal for scratch
///   exhaustion (unreachable: at most 15 register operands fit one instruction) and
///   mapping holes.
/// CEP:ASSUMES: mir came from isel::select over the same lir (vreg numbering
///   contract); alloc came from allocate_with over the same lir.
/// CEP:COST: O(mir instructions x operands).
/// CEP:EVIDENCE: spill_path_inserts_loads_and_stores plus the verify() call in every
///   pipeline test.
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
