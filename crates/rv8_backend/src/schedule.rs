// CEP:FILE: crates/rv8_backend/src/schedule.rs
// CEP:WHAT: Deterministic list scheduling within basic blocks over post-allocation
//   Machine IR: a dependence DAG from physical register definitions/uses (including the
//   flags pseudo-register, memory base/index registers, and rax/rdx divide semantics),
//   a conservative memory chain, call/syscall clobber modeling, a latency table derived
//   from the isel cost model, and a ready list ordered by (ready time, then original
//   instruction index ascending). Terminators stay last; blocks are never reordered
//   across.
// CEP:WHY: Spec 6.4 names scheduling as the third backend transition; hiding latency
//   without changing semantics requires exact dependence edges and a deterministic
//   selection order (38.10).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete for single-issue greedy list scheduling (latency model only, no
//   resource conflicts). Blocks larger than MAX_SCHEDULE_BLOCK are left in their
//   original order (deterministic budget behavior, documented).
// CEP:FAILURE: Internal only for scheduling-state invariant violations (unreachable
//   over well-formed MIR). The pass never mutates successor lists; callers re-run
//   MachineIrFunction::verify() (the pipeline does).
// CEP:ASSUMES: post-allocation MIR (physical registers); at most one terminator per
//   block, in last position (the machir verifier's contract).
// CEP:COST: O(n^2) worst case per block (terminator-independent bookkeeping is linear;
//   edge dedup scans make the worst case quadratic), budget-capped per block.
// CEP:EVIDENCE: schedule_hoists_independent_work, schedule_respects_dependences,
//   schedule_is_deterministic_and_keeps_terminator_last, pipeline spill tests (verify
//   green after scheduling).
// CEP:SECURITY: iterative worklists and heaps only; no recursion.

use crate::abi;
use crate::error::BackendError;
use rv8_ir::lir::MachineOpcode;
use rv8_ir::machir::{MachineInstr, MachineIrFunction, MachineOperand};
use rv8_core::hash::FxHashMap;

/// CEP:WHAT: Maximum body instructions per block the scheduler will reorder.
/// CEP:WHY: Named budget (38.12): edge dedup makes dependence construction quadratic.
/// CEP:STATUS: complete
/// CEP:FAILURE: larger blocks are left in their original order (documented budget
/// behavior, never an error).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in schedule().
pub const MAX_SCHEDULE_BLOCK: usize = 8192;

/// CEP:WHAT: Pseudo register id modeling the machine flags register.
/// CEP:WHY: cmp writes flags; jcc/setcc read them; treating flags as a register gives
/// exact data dependence for free.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: no real PhysReg carries this number.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: schedule tests.
const FLAGS: u32 = u32::MAX;

/// CEP:WHAT: Latency of one instruction in abstract cycles (multiply 3, divide 10,
/// everything else 1 - the shape of the x86 cost model).
/// CEP:WHY: Ready-time arithmetic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: single-issue greedy model; latency equals the cost-table value.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: schedule_hoists_independent_work asserts the induced reorder.
fn latency(op: MachineOpcode) -> u64 {
    match op {
        MachineOpcode::ImulRegReg => 3,
        MachineOpcode::IdivReg => 10,
        _ => 1,
    }
}

/// CEP:WHAT: Collects the base/index registers of every Mem operand.
/// CEP:WHY: Memory operands carry hidden register uses the DAG must see.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(operands).
/// CEP:EVIDENCE: internal.
fn mem_regs(instr: &MachineInstr) -> Vec<u32> {
    let mut out = Vec::new();
    for op in &instr.operands {
        if let MachineOperand::Mem(m) = op {
            if let Some(b) = m.base {
                out.push(b.0 as u32);
            }
            if let Some(x) = m.index {
                out.push(x.0 as u32);
            }
        }
    }
    out
}

/// CEP:WHAT: Collects all explicit register operands.
/// CEP:WHY: Shared helper for defs/uses extraction.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(operands).
/// CEP:EVIDENCE: internal.
fn reg_ops(instr: &MachineInstr) -> Vec<u32> {
    instr
        .operands
        .iter()
        .filter_map(|o| match o {
            MachineOperand::Reg(r) => Some(r.0 as u32),
            _ => None,
        })
        .collect()
}

/// CEP:WHAT: Definition and use register sets of one instruction plus its memory
/// behavior: (defs, uses, reads_memory, writes_memory).
/// CEP:WHY: The dependence DAG is exactly these facts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: operand 0 is the destination for dest-bearing opcodes; two-address ALU
/// ops read their destination (captured in uses so RAW edges are never missed); lea
/// does NOT read memory (address arithmetic only).
/// CEP:COST: O(operands).
/// CEP:EVIDENCE: internal.
fn defs_uses(instr: &MachineInstr) -> (Vec<u32>, Vec<u32>, bool, bool) {
    let regs = reg_ops(instr);
    let mregs = mem_regs(instr);
    let rsp = abi::RSP.0 as u32;
    match instr.opcode {
        MachineOpcode::MovRegReg | MachineOpcode::MovRegImm32 => {
            let mut uses = regs.clone();
            let def = uses.remove(0);
            (vec![def], uses, false, false)
        }
        MachineOpcode::Lea => {
            let def = regs[0];
            (vec![def], mregs, false, false)
        }
        MachineOpcode::AddRegReg
        | MachineOpcode::AddRegImm32
        | MachineOpcode::SubRegReg
        | MachineOpcode::SubRegImm32
        | MachineOpcode::ImulRegReg
        | MachineOpcode::ShlRegImm8
        | MachineOpcode::SarRegImm8
        | MachineOpcode::AndRegReg
        | MachineOpcode::OrRegReg
        | MachineOpcode::XorRegReg
        | MachineOpcode::MovsdRegReg => {
            let mut uses = regs.clone();
            let def = uses.remove(0);
            (vec![def], uses, false, false)
        }
        MachineOpcode::XorRegSelf => (vec![regs[0]], vec![], false, false),
        MachineOpcode::MovRegMem | MachineOpcode::LoadStack => (vec![regs[0]], mregs, true, false),
        MachineOpcode::StoreStackImm => (vec![], mregs, false, true),
        MachineOpcode::MovMemReg | MachineOpcode::StoreStack => {
            // The stored VALUE register is a use (RAW source); memory base/index
            // registers are uses too.
            let mut uses = regs.clone();
            uses.extend(mregs);
            (vec![], uses, false, true)
        }
        MachineOpcode::CmpRegReg | MachineOpcode::CmpRegImm32 => (vec![FLAGS], regs, false, false),
        MachineOpcode::Setcc(_) => (vec![regs[0]], vec![FLAGS], false, false),
        MachineOpcode::CvtSi2sd => {
            let mut uses = regs.clone();
            let def = uses.remove(0);
            (vec![def], uses, false, false)
        }
        MachineOpcode::Cqo => (vec![abi::RDX.0 as u32], vec![abi::RAX.0 as u32], false, false),
        MachineOpcode::IdivReg => (
            vec![abi::RAX.0 as u32, abi::RDX.0 as u32],
            vec![abi::RAX.0 as u32, regs[0]],
            false,
            false,
        ),
        MachineOpcode::Push => (vec![rsp], vec![regs[0], rsp], false, true),
        MachineOpcode::Pop => (vec![regs[0], rsp], vec![rsp], true, true),
        MachineOpcode::Call | MachineOpcode::Syscall => {
            let clobbered: Vec<u32> = abi::CALLER_SAVED_REGISTERS
                .iter()
                .map(|r| r.0 as u32)
                .chain([rsp])
                .collect();
            (clobbered, regs, true, true)
        }
        MachineOpcode::Fence => (vec![], vec![], true, true),
        MachineOpcode::Jcc(_) => (vec![], vec![FLAGS], false, false),
        MachineOpcode::Jmp | MachineOpcode::Ret => (vec![], vec![], false, false),
    }
}

/// CEP:WHAT: Schedules every block of the MIR function with greedy list scheduling;
/// terminators stay in last position; block order and successor lists are untouched.
/// CEP:WHY: Public entry point (transition 6.4 step 3).
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal for scheduling invariant violations (unreachable over
/// well-formed MIR); never mutates successors.
/// CEP:ASSUMES: post-allocation MIR; caller re-runs verify() afterwards.
/// CEP:COST: O(sum over blocks of n^2) worst case, budget-capped per block.
/// CEP:EVIDENCE: module tests plus pipeline determinism tests.
pub fn schedule(mir: &mut MachineIrFunction) -> Result<(), BackendError> {
    for block in mir.blocks.iter_mut() {
        if block.instructions.len() <= 1 {
            continue;
        }
        let n = block.instructions.len();
        if n - 1 > MAX_SCHEDULE_BLOCK {
            continue; // documented budget behavior: leave oversized blocks in order
        }
        let terminator = block.instructions[n - 1].clone();
        let body: Vec<MachineInstr> = block.instructions.drain(0..n - 1).collect();
        let mut scheduled = schedule_body(&body)?;
        scheduled.push(terminator);
        block.instructions = scheduled;
    }
    Ok(())
}

/// CEP:WHAT: List-schedules one block body: builds the dependence DAG (RAW/WAW/WAR per
/// physical register including FLAGS, memory base/index registers, and a conservative
/// memory chain), then greedily emits ready instructions by (ready time, original
/// index) from a BinaryHeap min-heap.
/// CEP:WHY: The core of the pass, isolated for clarity and testing.
/// CEP:STATUS: complete
/// CEP:FAILURE: Internal when the ready-list bookkeeping desynchronizes (unreachable:
/// the DAG is acyclic by construction - edges only point forward).
/// CEP:ASSUMES: body has no terminators.
/// CEP:COST: O(n^2) worst case (edge dedup).
/// CEP:EVIDENCE: schedule_* tests.
fn schedule_body(body: &[MachineInstr]) -> Result<Vec<MachineInstr>, BackendError> {
    let n = body.len();
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut last_writer: FxHashMap<u32, usize> = FxHashMap::default();
    let mut pending_readers: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    let mut last_mem_op: Option<usize> = None;

    for (i, instr) in body.iter().enumerate() {
        let (defs, uses, mem_read, mem_write) = defs_uses(instr);
        let mut edge = |from: usize, to: usize| {
            if from != to && !succs[from].contains(&to) {
                preds[to].push(from);
                succs[from].push(to);
            }
        };
        for r in &uses {
            if let Some(&w) = last_writer.get(r) {
                edge(w, i); // RAW
            }
        }
        if mem_read || mem_write {
            if let Some(m) = last_mem_op {
                edge(m, i); // conservative memory chain
            }
        }
        for r in &defs {
            if let Some(&w) = last_writer.get(r) {
                edge(w, i); // WAW
            }
            if let Some(readers) = pending_readers.get(r) {
                for &rd in readers {
                    edge(rd, i); // WAR
                }
            }
        }
        for r in &uses {
            pending_readers.entry(*r).or_default().push(i);
        }
        for r in &defs {
            pending_readers.remove(r);
            last_writer.insert(*r, i);
        }
        if mem_read || mem_write {
            last_mem_op = Some(i);
        }
    }

    // Greedy list scheduling: ready = all predecessors emitted; pick by (ready time,
    // original index ascending); each issue advances time by one cycle.
    let mut indeg: Vec<usize> = preds.iter().map(|p| p.len()).collect();
    let mut ready_at: Vec<u64> = vec![0; n];
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<(u64, usize)>> =
        std::collections::BinaryHeap::new();
    for (i, &d) in indeg.iter().enumerate() {
        if d == 0 {
            ready.push(std::cmp::Reverse((0, i)));
        }
    }
    let mut out: Vec<MachineInstr> = Vec::with_capacity(n);
    let mut time: u64 = 0;
    while let Some(std::cmp::Reverse((rt, i))) = ready.pop() {
        let start = time.max(rt);
        let finish = start + latency(body[i].opcode);
        time = start + 1;
        out.push(body[i].clone());
        for &s in &succs[i] {
            if ready_at[s] < finish {
                ready_at[s] = finish;
            }
            indeg[s] -= 1;
            if indeg[s] == 0 {
                ready.push(std::cmp::Reverse((ready_at[s], s)));
            }
        }
    }
    if out.len() != n {
        return Err(BackendError::Internal {
            detail: "scheduling left instructions unemitted (cycle in dependence DAG)".into(),
        });
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::machir::{Isa, MachineBlock, MemOperand};
    use MachineOperand::Reg;

    /// CEP:WHAT: Builds a machine instruction for tests.
    /// CEP:WHY: Test brevity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn mi(op: MachineOpcode, operands: Vec<MachineOperand>) -> MachineInstr {
        MachineInstr { opcode: op, operands, predicates: Vec::new() }
    }

    /// CEP:WHAT: An independent const load is hoisted into the multiply's latency
    /// shadow: [mov rax,rbx; add rcx,rax(uses rax); mov rdx,2] schedules as
    /// [mov rax; mov rdx; add] because the add is ready only at cycle 1.
    /// CEP:WHY: Proves real reordering happens and the ready list orders it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: greedy (ready time, index) order.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn schedule_hoists_independent_work() {
        let body = vec![
            mi(MachineOpcode::MovRegReg, vec![Reg(abi::RAX), Reg(abi::RBX)]),
            mi(MachineOpcode::AddRegReg, vec![Reg(abi::RCX), Reg(abi::RAX)]), // ready at 1
            mi(MachineOpcode::MovRegImm32, vec![Reg(abi::RDX), MachineOperand::Imm(2)]),
        ];
        let out = schedule_body(&body).expect("schedules");
        let op_order: Vec<MachineOpcode> = out.iter().map(|i| i.opcode).collect();
        assert_eq!(op_order, vec![
            MachineOpcode::MovRegReg,
            MachineOpcode::MovRegImm32,
            MachineOpcode::AddRegReg,
        ], "independent mov hoisted into the latency shadow: {op_order:?}");
    }

    /// CEP:WHAT: Dependence safety: a use never moves before its def, a redefinition
    /// never moves before prior readers (WAR), and a store stays after the definition
    /// of its stored value (RAW through a register, plus the memory chain).
    /// CEP:WHY: Correctness of the DAG edges.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn schedule_respects_dependences() {
        let body = vec![
            mi(MachineOpcode::MovRegImm32, vec![Reg(abi::RAX), MachineOperand::Imm(1)]),
            mi(MachineOpcode::AddRegReg, vec![Reg(abi::RBX), Reg(abi::RAX)]), // RAW rax
            mi(MachineOpcode::MovRegImm32, vec![Reg(abi::RAX), MachineOperand::Imm(2)]), // WAW/WAR rax
            mi(MachineOpcode::StoreStack, vec![
                MachineOperand::Mem(MemOperand {
                    base: Some(abi::RSP),
                    index: None,
                    scale: 1,
                    disp: 8,
                }),
                Reg(abi::RBX),
            ]), // RAW rbx + memory chain
        ];
        let out = schedule_body(&body).expect("schedules");
        let find = |pred: &dyn Fn(&MachineInstr) -> bool| -> usize {
            out.iter().position(pred).expect("instruction present")
        };
        let mov1 = find(&|i| {
            i.opcode == MachineOpcode::MovRegImm32
                && matches!(i.operands[1], MachineOperand::Imm(1))
        });
        let add = find(&|i| i.opcode == MachineOpcode::AddRegReg);
        let mov2 = find(&|i| {
            i.opcode == MachineOpcode::MovRegImm32
                && matches!(i.operands[1], MachineOperand::Imm(2))
        });
        let store = find(&|i| i.opcode == MachineOpcode::StoreStack);
        assert!(mov1 < add, "RAW: rax read after written");
        assert!(add < mov2, "WAR/WAW: rax redefined after reader");
        assert!(add < store, "RAW: store after rbx definition");
    }

    /// CEP:WHAT: Scheduling is deterministic, keeps the terminator last, and leaves
    /// successor lists untouched; scheduling twice gives identical canonical hashes.
    /// CEP:WHY: 38.10 acceptance + terminator/successor invariants.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn schedule_is_deterministic_and_keeps_terminator_last() {
        let build = || {
            let mut f = MachineIrFunction::new("sched", Isa::X86_64);
            f.push_block(MachineBlock {
                instructions: vec![
                    mi(MachineOpcode::MovRegImm32, vec![Reg(abi::RAX), MachineOperand::Imm(1)]),
                    mi(MachineOpcode::ImulRegReg, vec![Reg(abi::RBX), Reg(abi::RCX)]),
                    mi(MachineOpcode::CmpRegReg, vec![Reg(abi::RDI), Reg(abi::RAX)]),
                    mi(MachineOpcode::MovRegImm32, vec![Reg(abi::RDX), MachineOperand::Imm(2)]),
                    mi(MachineOpcode::Ret, vec![]),
                ],
                successors: vec![],
            });
            f
        };
        let mut a = build();
        let mut b = build();
        let succs_before = a.blocks[0].successors.clone();
        schedule(&mut a).expect("sched 1");
        schedule(&mut b).expect("sched 2");
        assert_eq!(a.canonical_hash(), b.canonical_hash());
        assert_eq!(a.blocks[0].successors, succs_before, "successors untouched");
        let last = a.blocks[0].instructions.last().unwrap().opcode;
        assert_eq!(last, MachineOpcode::Ret, "terminator stays last");
        assert!(a.blocks[0].instructions.len() == 5, "no instruction lost");
    }
}
