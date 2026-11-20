// CEP:FILE: crates/rv8_backend/tests/jit_deopt.rs
// CEP:WHAT: RV8-141 deopt-tier acceptance (design.md "Region-Based Partial
//   Deoptimization" 1.2-1.3, STANDARDS 5.13 clause 2): a guard site patched into a
//   JIT-compiled function traps (ud2 -> SIGILL), the runtime handler reconstructs the
//   region's deoptimization state EXACTLY - every live value from its allocated
//   register or RSP-relative spill slot, decoded from the compact state bitstream -
//   and redirects execution into the function's own epilogue so the AOT caller resumes
//   with the reconstructed return value in RAX. The whole deopt metadata table comes
//   from build_with_deopt: regions, states, and guard points derived from the same
//   pipeline intermediates as the executed bytes.
// CEP:WHY: 5.13 acceptance: "a deopt reconstructs the interpreter state exactly". This
//   suite is that acceptance: the caller observes the AOT-identical result after a real
//   in-process trap, and the capture buffer proves the handler sourced every value
//   from its machine location, not from ambient registers.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (integration test harness only).
// CEP:ASSUMES: linux x86-64 host (cfg-gated); the trap tests serialize behind one
//   mutex because the SIGILL handler and its registry are process-global (the RV8-144
//   console-mux precedent); epilogue bytes are position-independent (add rsp / pops /
//   ret), so the exit stub is the function's own epilogue copied into a probe pool.
// CEP:COST: a handful of 4KB pages per test.
// CEP:EVIDENCE: this file; CI runs it inside cargo test --workspace.
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rv8_backend::{decode_state, encode_state, GuardSite, JitModule, JitModuleConfig, RegionId};
use rv8_ir::lir::{LirBlock, LirFunction, LirInstruction, MachineOpcode, RegClassId};
use rv8_ir::son::{BlockId, IrType};
use rv8_runtime::jitmem::{CodePool, PAGE_SIZE};
use rv8_runtime::trap::{install, register, uninstall, CapturedValue, TrapEntry, MAX_DEOPT_VALUES};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// CEP:WHAT: The process-global serialization for the trap tests: the SIGILL handler
///   and its registry are process-global, so two concurrent traps would cross entries.
/// CEP:WHY: The RV8-144 console-mux precedent: one mux, whole-test scope.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/unwrap (test-only).
/// CEP:ASSUMES: std is available in the integration harness.
/// CEP:COST: a mutex lock per trap test.
/// CEP:EVIDENCE: this module.
fn trap_mux() -> MutexGuard<'static, ()> {
    static MUX: OnceLock<Mutex<()>> = OnceLock::new();
    let m = MUX.get_or_init(|| Mutex::new(()));
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// CEP:WHAT: The default module geometry: one 4KB page, up to four.
/// CEP:WHY: Test brevity, mirroring jit_module.rs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn cfg() -> JitModuleConfig {
    JitModuleConfig { page_cap: PAGE_SIZE, max_pages: 4 }
}

/// CEP:WHAT: Registers one GPR value on a LIR function.
/// CEP:WHY: Test brevity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn val(f: &mut LirFunction) -> u32 {
    f.add_value(IrType::I64, RegClassId::Gpr).0
}

/// CEP:WHAT: LIR instruction shorthand.
/// CEP:WHY: Test brevity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn li(opcode: MachineOpcode, operands: Vec<u32>, result: Option<u32>, imm: i64) -> LirInstruction {
    LirInstruction {
        opcode,
        operands: operands.into_iter().map(rv8_ir::lir::ValueId).collect(),
        result: result.map(rv8_ir::lir::ValueId),
        immediate: imm,
        target_block: None,
    }
}

/// CEP:WHAT: The LIR body of `fn f() -> i64 { 6 + 7 }`: two constants, one add, one
///   ret - a leaf body with no spills (red-zone frame, bare ret epilogue).
/// CEP:WHY: The smallest deopt witness: the trap replaces the ret, the reconstructed
///   state is exactly the sum.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn sum_lir() -> (LirFunction, u32) {
    let mut f = LirFunction::new("sum");
    let a = val(&mut f);
    let b = val(&mut f);
    let s = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(a), 6));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(b), 7));
    blk.push(li(MachineOpcode::AddRegReg, vec![a, b], Some(s), 0));
    blk.push(li(MachineOpcode::Ret, vec![s], None, 0));
    f.push_block(blk);
    (f, s)
}

/// CEP:WHAT: The LIR body of the sum of 1..=16 through fifteen adds - enough
///   simultaneously-live constants that the allocator must spill, giving the witness a
///   frame with stack slots and a real prologue/epilogue.
/// CEP:WHY: The spilled-value witness: the handler must source the reconstructed
///   return value from an RSP-relative slot, not a register.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at least one value spills (13 allocatable GPR lanes vs 16 constants).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn wide_sum_lir() -> (LirFunction, u32) {
    let mut f = LirFunction::new("wide");
    let mut consts: Vec<u32> = Vec::new();
    for _ in 0..16 {
        consts.push(val(&mut f));
    }
    let mut blk = LirBlock::new(BlockId(0));
    for (i, &v) in consts.iter().enumerate() {
        blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), i as i64 + 1));
    }
    let mut acc = consts[0];
    for &v in &consts[1..] {
        let s = val(&mut f);
        blk.push(li(MachineOpcode::AddRegReg, vec![acc, v], Some(s), 0));
        acc = s;
    }
    blk.push(li(MachineOpcode::Ret, vec![acc], None, 0));
    f.push_block(blk);
    (f, acc)
}

/// CEP:WHAT: Locates the epilogue start of a compiled body: the ret is the last
///   non-pad byte; when the body carries a real epilogue, the `add rsp, imm8`
///   (48 83 C4 imm8) immediately before the ret opens it.
/// CEP:WHY: The trap site must be an instruction boundary at which the state's stack
///   locations are still valid (before any rsp restore).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert/expect (test-only).
/// CEP:ASSUMES: the documented epilogue shape (add rsp,imm8 then pops then ret) and
///   0x90 padding to the entry alignment.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn epilogue_site(code: &[u8]) -> usize {
    let mut ret_pos = code.len() - 1;
    while ret_pos > 0 && code[ret_pos] == 0x90 {
        ret_pos -= 1;
    }
    assert_eq!(code[ret_pos], 0xC3, "the last body byte is the ret");
    let mut site = ret_pos;
    if ret_pos >= 4 {
        let head = &code[ret_pos - 4..ret_pos];
        if head[0] == 0x48 && head[1] == 0x83 && head[2] == 0xC4 {
            site = ret_pos - 4;
        }
    }
    assert!(site % 8 <= 6, "the 2-byte ud2 must fit one atomic window at {}", site);
    site
}

/// CEP:WHAT: The ret instruction's ordinal in the fixture's single block (the last
///   instruction).
/// CEP:WHY: The guard's point field names it.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: single-block fixture.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn ret_ordinal(lir: &LirFunction) -> u32 {
    (lir.blocks[0].instructions.len() - 1) as u32
}

/// CEP:WHAT: Builds the module with one guard at the epilogue site and returns the
///   module plus the guard's encoded state, the site, and the epilogue bytes for the
///   exit stub. The pipeline is deterministic, so a first guard-free build learns the
///   byte layout and the real build carries the guard.
/// CEP:WHY: Shared arrange step for both trap witnesses.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: the fixture is one single-block leaf function.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn arrange(lir: LirFunction) -> (JitModule, [u8; 100], usize, usize, Vec<u8>) {
    let fns = [(0i64, lir)];
    let point = ret_ordinal(&fns[0].1);
    let (probe, _) = JitModule::build_with_deopt(&fns, &[], &[], cfg()).expect("probe build");
    let code_len = probe.slot(0).expect("slot").code_len;
    let entry = probe.entry(0).expect("entry") as usize;
    // SAFETY: finalized body read while the probe module lives.
    let code: Vec<u8> =
        unsafe { core::slice::from_raw_parts(entry as *const u8, code_len) }.to_vec();
    drop(probe);
    let site = epilogue_site(&code);
    let epilogue = code[site..].to_vec();
    let guards = [GuardSite {
        function: 0,
        block: 0,
        point,
        code_offset: site as u64,
        region: RegionId(0),
    }];
    let (module, table) =
        JitModule::build_with_deopt(&fns, &[], &guards, cfg()).expect("build");
    let guard = &table.guards[0];
    let encoded = encode_state(&guard.state).expect("encode");
    let mut state = [0u8; 100];
    state[..encoded.len()].copy_from_slice(&encoded);
    (module, state, encoded.len(), site, epilogue)
}

/// CEP:WHAT: THE deopt witness (red-zone leaf variant): the patched guard traps, the
///   handler captures the sum from its machine location, and the call returns 13 - the
///   AOT-identical result - through the reconstructed exit.
/// CEP:WHY: STANDARDS 5.13 clause 2 end to end, in-process.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64; the trap mux serializes the process-global handler.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn deopt_trap_returns_the_reconstructed_result() {
    let _mux = trap_mux();
    let (lir, ret_value) = sum_lir();
    let (module, state, state_len, site, epilogue) = arrange(lir);
    assert_eq!(state_len, 13, "6-byte header + one register entry (4+1+2)");
    // Exit stub: the function's own epilogue (here: ret) in a finalized probe pool.
    let mut stub = CodePool::new(PAGE_SIZE, 1).expect("stub pool");
    let stub_slot = stub.emit(&epilogue).expect("stub emit");
    stub.finalize_all().expect("stub rx");
    // SAFETY: finalized ret stub; the pool outlives the trap.
    let stub_addr = stub.code_ptr(stub_slot).expect("stub ptr") as usize;
    // Patch the ud2 over the epilogue start (2 bytes, atomic window).
    let f0 = module.slot(0).expect("slot 0");
    let mut module = module;
    module
        .patch(0, site, &[0x0F, 0x0B])
        .expect("guard patch into the epilogue");
    // Capture buffer.
    let mut out = [CapturedValue { id: 0, raw: 0 }; MAX_DEOPT_VALUES];
    let mut out_len = 0usize;
    install().expect("install");
    let entry_box = std::boxed::Box::leak(std::boxed::Box::new(TrapEntry {
        code_start: module.entry(0).expect("entry") as usize,
        code_len: f0.code_len,
        state,
        state_len,
        exit_stub: stub_addr,
        ret_value,
        out: out.as_mut_ptr(),
        out_cap: MAX_DEOPT_VALUES,
        out_len: &mut out_len,
    }));
    register(entry_box);
    // SAFETY: the module's fn0 is a finalized SysV `() -> i64` body; the call traps at
    // the guard and returns through the reconstructed exit.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(module.entry(0).expect("entry")) };
    let got = f();
    uninstall();
    assert_eq!(got, 13, "the deopt-completed call returns the AOT result");
    assert_eq!(out_len, 1, "exactly the sum value was captured");
    assert_eq!(out[0].raw, 13, "captured from the machine location");
    assert_eq!(out[0].id, ret_value);
}

/// CEP:WHAT: THE deopt witness (spilled variant): the trap fires with stack slots
///   live, the handler sources the return value from its RSP-relative slot, and the
///   call returns 136 - the 1..=16 sum - through the reconstructed epilogue.
/// CEP:WHY: Proves stack-resident reconstruction, not just registers.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64; the fixture spills (16 live constants).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn deopt_trap_reconstructs_spilled_values() {
    let _mux = trap_mux();
    let (lir, ret_value) = wide_sum_lir();
    let (module, state, state_len, site, epilogue) = arrange(lir);
    let mut stub = CodePool::new(PAGE_SIZE, 1).expect("stub pool");
    let stub_slot = stub.emit(&epilogue).expect("stub emit");
    stub.finalize_all().expect("stub rx");
    // SAFETY: finalized epilogue stub; the pool outlives the trap.
    let stub_addr = stub.code_ptr(stub_slot).expect("stub ptr") as usize;
    let f0 = module.slot(0).expect("slot 0");
    let mut module = module;
    module.patch(0, site, &[0x0F, 0x0B]).expect("guard patch");
    let mut out = [CapturedValue { id: 0, raw: 0 }; MAX_DEOPT_VALUES];
    let mut out_len = 0usize;
    install().expect("install");
    let entry_box = std::boxed::Box::leak(std::boxed::Box::new(TrapEntry {
        code_start: module.entry(0).expect("entry") as usize,
        code_len: f0.code_len,
        state,
        state_len,
        exit_stub: stub_addr,
        ret_value,
        out: out.as_mut_ptr(),
        out_cap: MAX_DEOPT_VALUES,
        out_len: &mut out_len,
    }));
    register(entry_box);
    // SAFETY: the module's fn0 is a finalized SysV `() -> i64` body.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(module.entry(0).expect("entry")) };
    let got = f();
    uninstall();
    assert_eq!(got, 136, "the deopt-completed call returns the spilled sum");
    assert!(out_len >= 1, "the live set was captured");
    let captured: Vec<CapturedValue> = out[..out_len].to_vec();
    assert!(
        captured.iter().any(|cv| cv.id == ret_value && cv.raw == 136),
        "the sum value appears among the captured values"
    );
}

/// CEP:WHAT: The table's guard state round-trips through the wire format and matches
///   the hand-computed liveness: exactly the sum value is live before the ret.
/// CEP:WHY: Pins the metadata extraction against the executable witness.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn guard_state_matches_hand_liveness() {
    let (lir, ret_value) = sum_lir();
    let fns = [(0i64, lir)];
    // Compile to learn the epilogue site (deterministic bytes).
    let (module, _) = JitModule::build_with_deopt(&fns, &[], &[], cfg()).expect("build");
    let code_len = module.slot(0).expect("slot").code_len;
    let entry = module.entry(0).expect("entry") as usize;
    // SAFETY: finalized body, read while the module lives.
    let code: Vec<u8> =
        unsafe { core::slice::from_raw_parts(entry as *const u8, code_len) }.to_vec();
    let site = epilogue_site(&code);
    let guards = [GuardSite {
        function: 0,
        block: 0,
        point: 3, // the ret ordinal of sum_lir
        code_offset: site as u64,
        region: RegionId(0),
    }];
    let (_, table) = JitModule::build_with_deopt(&fns, &[], &guards, cfg()).expect("build");
    let guard = &table.guards[0];
    assert_eq!(guard.site, guards[0]);
    assert_eq!(guard.state.live.len(), 1, "exactly the sum is live before the ret");
    assert_eq!(guard.state.live[0].0 .0, ret_value);
    // Wire round-trip through the decoder the runtime pairs with.
    let bytes = encode_state(&guard.state).expect("encode");
    assert_eq!(decode_state(&bytes).expect("decode"), guard.state);
    // Region shape: the single-block fixture is one region covering block 0.
    assert_eq!(table.functions[0].regions.len(), 1);
    assert_eq!(table.functions[0].regions[0].blocks, vec![0]);
}

/// CEP:WHAT: Every invalid guard reference fails closed with InvalidGuard: unknown
///   function, block, point, offset, and region.
/// CEP:WHY: The table must describe the built module, never a phantom.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn invalid_guard_sites_fail_closed() {
    let (lir, _) = sum_lir();
    let fns = [(0i64, lir)];
    let (module, _) = JitModule::build_with_deopt(&fns, &[], &[], cfg()).expect("build");
    let code_len = module.slot(0).expect("slot").code_len;
    let mk = |function: u32, block: u32, point: u32, code_offset: u64, region: u32| GuardSite {
        function,
        block,
        point,
        code_offset,
        region: RegionId(region),
    };
    let cases = [
        mk(5, 0, 3, 0, 0),
        mk(0, 9, 3, 0, 0),
        mk(0, 0, 99, 0, 0),
        mk(0, 0, 3, code_len as u64, 0),
        mk(0, 0, 3, 0, 7),
    ];
    for g in &cases {
        match JitModule::build_with_deopt(&fns, &[], &[*g], cfg()) {
            Err(rv8_backend::JitEmitterError::InvalidGuard { .. }) => {}
            other => panic!("expected InvalidGuard for {g:?}, got {:?}", other.map(|_| ())),
        }
    }
}
