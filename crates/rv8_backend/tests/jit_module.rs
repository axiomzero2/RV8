// CEP:FILE: crates/rv8_backend/tests/jit_module.rs
// CEP:WHAT: RV8-141 JIT module acceptance (design.md "The Runtime Code Emitter" 3.4,
//   STANDARDS 5.13): whole modules of LIR functions are compiled through the AOT oracle,
//   emitted into one W^X CodePool with every call relocation resolved before
//   finalization (module-local slots and external function-table ordinals), and executed
//   in-process: forward, backward, chained, shared, and external call targets, argument
//   passing through the SysV registers, patch-time call-site redirection, layout
//   determinism, and fail-closed refusal of unknown/ambiguous/duplicate ordinals.
// CEP:WHY: STANDARDS 5.13 acceptance is "emitted code executes and returns the expected
//   result in-process"; jit_emitter.rs covers one leaf body, this suite covers what real
//   programs do - call each other - through the emitter's relocation resolver.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (integration test harness only).
// CEP:ASSUMES: linux x86-64 host (the shipped JIT target; the whole suite is cfg-gated
//   to it); emitted functions follow the SysV convention the backend encodes.
// CEP:COST: a handful of 4KB pages for the duration of each test.
// CEP:EVIDENCE: this file; CI runs it inside cargo test --workspace.
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rv8_backend::{compile_lir, resolve_relocs, JitEmitterError, JitModule, JitModuleConfig};
use rv8_ir::lir::{LirBlock, LirFunction, LirInstruction, MachineOpcode, RegClassId};
use rv8_ir::son::{BlockId, IrType};
use rv8_runtime::jitmem::{CodePool, JitError, PAGE_SIZE};

/// CEP:WHAT: The default module geometry: one 4KB page, up to four.
/// CEP:WHY: Test brevity; every test uses the same budget so failures are about layout.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn cfg() -> JitModuleConfig {
    JitModuleConfig { page_cap: PAGE_SIZE, max_pages: 4 }
}

/// CEP:WHAT: Registers one GPR value on a LIR function.
/// CEP:WHY: Test brevity, mirroring the lib.rs unit-test idiom.
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

/// CEP:WHAT: The LIR body of `fn f() -> i64 { imm }`.
/// CEP:WHY: The smallest callee: a constant in RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn const_fn(imm: i64) -> LirFunction {
    let mut f = LirFunction::new("callee");
    let v = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), imm));
    blk.push(li(MachineOpcode::Ret, vec![v], None, 0));
    f.push_block(blk);
    f
}

/// CEP:WHAT: The LIR body of a caller: `fn f(args...) -> i64 { call fn{ord}(args) }`,
///   routing the result (if any) through RAX into the return.
/// CEP:WHY: The smallest caller exercising the PcRel32 call relocation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at most six arguments (the SysV register set).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn caller(ord: i64, args: &[u32], with_result: bool) -> LirFunction {
    let mut f = LirFunction::new("caller");
    let r = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(
        MachineOpcode::Call,
        args.to_vec(),
        if with_result { Some(r) } else { None },
        ord,
    ));
    if with_result {
        blk.push(li(MachineOpcode::Ret, vec![r], None, 0));
    } else {
        let z = val(&mut f);
        blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(z), 0));
        blk.push(li(MachineOpcode::Ret, vec![z], None, 0));
    }
    f.push_block(blk);
    f
}

/// CEP:WHAT: The LIR body of `fn f(x) -> i64 { x + imm }` (one SysV register parameter).
/// CEP:WHY: Argument passing: the never-defined v0 arrives in RDI.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn add_to_param(imm: i64) -> LirFunction {
    let mut f = LirFunction::new("inc");
    let p = val(&mut f); // v0: never defined -> the first SysV parameter (RDI).
    let s = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::AddRegImm32, vec![p], Some(s), imm));
    blk.push(li(MachineOpcode::Ret, vec![s], None, 0));
    f.push_block(blk);
    f
}

/// CEP:WHAT: Calls the module's entry for `ord` and returns its result.
/// CEP:WHY: Shared act step; transmute is the documented execution idiom.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: the entry is a finalized SysV `() -> i64` body; the module outlives it.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn call_entry(m: &JitModule, ord: i64) -> i64 {
    // SAFETY: entry() hands out the finalized body of a `() -> i64` SysV function built
    // by this module; the module outlives the call.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(m.entry(ord).expect("entry")) };
    f()
}

/// CEP:WHAT: A forward call (slot 0 calls slot 1) executes and returns the callee's
///   constant.
/// CEP:WHY: The base relocation witness: S > P displacement resolved before finalize.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn forward_call_target_executes() {
    let fns = [(0, caller(1, &[], true)), (1, const_fn(5))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 5);
    assert_eq!(call_entry(&m, 1), 5);
}

/// CEP:WHAT: A backward call (slot 1 calls slot 0) executes: the resolver handles
///   negative displacements.
/// CEP:WHY: S < P is the sign branch of the S + A - P formula.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn backward_call_target_executes() {
    let fns = [(0, const_fn(3)), (1, caller(0, &[], true))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 1), 3);
}

/// CEP:WHAT: A three-deep call chain (f0 -> f1 -> f2) executes.
/// CEP:WHY: Transitive relocations: every body's call site is resolved, not just the
///   entry function's.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn chain_depth_three_executes() {
    let fns = [(0, caller(1, &[], true)), (1, caller(2, &[], true)), (2, const_fn(11))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 11);
    assert_eq!(call_entry(&m, 1), 11);
}

/// CEP:WHAT: Two callers share one callee: both call sites resolve to the same target
///   and both entries return the callee's value.
/// CEP:WHY: Multiple reloc sites per symbol is the common real-module shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn shared_callee_two_sites_executes() {
    let fns = [(0, caller(2, &[], true)), (1, caller(2, &[], true)), (2, const_fn(6))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 6);
    assert_eq!(call_entry(&m, 1), 6);
}

/// CEP:WHAT: An argument routed through the SysV register convention (RDI in, RAX out)
///   across JIT-compiled code: caller(4) -> callee(x) = x + 3 returns 7.
/// CEP:WHY: The call lowering's move window (isel 648-688) must survive regalloc,
///   scheduling, and the JIT bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn argument_passes_through_registers() {
    let mut caller_lir = LirFunction::new("caller");
    let a = val(&mut caller_lir);
    let r = val(&mut caller_lir);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(a), 4));
    blk.push(li(MachineOpcode::Call, vec![a], Some(r), 1));
    blk.push(li(MachineOpcode::Ret, vec![r], None, 0));
    caller_lir.push_block(blk);
    let fns = [(0, caller_lir), (1, add_to_param(3))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 7);
}

/// CEP:WHAT: An external call resolves through the caller-provided function table: the
///   module calls fn5, whose address is a finalized probe pool body, and observes 21.
/// CEP:WHY: The driver's function-table contract (jitmem SECURITY note): external
///   ordinals resolve to absolute addresses, never embedded in the AOT bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn external_call_through_function_table() {
    let mut probe = CodePool::new(PAGE_SIZE, 1).expect("probe pool");
    let slot = probe
        .emit(&rv8_runtime::jitmem::x86_mov_eax_ret(21))
        .expect("probe emit");
    probe.finalize_all().expect("probe rx");
    // SAFETY: finalized `mov eax, 21; ret` probe; the pool outlives the module's calls.
    let addr = probe.code_ptr(slot).expect("probe ptr") as usize;
    let fns = [(0, caller(5, &[], true))];
    let m = JitModule::build(&fns, &[(5, addr)], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 21);
}

/// CEP:WHAT: An external target can be another JIT module's entry: module A calls
///   module B's ordinal 0 through the function table.
/// CEP:WHY: Cross-module calls are the tiering seam (design 4): AOT or JIT code behind
///   the same function-table convention.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn external_call_into_second_jit_module() {
    let b = JitModule::build(&[(0, const_fn(33))], &[], cfg()).expect("module b");
    let addr = b.entry(0).expect("b entry") as usize;
    let a = JitModule::build(&[(7, caller(0, &[], true))], &[(0, addr)], cfg()).expect("module a");
    assert_eq!(call_entry(&a, 7), 33);
}

/// CEP:WHAT: A call to an ordinal that is neither local nor external fails closed with
///   UnknownSymbol, naming the symbol.
/// CEP:WHY: A silently-unresolved call would jump through garbage; refusing is the
///   module's CEP:FAILURE contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn unknown_ordinal_fails_closed() {
    let fns = [(0, caller(9, &[], true))];
    assert_eq!(
        JitModule::build(&fns, &[], cfg()).unwrap_err(),
        JitEmitterError::UnknownSymbol { symbol: "fn9".to_string() }
    );
}

/// CEP:WHAT: An ordinal that is both module-local and in the function table is
///   ambiguous and refused before any memory maps.
/// CEP:WHY: Which body should the call reach? The input contract forbids the question.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn local_and_external_collision_fails() {
    let fns = [(1, const_fn(1))];
    let externs = [(1, 0x1000usize)];
    assert_eq!(
        JitModule::build(&fns, &externs, cfg()).unwrap_err(),
        JitEmitterError::AmbiguousSymbol { symbol: "fn1".to_string() }
    );
}

/// CEP:WHAT: A duplicate module-local ordinal is refused.
/// CEP:WHY: Two bodies cannot own one function-table slot.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn duplicate_ordinal_fails() {
    let fns = [(0, const_fn(1)), (0, const_fn(2))];
    assert_eq!(
        JitModule::build(&fns, &[], cfg()).unwrap_err(),
        JitEmitterError::DuplicateOrdinal { ordinal: 0 }
    );
}

/// CEP:WHAT: An empty module is refused.
/// CEP:WHY: Building an executable nothing is an input bug, not a success.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn empty_module_fails() {
    assert_eq!(JitModule::build(&[], &[], cfg()).unwrap_err(), JitEmitterError::EmptyModule);
}

/// CEP:WHAT: Entry and slot lookups refuse ordinals the module does not implement.
/// CEP:WHY: The ordinal directory is the only sanctioned name -> code mapping.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn entry_unknown_ordinal_fails() {
    let m = JitModule::build(&[(0, const_fn(1))], &[], cfg()).expect("module");
    assert_eq!(
        m.entry(42).unwrap_err(),
        JitEmitterError::UnknownSymbol { symbol: "fn42".to_string() }
    );
    assert_eq!(
        m.slot(42).unwrap_err(),
        JitEmitterError::UnknownSymbol { symbol: "fn42".to_string() }
    );
}

/// CEP:WHAT: Two builds of the same module produce the same layout: identical slots
///   (page, offset, length), identical page counts.
/// CEP:WHY: The pool is deterministic first-fit and the compile is deterministic, so
///   the layout is content-seeded (the future tiering input).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn module_layout_is_deterministic() {
    let fns = [(0, caller(1, &[], true)), (1, const_fn(5)), (2, const_fn(6))];
    let a = JitModule::build(&fns, &[], cfg()).expect("build a");
    let b = JitModule::build(&fns, &[], cfg()).expect("build b");
    assert_eq!(a.function_count(), b.function_count());
    assert_eq!(a.page_count(), b.page_count());
    for ord in 0..3i64 {
        assert_eq!(a.slot(ord), b.slot(ord), "slot {ord} identical across builds");
    }
}

/// CEP:WHAT: The executed bytes are the linked AOT bytes: a leaf body is byte-equal to
///   compile_lir output, and the caller differs only at its relocation site, where the
///   four bytes equal the resolver's S + A - P displacement computed from the live
///   entry addresses.
/// CEP:WHY: The AOT pipeline is the JIT's correctness oracle (STANDARDS 5.13); the JIT
///   may only differ from the object path by resolved relocations.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn executed_bytes_equal_linked_aot_bytes() {
    let caller_lir = caller(1, &[], true);
    let fns = [(0, caller(1, &[], true)), (1, const_fn(5))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    let f0 = m.slot(0).expect("f0");
    let f1 = m.slot(1).expect("f1");
    let e0 = m.entry(0).expect("e0") as usize;
    let e1 = m.entry(1).expect("e1") as usize;
    // SAFETY: finalized module bodies; reading them while the module lives.
    let bytes0 = unsafe { core::slice::from_raw_parts(e0 as *const u8, f0.code_len) };
    let bytes1 = unsafe { core::slice::from_raw_parts(e1 as *const u8, f1.code_len) };
    let leaf = compile_lir(&const_fn(5), "fn1").expect("leaf compile");
    assert_eq!(bytes1, &leaf.code[..], "leaf body is the AOT bytes verbatim");
    let out = compile_lir(&caller_lir, "fn0").expect("caller compile");
    assert_eq!(f0.code_len, out.code.len());
    assert_eq!(out.relocs.len(), 1, "exactly the call relocation");
    let r = &out.relocs[0];
    let site = r.offset as usize;
    let mut expected = out.code.clone();
    let patches = resolve_relocs(
        out.code.len(),
        e0,
        &out.relocs,
        &|s: &str| Ok(if s == "fn1" { e1 } else { unreachable!() }),
    )
    .expect("resolves");
    expected[site..site + 4].copy_from_slice(&patches[0].bytes[..4]);
    assert_eq!(bytes0, &expected[..], "caller is the AOT bytes with the reloc applied");
    // And the applied displacement is exactly S + A - P against the live addresses.
    let disp = i32::from_le_bytes([bytes0[site], bytes0[site + 1], bytes0[site + 2], bytes0[site + 3]]);
    assert_eq!(disp as i64, e1 as i64 + r.addend - (e0 as i64 + r.offset as i64));
}

/// CEP:WHAT: A live call site is redirected through the atomic patch protocol: after
///   patching the four rel32 bytes of f0's call to point at fn2, the next call returns
///   fn2's value.
/// CEP:WHY: Design 3.6 patching must change observable execution of module code - the
///   seam the deopt/OSR tiers will use.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64; the rel32 site lies inside one atomic window (pinned by
///   the prologue layout: push rbp, mov rbp/rsp, sub rsp,imm8 puts the site at 9%8=1).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn call_site_redirect_via_patch() {
    let fns = [(0, caller(1, &[], true)), (1, const_fn(1)), (2, const_fn(42))];
    let mut m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(call_entry(&m, 0), 1);
    // Recompile the caller (deterministic bytes) to learn the relocation site.
    let out = compile_lir(&caller(1, &[], true), "fn0").expect("caller compile");
    let site = out.relocs[0].offset as usize;
    let e0 = m.entry(0).expect("e0") as usize;
    let e2 = m.entry(2).expect("e2") as usize;
    let disp = (e2 as i64 - 4 - (e0 as i64 + site as i64)) as i32;
    let bytes = disp.to_le_bytes();
    m.patch(0, site, &bytes).expect("atomic call redirect");
    assert_eq!(call_entry(&m, 0), 42, "the call now reaches fn2");
    // The callee slot itself is untouched: fn1 still returns its own constant.
    assert_eq!(call_entry(&m, 1), 1);
}

/// CEP:WHAT: The pool budget fails closed: more bodies than the page budget holds
///   refuses the whole module with Memory(OutOfPages).
/// CEP:WHY: Fail-closed memory discipline (CEP:FAILURE of jitmem) must hold through the
///   emitter assembly too.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn oversized_module_fails_closed() {
    let fns: Vec<(i64, LirFunction)> = (0..200).map(|i| (i, const_fn(i))).collect();
    let tiny = JitModuleConfig { page_cap: PAGE_SIZE, max_pages: 1 };
    assert_eq!(
        JitModule::build(&fns, &[], tiny).unwrap_err(),
        JitEmitterError::Memory(JitError::OutOfPages)
    );
}

/// CEP:WHAT: Module shape reporting: function_count and page_count describe what was
///   built (three functions here, at least one page).
/// CEP:WHY: Budget accounting and diagnostics need the truth, not a guess.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn module_shape_reports_functions_and_pages() {
    let fns = [(0, const_fn(1)), (1, const_fn(2)), (2, const_fn(3))];
    let m = JitModule::build(&fns, &[], cfg()).expect("module");
    assert_eq!(m.function_count(), 3);
    assert!(m.page_count() >= 1 && m.page_count() <= 4);
}
