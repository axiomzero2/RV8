// CEP:FILE: crates/rv8_backend/tests/rv8_137_regression.rs
// CEP:WHAT: The RV8-137 regression batch: twenty regression tests for the MCF
//   allocator defects fixed with the lane-private job-pair network (cross-lane flow,
//   multi-def and back-edge wrap liveness, surplus-take degeneracy, fill lane cost),
//   plus the two standing corpus tests the ticket requires: a multi-def brute-force
//   optimality corpus and a randomized lane-lock feasibility corpus.
// CEP:WHY: STANDARDS section 8 records RV8-137 as the open backend ticket; the fix is
//   only accepted with one regression test per documented failure shape (twenty) and
//   corpus coverage of multi-def instances.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (test-only).
// CEP:ASSUMES: the conservative interval rule mirrored here matches build_ranges:
//   multi-def values and values live across a back-edge take [first_def, last_point];
//   single-def forward values take [def, last_use].
// CEP:COST: test-only; corpus instances stay small (16^4 brute force worst case).
// CEP:EVIDENCE: this module.
use rv8_backend::abi;
use rv8_backend::isel;
use rv8_backend::regalloc::{allocate, allocate_with, rewrite};
use rv8_core::rng::SplitMix64;
use rv8_ir::lir::{
    LirBlock, LirFunction, LirInstruction, MachineOpcode, RegClassId, RegisterAllocation, RegId,
    SetKind, ValueId,
};
use rv8_ir::son::{BlockId, IrType};

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

/// CEP:WHAT: Registers one FPR value.
/// CEP:WHY: FPR regression coverage.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn fval(f: &mut LirFunction) -> ValueId {
    f.add_value(IrType::F64, RegClassId::Fpr)
}

/// CEP:WHAT: Plain LIR instruction constructor.
/// CEP:WHY: Test brevity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn li(
    op: MachineOpcode,
    operands: Vec<ValueId>,
    result: Option<ValueId>,
    imm: i64,
) -> LirInstruction {
    LirInstruction { opcode: op, operands, result, immediate: imm, target_block: None }
}

/// CEP:WHAT: The canonical loop LIR bundle: entry defines the zero constant, i0/acc0
/// and the phi edge copies; the header compares and branches; the body computes
/// t=acc+i and u=i+1; the latch redefines the phi values; the exit returns acc.
/// CEP:WHY: The exact shape that exposed RV8-137; shared by many regression tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
struct LoopLir {
    f: LirFunction,
    i: ValueId,
    acc: ValueId,
}

/// CEP:WHAT: Constructor for LoopLir.
/// CEP:WHY: See struct.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn build_loop() -> LoopLir {
    let mut f = LirFunction::new("loop");
    let zero = val(&mut f); // guard-test zero constant, used only in the header
    let acc0 = val(&mut f);
    let i0 = val(&mut f);
    let i = val(&mut f); // multi-def (entry + latch edge copies)
    let acc = val(&mut f); // multi-def
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(li(MachineOpcode::MovRegImm32, vec![], Some(zero), 0));
    b0.push(li(MachineOpcode::MovRegImm32, vec![], Some(acc0), 0));
    b0.push(li(MachineOpcode::MovRegImm32, vec![], Some(i0), 1));
    b0.push(li(MachineOpcode::MovRegReg, vec![i0], Some(i), 0));
    b0.push(li(MachineOpcode::MovRegReg, vec![acc0], Some(acc), 0));
    b0.push(li(MachineOpcode::Jmp, vec![], None, 0));
    let last = b0.instructions.len() - 1;
    b0.instructions[last].target_block = Some(BlockId(1));
    b0.successors = vec![BlockId(1)];
    f.push_block(b0);
    let mut b1 = LirBlock::new(BlockId(1));
    let six = val(&mut f);
    b1.push(li(MachineOpcode::MovRegImm32, vec![], Some(six), 6));
    b1.push(li(MachineOpcode::CmpRegReg, vec![i, six], None, 0));
    let flag = val(&mut f);
    b1.push(li(MachineOpcode::Setcc(SetKind::Lt), vec![], Some(flag), 0));
    b1.push(li(MachineOpcode::CmpRegReg, vec![flag, zero], None, 0));
    b1.push(li(
        MachineOpcode::Jcc(SetKind::Eq),
        vec![],
        None,
        0,
    ));
    let last = b1.instructions.len() - 1;
    b1.instructions[last].target_block = Some(BlockId(4));
    b1.successors = vec![BlockId(4), BlockId(2)];
    f.push_block(b1);
    let mut b2 = LirBlock::new(BlockId(2));
    let t = val(&mut f);
    b2.push(li(MachineOpcode::AddRegReg, vec![acc, i], Some(t), 0));
    let one = val(&mut f);
    b2.push(li(MachineOpcode::MovRegImm32, vec![], Some(one), 1));
    let u = val(&mut f);
    b2.push(li(MachineOpcode::AddRegReg, vec![one, i], Some(u), 0));
    b2.push(li(MachineOpcode::Jmp, vec![], None, 0));
    let last = b2.instructions.len() - 1;
    b2.instructions[last].target_block = Some(BlockId(3));
    b2.successors = vec![BlockId(3)];
    f.push_block(b2);
    let mut b3 = LirBlock::new(BlockId(3));
    b3.push(li(MachineOpcode::MovRegReg, vec![u], Some(i), 0));
    b3.push(li(MachineOpcode::MovRegReg, vec![t], Some(acc), 0));
    b3.push(li(MachineOpcode::Jmp, vec![], None, 0));
    let last = b3.instructions.len() - 1;
    b3.instructions[last].target_block = Some(BlockId(1));
    b3.successors = vec![BlockId(1)];
    f.push_block(b3);
    let mut b4 = LirBlock::new(BlockId(4));
    b4.push(li(MachineOpcode::Ret, vec![acc], None, 0));
    f.push_block(b4);
    LoopLir { f, i, acc }
}

/// CEP:WHAT: Mirrors the conservative interval rule of build_ranges for these tests:
/// multi-def values take [first_def, last_point]; the caller supplies the wrapped set
/// for single-def values; the rest take [def, last_use].
/// CEP:WHY: The feasibility oracle must use the same intervals as the allocator.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: the test knows which values wrap.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn intervals_of(
    defs: &[u32],
    last_uses: &[u32],
    extended: &[bool],
    last_point: u32,
) -> Vec<(u32, u32)> {
    defs.iter()
        .zip(last_uses.iter())
        .zip(extended.iter())
        .map(|((&d, &u), &m)| {
            if m {
                (d, last_point)
            } else {
                (d, u)
            }
        })
        .collect()
}

/// CEP:WHAT: Asserts that no two values allocated to the same class-local register
/// have overlapping half-open intervals under the conservative rule.
/// CEP:WHY: The structural feasibility property that RV8-137 broke.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: all checked values are GPR class.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn assert_lane_disjoint(
    alloc: &RegisterAllocation,
    ids: &[ValueId],
    intervals: &[(u32, u32)],
    what: &str,
) {
    for a in 0..ids.len() {
        let ra = alloc.register(ids[a]);
        for b in 0..ids.len() {
            if a == b {
                continue;
            }
            if alloc.register(ids[b]) == ra && ra.is_some() {
                let (lo_a, hi_a) = intervals[a];
                let (lo_b, hi_b) = intervals[b];
                assert!(
                    hi_a <= lo_b || hi_b <= lo_a,
                    "{what}: values {:?} and {:?} share lane {:?} with overlapping \
                     intervals {:?} and {:?}",
                    ids[a].0,
                    ids[b].0,
                    ra,
                    (lo_a, hi_a),
                    (lo_b, hi_b)
                );
            }
        }
    }
}

/// CEP:WHAT: Regression 01: the two loop phi values never share a lane.
/// CEP:WHY: The original RV8-137 report: two multi-def values were double-booked.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_01_loop_phis_differ() {
    let built = build_loop();
    let alloc = allocate(&built.f).expect("allocate loop");
    assert_ne!(
        alloc.register(built.i),
        alloc.register(built.acc),
        "the two multi-def phi values must sit in different registers"
    );
}

/// CEP:WHAT: Regression 02: a single-def constant live only in the header is protected
/// across the back-edge; no body value shares its lane.
/// CEP:WHY: The zero-constant clobber: a body value reused its lane after the last
/// layout use, and the wrap re-read garbage at the header.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: the loop shape of build_loop; point map in the body.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_02_wrapped_constant_protected() {
    let built = build_loop();
    let alloc = allocate(&built.f).expect("allocate loop");
    // Points: b0 6 instrs (1..6), b1 5 (7..11), b2 4 (12..15), b3 3 (16..18), b4 1
    // (19); points = 20, last point 19. Values: 0=zero(def1,use10,wraps), 1=acc0(2,5),
    // 2=i0(3,4), 3=i(4,wrap), 4=acc(5,wrap), 5=six(7,8), 6=flag(9,10), 7=t(12,17),
    // 8=one(13,14), 9=u(14,16). A def point and the last-use point of the value it
    // copies do not conflict: the move reads the source and writes the destination at
    // one point.
    let defs = [1u32, 2, 3, 4, 5, 7, 9, 12, 13, 14];
    let uses = [19u32, 5, 4, 19, 19, 8, 10, 17, 14, 16];
    let extended = [
        true, false, false, true, true, false, false, false, false, false,
    ];
    let intervals = intervals_of(&defs, &uses, &extended, 19);
    let ids: Vec<ValueId> = (0..10).map(ValueId).collect();
    assert_lane_disjoint(&alloc, &ids, &intervals, "wrapped constant");
}

/// CEP:WHAT: Regression 03: allocation + rewrite on the loop verifies and both phi
/// edge copies target the same physical register.
/// CEP:WHY: The rewrites of a multi-def value must land in one register.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_03_phi_rewrites_share_register() {
    let built = build_loop();
    let alloc = allocate(&built.f).expect("allocate");
    let mut mir = isel::select(&built.f).expect("isel");
    rewrite(&mut mir, &built.f, &alloc).expect("rewrite");
    mir.verify().expect("verified MIR");
    let ri = alloc.register(built.i).expect("i allocated");
    let pi = abi::gpr_phys(ri).expect("physical");
    let i0_phys: rv8_ir::machir::PhysReg = {
        let r = alloc.register(ValueId(2)).expect("i0 allocated");
        abi::gpr_phys(r).expect("physical")
    };
    // The entry edge copy reads i0's register and writes i's register; locate it by
    // source (the rewrite inserts prologue instructions before it).
    let find_copy = |block: &rv8_ir::machir::MachineBlock, src: rv8_ir::machir::PhysReg| -> rv8_ir::machir::PhysReg {
        for instr in &block.instructions {
            if instr.opcode == MachineOpcode::MovRegReg && instr.operands.len() == 2 {
                if let (rv8_ir::machir::MachineOperand::Reg(d), rv8_ir::machir::MachineOperand::Reg(s)) =
                    (&instr.operands[0], &instr.operands[1])
                {
                    if *s == src {
                        return *d;
                    }
                }
            }
        }
        panic!("copy with source {src:?} not found");
    };
    let entry_dest = find_copy(&mir.blocks[0], i0_phys);
    let latch_dest = find_copy(&mir.blocks[3], {
        let r = alloc.register(ValueId(9)).expect("u allocated");
        abi::gpr_phys(r).expect("physical")
    });
    assert_eq!(entry_dest, pi, "entry edge copy writes i's register");
    assert_eq!(latch_dest, pi, "latch edge copy writes i's register");
}

/// CEP:WHAT: Regression 04: two multi-def values never share a lane.
/// CEP:WHY: Interior def-writes of one value must not clobber the other.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_04_two_multi_def_differ() {
    let mut f = LirFunction::new("twophi");
    let a = val(&mut f);
    let b = val(&mut f);
    let a0 = val(&mut f);
    let b0 = val(&mut f);
    let a1 = val(&mut f);
    let b1 = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(a0), 1));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(b0), 2));
    blk.push(li(MachineOpcode::MovRegReg, vec![a0], Some(a), 0));
    blk.push(li(MachineOpcode::MovRegReg, vec![b0], Some(b), 0));
    blk.push(li(MachineOpcode::MovRegReg, vec![a], Some(a1), 0));
    blk.push(li(MachineOpcode::MovRegReg, vec![b], Some(b1), 0));
    blk.push(li(MachineOpcode::Ret, vec![a1], None, 0));
    f.push_block(blk);
    let alloc = allocate(&f).expect("allocate");
    assert_ne!(alloc.register(a), alloc.register(b), "multi-def values differ");
}

/// CEP:WHAT: Regression 05: a single-def value whose linear interval sits inside a
/// multi-def value's extended span never shares its lane.
/// CEP:WHY: The wrap extension must actually exclude other values.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_05_inside_span_excluded() {
    let mut f = LirFunction::new("span");
    let phi = val(&mut f);
    let phi0 = val(&mut f);
    let inner = val(&mut f);
    let mut b = LirBlock::new(BlockId(0));
    b.push(li(MachineOpcode::MovRegImm32, vec![], Some(phi0), 1));
    b.push(li(MachineOpcode::MovRegReg, vec![phi0], Some(phi), 0));
    b.push(li(MachineOpcode::MovRegImm32, vec![], Some(inner), 2));
    b.push(li(MachineOpcode::AddRegReg, vec![inner, phi], Some(phi), 0));
    b.push(li(MachineOpcode::Ret, vec![phi], None, 0));
    f.push_block(b);
    let alloc = allocate(&f).expect("allocate");
    assert_ne!(
        alloc.register(phi),
        alloc.register(inner),
        "inner value overlaps the phi extended span"
    );
}

/// CEP:WHAT: Regression 06: allocation is deterministic on the loop shape.
/// CEP:WHY: The repair pass must not introduce run-to-run variation.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_06_deterministic_repair() {
    let a = build_loop();
    let b = build_loop();
    let ra = allocate(&a.f).expect("allocate a");
    let rb = allocate(&b.f).expect("allocate b");
    assert_eq!(format!("{ra:?}"), format!("{rb:?}"), "identical allocations");
}

/// CEP:WHAT: Regression 07: the surplus-take degeneracy cannot spill ranges that fit
/// (ten interleaved ranges, fifteen lanes).
/// CEP:WHY: Idle units re-taking popular ranges displaced fresh ones.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_07_no_unjustified_spills() {
    let mut f = LirFunction::new("pop");
    let vs: Vec<ValueId> = (0..10).map(|_| val(&mut f)).collect();
    let mut b = LirBlock::new(BlockId(0));
    // Single-def chains: v_i defined once, consumed once by the next link; intervals
    // stay short and disjoint, so a spill is never justified.
    let mut prev: Option<ValueId> = None;
    for (i, &v) in vs.iter().enumerate() {
        b.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), i as i64));
        let d = val(&mut f);
        b.push(li(MachineOpcode::MovRegImm32, vec![], Some(d), i as i64 + 100));
        let s = val(&mut f);
        match prev {
            Some(p) => b.push(li(MachineOpcode::AddRegReg, vec![p, d], Some(s), 0)),
            None => b.push(li(MachineOpcode::AddRegReg, vec![v, d], Some(s), 0)),
        }
        prev = Some(s);
    }
    b.push(li(MachineOpcode::Ret, vec![prev.expect("chain")], None, 0));
    f.push_block(b);
    let alloc = allocate(&f).expect("allocate");
    assert!(
        alloc.spill_slots.iter().flatten().count() == 0,
        "no value needs a spill here: {:?}",
        alloc.spill_slots
    );
}

/// CEP:WHAT: Regression 08: the fill prefers caller-saved lanes (four
/// pairwise-overlapping ranges, the corpus seed 0x9010 shape).
/// CEP:WHY: The first idle lane was a callee-saved register; the exact optimum uses
/// caller-saved lanes only.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: ALLOCATABLE_GPRS lane order per abi.rs.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_08_fill_prefers_caller_saved() {
    let mut f = LirFunction::new("seed9010");
    let vs: Vec<ValueId> = (0..4).map(|_| val(&mut f)).collect();
    let defs = [4u32, 6, 5, 3];
    let uses = [8u32, 11, 8, 8];
    let max_point = 11u32;
    let mut point_defs: Vec<Option<ValueId>> = vec![None; max_point as usize];
    for (i, &v) in vs.iter().enumerate() {
        point_defs[(defs[i] - 1) as usize] = Some(v);
    }
    let mut point_uses: Vec<Vec<ValueId>> = vec![Vec::new(); max_point as usize];
    for (i, &v) in vs.iter().enumerate() {
        point_uses[(uses[i] - 1) as usize].push(v);
    }
    let mut b = LirBlock::new(BlockId(0));
    for p in 0..max_point as usize {
        let mut instr = li(MachineOpcode::MovRegImm32, vec![], point_defs[p], p as i64);
        instr.operands = point_uses[p].clone();
        b.push(instr);
    }
    b.push(li(MachineOpcode::Ret, vec![], None, 0));
    f.push_block(b);
    let alloc = allocate(&f).expect("allocate");
    for (i, &v) in vs.iter().enumerate() {
        if let Some(r) = alloc.register(v) {
            let p = abi::gpr_phys(r).expect("physical");
            assert!(
                !abi::is_callee_saved(p),
                "value {i} (def {} use {}) landed in callee-saved {p:?}",
                defs[i],
                uses[i]
            );
        }
    }
}

/// CEP:WHAT: Regression 09: a ret operand pins its value to RAX.
/// CEP:WHY: The return-value contract the lowering now threads.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: RAX is allocatable.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_09_ret_operand_pins_rax() {
    let mut f = LirFunction::new("retpin");
    let a = val(&mut f);
    let b = val(&mut f);
    let s = val(&mut f);
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(li(MachineOpcode::MovRegImm32, vec![], Some(a), 1));
    b0.push(li(MachineOpcode::MovRegImm32, vec![], Some(b), 2));
    b0.push(li(MachineOpcode::AddRegReg, vec![a, b], Some(s), 0));
    b0.push(li(MachineOpcode::Ret, vec![s], None, 0));
    f.push_block(b0);
    let alloc = allocate(&f).expect("allocate");
    let rax_index = abi::ALLOCATABLE_GPRS
        .iter()
        .position(|&p| p == abi::RETURN_REGISTER)
        .expect("rax allocatable") as u32;
    assert_eq!(alloc.register(s), Some(RegId(rax_index)));
}

/// CEP:WHAT: Regression 10: a parameter returned directly keeps its entry pin.
/// CEP:WHY: The ret pin must not overwrite the ABI argument pin.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: RDI is the first argument register.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_10_param_pin_survives_ret() {
    let mut f = LirFunction::new("ident");
    let x = val(&mut f); // parameter: never defined, used at the ret
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(li(MachineOpcode::Ret, vec![x], None, 0));
    f.push_block(b0);
    let alloc = allocate(&f).expect("allocate");
    let rdi_index = abi::ALLOCATABLE_GPRS
        .iter()
        .position(|&p| p == abi::ARG_REGISTERS[0])
        .expect("rdi allocatable") as u32;
    assert_eq!(
        alloc.register(x),
        Some(RegId(rdi_index)),
        "the parameter keeps RDI; isel emits the register-to-register ABI move"
    );
}

/// CEP:WHAT: Regression 11: a parameter and a separately computed return value take
/// their own pins (RDI and RAX respectively).
/// CEP:WHY: Both ABI constraints must hold simultaneously.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_11_param_and_return_pins() {
    let mut f = LirFunction::new("pinpair");
    let x = val(&mut f);
    let s = val(&mut f);
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(li(MachineOpcode::AddRegReg, vec![x, x], Some(s), 0));
    b0.push(li(MachineOpcode::Ret, vec![s], None, 0));
    f.push_block(b0);
    let alloc = allocate(&f).expect("allocate");
    let rdi_index = abi::ALLOCATABLE_GPRS
        .iter()
        .position(|&p| p == abi::ARG_REGISTERS[0])
        .expect("rdi") as u32;
    let rax_index = abi::ALLOCATABLE_GPRS
        .iter()
        .position(|&p| p == abi::RETURN_REGISTER)
        .expect("rax") as u32;
    assert_eq!(alloc.register(x), Some(RegId(rdi_index)));
    assert_eq!(alloc.register(s), Some(RegId(rax_index)));
}

/// CEP:WHAT: Regression 12: a parameter live across a call spills (its pinned lane is
/// caller-clobbered), the rewrite stores it at entry, and the MIR verifies.
/// CEP:WHY: Pinned-lane barriers must keep feasibility absolute.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_12_pinned_param_across_call_spills() {
    let mut f = LirFunction::new("paramcall");
    let x = val(&mut f);
    let s = val(&mut f);
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(LirInstruction {
        opcode: MachineOpcode::Call,
        operands: vec![],
        result: None,
        immediate: 3,
        target_block: None,
    });
    b0.push(li(MachineOpcode::AddRegReg, vec![x, x], Some(s), 0));
    b0.push(li(MachineOpcode::Ret, vec![s], None, 0));
    f.push_block(b0);
    let alloc = allocate(&f).expect("allocate");
    assert!(
        alloc.register(x).is_none(),
        "the pinned parameter crosses a call and must spill"
    );
    assert!(alloc.spill_slots[x.0 as usize].is_some(), "spill slot assigned");
    let mut mir = isel::select(&f).expect("isel");
    rewrite(&mut mir, &f, &alloc).expect("rewrite");
    mir.verify().expect("verified MIR");
}

/// CEP:WHAT: Regression 13: two multi-def values rewritten on consecutive latch points
/// keep distinct lanes end to end (allocate + rewrite + verify).
/// CEP:WHY: The exact interference the original ticket named.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_13_latch_rewrites_verified() {
    let built = build_loop();
    let alloc = allocate(&built.f).expect("allocate");
    let mut mir = isel::select(&built.f).expect("isel");
    rewrite(&mut mir, &built.f, &alloc).expect("rewrite");
    mir.verify().expect("verified");
    assert_ne!(alloc.register(built.i), alloc.register(built.acc));
}

/// CEP:WHAT: Regression 14: the repair is deterministic on a multi-def shape.
/// CEP:WHY: 38.10: same input, same allocation, after the repair passes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_14_multi_def_determinism() {
    let build = || {
        let mut f = LirFunction::new("md");
        let a = val(&mut f);
        let a0 = val(&mut f);
        let b = val(&mut f);
        let b0 = val(&mut f);
        let mut blk = LirBlock::new(BlockId(0));
        blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(a0), 1));
        blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(b0), 2));
        blk.push(li(MachineOpcode::MovRegReg, vec![a0], Some(a), 0));
        blk.push(li(MachineOpcode::MovRegReg, vec![b0], Some(b), 0));
        blk.push(li(MachineOpcode::AddRegReg, vec![a, b], Some(a), 0));
        blk.push(li(MachineOpcode::MovRegReg, vec![b], Some(b), 0));
        blk.push(li(MachineOpcode::Ret, vec![a], None, 0));
        f.push_block(blk);
        f
    };
    let ra = allocate(&build()).expect("a");
    let rb = allocate(&build()).expect("b");
    assert_eq!(format!("{ra:?}"), format!("{rb:?}"));
}

/// CEP:WHAT: Regression 15: FPR phi values in a loop keep distinct lanes.
/// CEP:WHY: The wrap rule and lane lock apply per register class.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_15_fpr_phis_differ() {
    let mut f = LirFunction::new("fprloop");
    let x = fval(&mut f); // multi-def
    let x0 = fval(&mut f);
    let y = fval(&mut f); // multi-def
    let y0 = fval(&mut f);
    let z = fval(&mut f);
    let mut b0 = LirBlock::new(BlockId(0));
    b0.push(li(MachineOpcode::MovsdRegReg, vec![], Some(x0), 0));
    b0.push(li(MachineOpcode::MovsdRegReg, vec![], Some(y0), 0));
    b0.push(li(MachineOpcode::MovsdRegReg, vec![x0], Some(x), 0));
    b0.push(li(MachineOpcode::MovsdRegReg, vec![y0], Some(y), 0));
    b0.successors = vec![BlockId(1)];
    f.push_block(b0);
    let mut b1 = LirBlock::new(BlockId(1));
    b1.push(li(MachineOpcode::CvtSi2sd, vec![], Some(z), 0));
    b1.successors = vec![BlockId(2), BlockId(3)];
    f.push_block(b1);
    let mut b2 = LirBlock::new(BlockId(2));
    b2.push(li(MachineOpcode::MovsdRegReg, vec![x], Some(x), 0));
    b2.push(li(MachineOpcode::MovsdRegReg, vec![y], Some(y), 0));
    b2.successors = vec![BlockId(1)];
    f.push_block(b2);
    let mut b3 = LirBlock::new(BlockId(3));
    b3.push(li(MachineOpcode::Ret, vec![], None, 0));
    f.push_block(b3);
    let alloc = allocate(&f).expect("allocate");
    assert_ne!(alloc.register(x), alloc.register(y), "FPR phis differ");
}

/// CEP:WHAT: Regression 16: exhaustive pairwise interval feasibility on the loop shape
/// (every value pair, not just the phis).
/// CEP:WHY: End-to-end structural feasibility on the failing program shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: the interval table of regression 02.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_16_loop_pairwise_feasible() {
    let built = build_loop();
    let alloc = allocate(&built.f).expect("allocate");
    let defs = [1u32, 2, 3, 4, 5, 7, 9, 12, 13, 14];
    let uses = [19u32, 5, 4, 19, 19, 8, 10, 17, 14, 16];
    let extended = [
        true, false, false, true, true, false, false, false, false, false,
    ];
    let intervals = intervals_of(&defs, &uses, &extended, 19);
    let ids: Vec<ValueId> = (0..10).map(ValueId).collect();
    assert_lane_disjoint(&alloc, &ids, &intervals, "loop pairwise");
}

/// CEP:WHAT: Regression 17: allocation succeeds on two sequential loop shapes in one
/// function, and the MIR verifies after rewrite.
/// CEP:WHY: The repair must not leak state between solves or shapes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_17_two_loops_allocate() {
    let a = build_loop();
    let mut f = a.f;
    let b = build_loop();
    let value_shift = f.values.len() as u32;
    let block_shift = f.blocks.len() as u32;
    for block in b.f.blocks {
        let mut block = block;
        for instr in &mut block.instructions {
            for op in &mut instr.operands {
                op.0 += value_shift;
            }
            if let Some(r) = &mut instr.result {
                r.0 += value_shift;
            }
        }
        for s in &mut block.successors {
            s.0 += block_shift;
        }
        block.id.0 += block_shift;
        f.push_block(block);
    }
    for v in &b.f.values {
        let mut v = v.clone();
        v.uses.clear();
        f.values.push(v);
    }
    let alloc = allocate(&f).expect("allocate both loops");
    let mut mir = isel::select(&f).expect("isel");
    rewrite(&mut mir, &f, &alloc).expect("rewrite");
    mir.verify().expect("verified");
}

/// CEP:WHAT: Regression 18: the multi-def brute-force optimality corpus (the ticket
/// requirement): twelve seeded instances where some values carry several definitions;
/// the MCF cost equals the brute-force optimum under the mirrored interval rule.
/// CEP:WHY: STANDARDS section 8 RV8-137 acceptance sentence.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: single-block instances, no calls, no pins.
/// CEP:COST: test-only; brute force capped at 16^4 assignments.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_18_multidef_bruteforce_corpus() {
    let nregs = abi::ALLOCATABLE_GPRS.len();
    for seed in 0x9100u64..0x9100 + 12 {
        let mut rng = SplitMix64::seed_from_u64(seed);
        let n = 2 + (rng.next_u64() % 3) as usize;
        // Def and redefinition points are unique (one write per instruction); use
        // points may collide, as in the single-def corpus. The span bounds the unique
        // write points (at most 2n <= 8) with slack so the walk always terminates.
        let span = 13u64;
        let mut f = LirFunction::new("mdcorpus");
        let ids: Vec<ValueId> = (0..n).map(|_| val(&mut f)).collect();
        let mut defs = vec![0u32; n];
        let mut redefs = vec![None; n];
        let mut uses = vec![0u32; n];
        let mut used_points = std::collections::BTreeSet::new();
        for i in 0..n {
            let mut d = (rng.next_u64() % span) as u32 + 1;
            while !used_points.insert(d) {
                d = d % (span as u32) + 1;
            }
            defs[i] = d;
            if rng.next_u64().is_multiple_of(2) {
                let mut r = (rng.next_u64() % span) as u32 + 1;
                while !used_points.insert(r) {
                    r = r % (span as u32) + 1;
                }
                if r > d {
                    redefs[i] = Some(r);
                }
            }
            let u = (rng.next_u64() % span) as u32 + 1;
            uses[i] = u.max(defs[i] + 1);
        }
        let mut max_point = defs.iter().copied().max().expect("defs");
        for i in 0..n {
            max_point = max_point.max(uses[i]).max(redefs[i].unwrap_or(0));
        }
        let last_point = max_point + 1; // the ret instruction point
        let mut point_write: Vec<Option<(ValueId, bool)>> = vec![None; max_point as usize];
        let mut ord: Vec<usize> = (0..n).collect();
        ord.sort_by_key(|&i| defs[i]);
        for &i in &ord {
            point_write[(defs[i] - 1) as usize] = Some((ids[i], false));
            if let Some(r) = redefs[i] {
                point_write[(r - 1) as usize] = Some((ids[i], true));
            }
        }
        let mut point_uses: Vec<Vec<ValueId>> = vec![Vec::new(); max_point as usize];
        for (i, &v) in ids.iter().enumerate() {
            point_uses[(uses[i] - 1).min(max_point - 1) as usize].push(v);
        }
        let mut blk = LirBlock::new(BlockId(0));
        for p in 0..max_point as usize {
            let mut instr = li(MachineOpcode::MovRegImm32, vec![], None, p as i64);
            if let Some((v, is_redef)) = point_write[p] {
                instr.result = Some(v);
                if is_redef {
                    instr.opcode = MachineOpcode::MovRegReg;
                }
            }
            instr.operands = point_uses[p].clone();
            blk.push(instr);
        }
        blk.push(li(MachineOpcode::Ret, vec![], None, 0));
        f.push_block(blk);

        let alloc =
            allocate(&f).unwrap_or_else(|e| panic!("seed {seed}: allocate failed: {e}"));

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
            if multidef_feasible(&defs, &uses, &redefs, &assign, nregs, last_point) {
                let cost: i64 = assign
                    .iter()
                    .map(|a| match a {
                        Some(r) => {
                            let save = if abi::is_callee_saved(abi::ALLOCATABLE_GPRS[*r]) {
                                rv8_backend::regalloc::SAVE_RESTORE_COST
                            } else {
                                0
                            };
                            2 * rv8_backend::regalloc::MOVE_COST + save
                        }
                        None => rv8_backend::regalloc::SPILL_COST,
                    })
                    .sum();
                best = Some(best.map_or(cost, |b: i64| b.min(cost)));
            }
        }
        let brute = best.expect("spill-only is feasible");
        let mcf_cost: i64 = (0..n)
            .map(|i| match alloc.register(ids[i]) {
                Some(r) => {
                    let save =
                        if abi::is_callee_saved(abi::gpr_phys(r).expect("physical")) {
                            rv8_backend::regalloc::SAVE_RESTORE_COST
                        } else {
                            0
                        };
                    2 * rv8_backend::regalloc::MOVE_COST + save
                }
                None => rv8_backend::regalloc::SPILL_COST,
            })
            .sum();
        assert_eq!(mcf_cost, brute, "seed {seed}: mcf {mcf_cost} != brute {brute}");
    }
}

/// CEP:WHAT: Feasibility oracle for the multi-def corpus: half-open interval overlap
/// per register under the mirrored conservative rule.
/// CEP:WHY: The brute force must enforce the same capacity semantics as the network.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: single-block, no barriers.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn multidef_feasible(
    defs: &[u32],
    uses: &[u32],
    redefs: &[Option<u32>],
    assign: &[Option<usize>],
    nregs: usize,
    last_point: u32,
) -> bool {
    let n = assign.len();
    let interval = |i: usize| -> (u32, u32) {
        if redefs[i].is_some() {
            (defs[i], last_point)
        } else {
            (defs[i], uses[i].max(defs[i] + 1))
        }
    };
    for a in 0..n {
        if let Some(ra) = assign[a] {
            if ra >= nregs {
                return false;
            }
            let (lo_a, hi_a) = interval(a);
            #[allow(clippy::needless_range_loop)] // the index is the value id
            for b in 0..n {
                if a == b || assign[b] != Some(ra) {
                    continue;
                }
                let (lo_b, hi_b) = interval(b);
                if lo_a < hi_b && lo_b < hi_a {
                    return false;
                }
            }
        }
    }
    true
}

/// CEP:WHAT: Regression 19: the lane-lock feasibility corpus: forty seeded single-block
/// instances; the allocation never double-books a lane under the mirrored intervals,
/// and every instance allocates deterministically twice.
/// CEP:WHY: The structural property the cross-lane flow broke.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_19_lane_lock_corpus() {
    for seed in 0x9200u64..0x9200 + 40 {
        let mut rng = SplitMix64::seed_from_u64(seed);
        let n = 2 + (rng.next_u64() % 6) as usize;
        let span = 10u64;
        let mut f = LirFunction::new("lockcorpus");
        let ids: Vec<ValueId> = (0..n).map(|_| val(&mut f)).collect();
        let mut defs = Vec::new();
        let mut uses = Vec::new();
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
        let max_point = uses.iter().copied().max().expect("uses");
        let mut point_defs: Vec<Option<ValueId>> = vec![None; max_point as usize];
        let mut point_uses: Vec<Vec<ValueId>> = vec![Vec::new(); max_point as usize];
        let mut ord: Vec<usize> = (0..n).collect();
        ord.sort_by_key(|&i| (defs[i], ids[i]));
        for &i in &ord {
            point_defs[(defs[i] - 1) as usize] = Some(ids[i]);
        }
        for i in 0..n {
            if uses[i] > defs[i] {
                let idx = (uses[i] - 1).min(max_point - 1) as usize;
                point_uses[idx].push(ids[i]);
            }
        }
        let mut blk = LirBlock::new(BlockId(0));
        for p in 0..max_point as usize {
            let mut instr = li(MachineOpcode::MovRegImm32, vec![], point_defs[p], p as i64);
            instr.operands = point_uses[p].clone();
            blk.push(instr);
        }
        blk.push(li(MachineOpcode::Ret, vec![], None, 0));
        f.push_block(blk);
        let alloc =
            allocate(&f).unwrap_or_else(|e| panic!("seed {seed}: allocate failed: {e}"));
        let alloc2 = allocate(&f).unwrap_or_else(|e| panic!("seed {seed}: rerun: {e}"));
        assert_eq!(format!("{alloc:?}"), format!("{alloc2:?}"), "seed {seed}: deterministic");
        let intervals: Vec<(u32, u32)> = (0..n).map(|i| (defs[i], uses[i])).collect();
        assert_lane_disjoint(&alloc, &ids, &intervals, &format!("seed {seed}"));
    }
}

/// CEP:WHAT: Regression 20: the full backend on the loop shape is deterministic and
/// verifies (isel + allocate + rewrite twice, identical canonical hashes).
/// CEP:WHY: The 38.10/38.19 gate over the repaired pipeline.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_20_pipeline_deterministic_and_verified() {
    let run = || {
        let built = build_loop();
        let alloc = allocate(&built.f).expect("allocate");
        let mut mir = isel::select(&built.f).expect("isel");
        rewrite(&mut mir, &built.f, &alloc).expect("rewrite");
        mir.verify().expect("verified");
        mir.canonical_hash()
    };
    let h1 = run();
    let h2 = run();
    assert_eq!(h1, h2, "identical canonical hashes");
}

/// CEP:WHAT: Regression 21 (shared-core acceptance): the same LIR allocated through
/// two ISL target descriptions - the shipped x86-64 and a four-lane toy target -
/// proves one allocator core serves both: the toy plan spills under pressure where
/// the x86-64 plan does not, and both allocations are lane-disjoint under their own
/// lane counts.
/// CEP:WHY: User law 3 and 4: each target shares exactly one reggolloc and the ISL
/// carries the target descriptions.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert (test-only).
/// CEP:ASSUMES: the toy plan's four lanes make the eight-value shape infeasible
/// without spills.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn regression_137_21_allocate_with_two_targets_share_the_core() {
    let x86 = rv8_backend::regalloc::desc::x86_64_plan();
    let toy = rv8_backend::regalloc::desc::toy4_plan();
    let build = || {
        let mut f = LirFunction::new("share");
        let vs: Vec<ValueId> = (0..8).map(|_| val(&mut f)).collect();
        let mut b = LirBlock::new(BlockId(0));
        for (i, &v) in vs.iter().enumerate() {
            b.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), i as i64));
        }
        // Chain every value into the result so all eight stay live to the ret:
        // eight genuinely simultaneous intervals, eight on x86-64, four on toy4.
        let mut acc = vs[0];
        for &v in vs.iter().skip(1) {
            let s = val(&mut f);
            b.push(li(MachineOpcode::AddRegReg, vec![acc, v], Some(s), 0));
            acc = s;
        }
        b.push(li(MachineOpcode::Ret, vec![acc], None, 0));
        f.push_block(b);
        f
    };
    let ax = allocate_with(&x86, &build()).expect("x86 allocates");
    let at = allocate_with(&toy, &build()).expect("toy allocates");
    assert!(
        ax.spill_slots.iter().flatten().count() == 0,
        "eight short ranges fit fifteen lanes"
    );
    eprintln!("toy spills: {}", at.spill_slots.iter().flatten().count());
    assert!(
        at.spill_slots.iter().flatten().count() > 0,
        "eight simultaneous values cannot fit four lanes: spills required"
    );
}

