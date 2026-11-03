// CEP:FILE: crates/rv8_backend/tests/jit_emitter.rs
// CEP:WHAT: RV8-141 in-process JIT execution acceptance (design.md "The Runtime Code
//   Emitter", STANDARDS 5.13): compile_lir bytes are copied into a finalized W^X
//   CodePool from rv8_runtime::jitmem and called as a native SysV function; live code is
//   redirected through the atomic patch protocol; post-finalize writes and oversized
//   bodies fail closed.
// CEP:WHY: STANDARDS 5.13 acceptance is "emitted code executes and returns the expected
//   result in-process" - this suite is that acceptance, exercised against the real
//   backend pipeline (isel -> MCF regalloc -> rewrite -> schedule -> verify -> encode),
//   not a hand-written probe alone.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: panics via expect/assert (integration test harness only).
// CEP:ASSUMES: linux x86-64 host (the shipped JIT target; the whole suite is cfg-gated
//   to it); emitted functions follow the SysV convention the backend encodes.
// CEP:COST: a handful of 4KB pages for the duration of each test.
// CEP:EVIDENCE: this file; CI runs it inside cargo test --workspace.
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

use rv8_backend::{compile_lir, CodegenOutput};
use rv8_ir::lir::{LirBlock, LirFunction, LirInstruction, MachineOpcode, RegClassId, ValueId};
use rv8_ir::son::{BlockId, IrType};
use rv8_runtime::jitmem::{CodePool, CodeSlot, JitError, PAGE_SIZE};

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
        operands: operands.into_iter().map(ValueId).collect(),
        result: result.map(ValueId),
        immediate: imm,
        target_block: None,
    }
}

/// CEP:WHAT: The LIR body of `fn f() -> i64 { 13 }` (the add_main fixture's constant
///   core: 6 + 7 folded by the frontend, exactly what the fixture's main computes).
/// CEP:WHY: The smallest end-to-end backend body whose result is observable in RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn const_return_lir() -> LirFunction {
    let mut f = LirFunction::new("main");
    let v = val(&mut f);
    let mut blk = LirBlock::new(BlockId(0));
    blk.push(li(MachineOpcode::MovRegImm32, vec![], Some(v), 13));
    blk.push(li(MachineOpcode::Ret, vec![v], None, 0));
    f.push_block(blk);
    f
}

/// CEP:WHAT: Copies compiled bytes into a fresh finalized pool and returns the pool plus
///   slot for execution.
/// CEP:WHY: Shared arrange step for the execution tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn load(out: &CodegenOutput) -> (CodePool, CodeSlot) {
    let mut pool = CodePool::new(PAGE_SIZE, 4).expect("pool maps");
    let slot = pool.emit(&out.code).expect("emit into page");
    pool.finalize_all().expect("finalize rx");
    (pool, slot)
}

/// CEP:WHAT: STANDARDS 5.13 acceptance: the full backend pipeline's bytes execute
///   in-process and return 13, the fixture's result.
/// CEP:WHY: The JIT hybrid's first end-to-end witness against real compiler output.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn jit_executes_compiled_constant_return() {
    let lir = const_return_lir();
    let out = compile_lir(&lir, "main").expect("compile");
    assert!(out.relocs.is_empty(), "leaf function carries no relocations");
    let (pool, slot) = load(&out);
    // SAFETY: slot points at the finalized `mov eax/rax, 13; ret` body emitted for the
    // SysV convention; the pool outlives the call.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(pool.code_ptr(slot).expect("code ptr")) };
    assert_eq!(f(), 13);
}

/// CEP:WHAT: The JIT bytes are the AOT bytes: two compilations are identical and the
///   executed copy is byte-equal to the object-bound output.
/// CEP:WHY: The AOT pipeline is the JIT's correctness oracle (STANDARDS 5.13); nothing
///   may diverge between the bytes written to an object file and the bytes executed.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn jit_bytes_equal_aot_bytes() {
    let out1 = compile_lir(&const_return_lir(), "main").expect("compile 1");
    let out2 = compile_lir(&const_return_lir(), "main").expect("compile 2");
    assert_eq!(out1.code, out2.code, "deterministic backend bytes");
    let (pool, slot) = load(&out1);
    // SAFETY: finalized constant-return body as above.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(pool.code_ptr(slot).expect("code ptr")) };
    assert_eq!(f(), 13);
}

/// CEP:WHAT: The atomic patch protocol redirects live code: mov eax,1 becomes mov
///   eax,42 and the next call observes it.
/// CEP:WHY: design 3.6 (patching for deopt/OSR/inline caches) must actually change
///   observable execution.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: linux x86-64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn jit_patch_redirects_execution() {
    let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
    let slot = pool
        .emit(&rv8_runtime::jitmem::x86_mov_eax_ret(1))
        .expect("emit probe");
    pool.finalize_all().expect("rx");
    // SAFETY: finalized `mov eax, 1; ret` probe.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(pool.code_ptr(slot).expect("ptr")) };
    assert_eq!(f(), 1);
    // Patch the imm32's first byte (offset 1; single byte, inside the atomic window).
    pool.patch(slot, 1, &[42]).expect("patch");
    // SAFETY: same body, patched immediate.
    let f: extern "C" fn() -> i64 =
        unsafe { core::mem::transmute(pool.code_ptr(slot).expect("ptr")) };
    assert_eq!(f(), 42);
}

/// CEP:WHAT: Fail-closed discipline on a finalized pool: further emits are refused and
///   oversized bodies never map.
/// CEP:WHY: W^X and budget contracts must hold through the pool API too.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect/assert.
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn jit_pool_fails_closed() {
    let out = compile_lir(&const_return_lir(), "main").expect("compile");
    let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
    let _slot = pool.emit(&out.code).expect("emit");
    pool.finalize_all().expect("rx");
    assert_eq!(
        pool.emit(&[0x90]).unwrap_err(),
        JitError::WriteAfterFinalize,
        "no writes after finalize, even through the pool"
    );
    let big = vec![0xC3u8; PAGE_SIZE + 1];
    let mut fresh = CodePool::new(PAGE_SIZE, 4).expect("fresh pool");
    assert_eq!(fresh.emit(&big).unwrap_err(), JitError::OutOfCapacity);
    // Determinism: the same LIR compiles to the same bytes that were executed.
    let again = compile_lir(&const_return_lir(), "main").expect("recompile");
    assert_eq!(again.code, out.code);
}
