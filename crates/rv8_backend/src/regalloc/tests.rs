// CEP:FILE: crates/rv8_backend/src/regalloc/tests.rs
// CEP:WHAT: The allocator's inline test module: the single-def brute-force optimality
//   corpus, determinism, pressure, pre-coloring, caller-saved avoidance, and the
//   spill-path acceptance. The RV8-137 regression batch and the multi-def corpus live
//   in crates/rv8_backend/tests/.
// CEP:WHY: STANDARDS 5.1 acceptance evidence.
// CEP:CLASS: CC
// CEP:STATUS: complete
// CEP:FAILURE: panics via assert/expect (test-only).
// CEP:ASSUMES: corpus instances have no calls and no ABI pins unless stated.
// CEP:COST: test-only.
// CEP:EVIDENCE: this module.
use crate::abi;
use rv8_ir::lir::{MachineOpcode, ValueId};
use rv8_ir::machir::MachineOperand;
use MachineOperand::Reg;


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
                    // Immediate definitions spill with a direct store (no register).
                    MachineOpcode::StoreStackImm => {
                        stores += 1;
                        assert!(matches!(i.operands[0], MachineOperand::Mem(_)));
                        assert!(matches!(i.operands[1], MachineOperand::Imm(_)));
                    }
                    _ => {}
                }
            }
        }
        assert!(loads > 0 && stores > 0, "both spill directions must appear");
    }


// ---------------------------------------------------------------------------
// RV8-140 batch: linear-scan warm start and incremental/region re-solve over the
// ONE shared timeline network. The corpora arbitrate exactness (warm) and identity
// (incremental on unchanged input); feasibility and determinism are asserted
// everywhere.
// ---------------------------------------------------------------------------

/// CEP:WHAT: Builds a corpus-style function from defs/uses tables (one instruction
/// per program point, filler points carry no definition, Ret terminates).
/// CEP:WHY: Shared fixture for the warm/incremental batch; mirrors the corpus shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: defs[i] >= 1; uses[i] > defs[i].
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this batch.
fn build_corpus_fn(name: &str, defs: &[u32], uses: &[u32], n: usize) -> (rv8_ir::lir::LirFunction, Vec<ValueId>) {
    use rv8_ir::lir::LirBlock;
    use rv8_ir::son::BlockId;
    let mut f = LirFunction::new(name);
    let ids: Vec<ValueId> = (0..n).map(|_| val(&mut f)).collect();
    let max_point = uses.iter().copied().max().unwrap_or(1);
    let mut point_defs: Vec<Option<ValueId>> = vec![None; max_point as usize];
    let mut point_uses: Vec<Vec<ValueId>> = vec![Vec::new(); max_point as usize];
    let mut ord: Vec<usize> = (0..ids.len()).collect();
    ord.sort_by_key(|&i| (defs[i], ids[i]));
    for &i in &ord {
        point_defs[(defs[i] - 1) as usize] = Some(ids[i]);
    }
    for i in 0..ids.len() {
        point_uses[(uses[i] - 1) as usize].push(ids[i]);
    }
    let mut block = LirBlock::new(BlockId(0));
    for p in 0..max_point as usize {
        let mut instr = li(MachineOpcode::MovRegImm32, vec![], point_defs[p], p as i64);
        instr.operands = point_uses[p].clone();
        block.push(instr);
    }
    block.push(li(MachineOpcode::Ret, vec![], None, 0));
    f.push_block(block);
    (f, ids)
}

/// CEP:WHAT: Random corpus instance generator shared by the batch.
/// CEP:WHY: Deterministic instances from fixed seeds (SplitMix64), same shape as the
/// optimality corpus.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this batch.
fn random_instance(seed: u64) -> (Vec<u32>, Vec<u32>, usize) {
    use rv8_core::rng::SplitMix64;
    let mut rng = SplitMix64::seed_from_u64(seed);
    let n = 2 + (rng.next_u64() % 3) as usize;
    let span = 6u64;
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
    (defs, uses, n)
}

/// CEP:WHAT: The warm-started solve matches the brute-force optimum on the whole
/// single-def corpus (24 seeds).
/// CEP:WHY: Exactness acceptance for RV8-140 warm start; the corpus is the arbiter.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn warm_start_matches_bruteforce_optimum_corpus() {
    let nregs = abi::ALLOCATABLE_GPRS.len();
    let mut failures: Vec<String> = Vec::new();
    for seed in 0x9000u64..0x9000 + 24 {
        let (defs, uses, n) = random_instance(seed);
        let (f, ids) = build_corpus_fn("warm", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let alloc = allocate_warm(&plan, &f)
            .unwrap_or_else(|e| panic!("seed {seed}: warm allocate failed: {e}"));
        let assign: Vec<Option<usize>> = (0..n)
            .map(|i| alloc.register(ids[i]).map(|r| r.0 as usize))
            .collect();
        if !feasible(&defs, &uses, &assign, nregs) {
            failures.push(format!("seed {seed}: warm infeasible {assign:?}"));
            continue;
        }
        let mut best: Option<i64> = None;
        let total = (nregs + 1).pow(n as u32);
        for code in 0..total {
            let mut a = vec![None; n];
            let mut c = code;
            for x in a.iter_mut().take(n) {
                let choice = c % (nregs + 1);
                c /= nregs + 1;
                *x = if choice == nregs { None } else { Some(choice) };
            }
            if feasible(&defs, &uses, &a, nregs) {
                let cost = assignment_cost(&a);
                best = Some(best.map_or(cost, |b: i64| b.min(cost)));
            }
        }
        let brute = best.expect("spill-only assignment is always feasible");
        let warm_cost = assignment_cost(&assign);
        if warm_cost != brute {
            failures.push(format!(
                "seed {seed}: warm {warm_cost} != brute {brute} (defs {defs:?} uses {uses:?} got {assign:?})"
            ));
        }
    }
    assert!(failures.is_empty(), "warm optimality failures:\n{}", failures.join("\n"));
}

/// CEP:WHAT: Warm and cold paths produce identical-cost (and typically identical)
/// assignments on the corpus, and warm is deterministic across runs.
/// CEP:WHY: The warm start is an accelerator, not a different allocator.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn warm_start_agrees_with_cold_and_is_deterministic() {
    for seed in 0xA140u64..0xA140 + 20 {
        let (defs, uses, n) = random_instance(seed);
        let (f, ids) = build_corpus_fn("warmcold", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let cold = allocate(&f).unwrap();
        let warm1 = allocate_warm(&plan, &f).unwrap();
        let warm2 = allocate_warm(&plan, &f).unwrap();
        assert_eq!(format!("{warm1:?}"), format!("{warm2:?}"), "seed {seed}: warm deterministic");
        let cost = |a: &rv8_ir::lir::RegisterAllocation| -> i64 {
            let assign: Vec<Option<usize>> = (0..ids.len())
                .map(|i| a.register(ids[i]).map(|r| r.0 as usize))
                .collect();
            assignment_cost(&assign)
        };
        assert_eq!(cost(&cold), cost(&warm1), "seed {seed}: warm cost equals cold cost");
    }
}

/// CEP:WHAT: Under extreme pressure the warm path spills honestly and stays feasible
/// and deterministic.
/// CEP:WHY: Feasibility holds by construction; the test pins it under stress.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn warm_start_spills_under_pressure_and_verifies() {
    let nregs = abi::ALLOCATABLE_GPRS.len();
    for seed in 0xB140u64..0xB140 + 12 {
        let n = nregs * 2 + 2; // guaranteed over-subscription
        let defs: Vec<u32> = (1..=n as u32).collect();
        let uses: Vec<u32> = defs.iter().map(|d| d + n as u32).collect();
        let (f2, ids) = build_corpus_fn("pressure", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let alloc = allocate_warm(&plan, &f2).unwrap();
        let assign: Vec<Option<usize>> = (0..n)
            .map(|i| alloc.register(ids[i]).map(|r| r.0 as usize))
            .collect();
        let spilled = assign.iter().filter(|a| a.is_none()).count();
        assert!(spilled >= n - nregs, "seed {seed}: expected real spilling");
        assert!(feasible(&defs, &uses, &assign, nregs), "seed {seed}: infeasible under pressure");
        let alloc2 = allocate_warm(&plan, &f2).unwrap();
        assert_eq!(format!("{alloc:?}"), format!("{alloc2:?}"), "seed {seed}: deterministic");
    }
}

/// CEP:WHAT: The incremental re-solve is EXACTLY the cold allocation when nothing
/// changed (assignments and spill slots identical).
/// CEP:WHY: Identity on unchanged input is the correctness baseline (design.md 7.2).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn incremental_resolve_is_identity_when_unchanged() {
    for seed in 0xC140u64..0xC140 + 20 {
        let (defs, uses, n) = random_instance(seed);
        let (f, _ids) = build_corpus_fn("inc", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let cold = allocate(&f).unwrap();
        let inc = allocate_incremental(&plan, &f, &cold, &[]).unwrap();
        assert_eq!(
            format!("{cold:?}"),
            format!("{inc:?}"),
            "seed {seed}: incremental must be identity on unchanged input"
        );
    }
}

/// CEP:WHAT: After a change (half the values re-routed), the incremental re-solve
/// stays feasible, deterministic, and never costs more than the all-spill baseline.
/// CEP:WHY: The edit-compile contract: feasible fast answers, honest fallback.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn incremental_resolve_keeps_feasibility_after_change() {
    let nregs = abi::ALLOCATABLE_GPRS.len();
    for seed in 0xD140u64..0xD140 + 16 {
        let (defs, uses, n) = random_instance(seed);
        let (f, ids) = build_corpus_fn("incchg", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let cold = allocate(&f).unwrap();
        let changed: Vec<ValueId> = ids.iter().take(n / 2 + 1).copied().collect();
        let inc = allocate_incremental(&plan, &f, &cold, &changed).unwrap();
        let assign: Vec<Option<usize>> = (0..n)
            .map(|i| inc.register(ids[i]).map(|r| r.0 as usize))
            .collect();
        assert!(
            feasible(&defs, &uses, &assign, nregs),
            "seed {seed}: incremental infeasible {assign:?}"
        );
        let inc2 = allocate_incremental(&plan, &f, &cold, &changed).unwrap();
        assert_eq!(format!("{inc:?}"), format!("{inc2:?}"), "seed {seed}: deterministic");
        // Never worse than spilling everything.
        let all_spill: Vec<Option<usize>> = vec![None; n];
        assert!(assignment_cost(&assign) <= assignment_cost(&all_spill));
    }
}

/// CEP:WHAT: Incremental with EVERY value changed still reaches the cold optimum.
/// CEP:WHY: The changed set being the whole function degenerates to a full re-solve.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn incremental_matches_cold_when_everything_changed() {
    for seed in 0xE140u64..0xE140 + 12 {
        let (defs, uses, n) = random_instance(seed);
        let (f, ids) = build_corpus_fn("incall", &defs, &uses, n);
        let plan = desc::x86_64_plan();
        let cold = allocate(&f).unwrap();
        let prev = allocate_incremental(&plan, &f, &cold, &[]).unwrap();
        let inc = allocate_incremental(&plan, &f, &prev, &ids).unwrap();
        let cost = |a: &rv8_ir::lir::RegisterAllocation| -> i64 {
            let assign: Vec<Option<usize>> = (0..ids.len())
                .map(|i| a.register(ids[i]).map(|r| r.0 as usize))
                .collect();
            assignment_cost(&assign)
        };
        assert_eq!(cost(&cold), cost(&inc), "seed {seed}: all-changed equals cold cost");
    }
}

/// CEP:WHAT: A prev allocation from a DIFFERENT LIR shape (points shifted by an
/// inserted instruction) still produces a feasible, deterministic allocation via the
/// documented fallback.
/// CEP:WHY: Fail-closed fallback instead of a stale-lane miscompile.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn incremental_falls_back_gracefully_on_shifted_points() {
    let nregs = abi::ALLOCATABLE_GPRS.len();
    let (defs, uses, n) = random_instance(0xF140);
    let (f1, ids) = build_corpus_fn("shift1", &defs, &uses, n);
    let prev = allocate(&f1).unwrap();
    // LIR v2: one extra live value shifts every point after it.
    let mut defs2 = defs.clone();
    let mut uses2 = uses.clone();
    defs2.push(1);
    uses2.push(2);
    let (f2, ids2) = build_corpus_fn("shift2", &defs2, &uses2, n + 1);
    let plan = desc::x86_64_plan();
    let inc = allocate_incremental(&plan, &f2, &prev, &ids).unwrap();
    let assign: Vec<Option<usize>> = (0..n + 1)
        .map(|i| inc.register(ids2[i]).map(|r| r.0 as usize))
        .collect();
    assert!(feasible(&defs2, &uses2, &assign, nregs), "fallback must stay feasible");
    let inc2 = allocate_incremental(&plan, &f2, &prev, &ids).unwrap();
    assert_eq!(format!("{inc:?}"), format!("{inc2:?}"), "fallback deterministic");
}
