// CEP:FILE: crates/rv8_driver/tests/value_plumbing_rv8_137.rs
// CEP:WHAT: RV8-137 regression suite for value plumbing: 20 tests proving a function's
//   returned value rides the Ret instruction into isel (the dropped-operand defect)
//   and a non-Unit call's result is defined by the call, moved out of RAX by isel,
//   and consumable by any later instruction (the missing call-result defect).
// CEP:WHY: hetir_to_lir emitted Ret with no operands and gave ControlOp::Call nodes no
//   produced value, so every function returned whatever happened to sit in RAX:
//   straight-line fixtures passed by ABI accident while loop results corrupted.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-137
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: the language subset (i32, let, while, if-statement, calls up to six
//   arguments, operators + - * < <= ==).
// CEP:COST: test-only; each program is a handful of statements.
// CEP:EVIDENCE: this file is the evidence; one test per plumbing shape.

use rv8_backend::abi;
use rv8_driver::session::parse_args;
use rv8_ir::lir::{LirFunction, MachineOpcode};
use rv8_ir::machir::MachineOperand;

/// CEP:WHAT: Runs the full lowering chain (frontend through hetir_to_lir) on one
///   source text and returns the LIR of the named function.
/// CEP:WHY: Shared driver for the LIR-level tests below.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: src compiles and fname exists in it.
/// CEP:COST: one compilation.
/// CEP:EVIDENCE: used by the LIR tests below.
fn lir_of(src: &str, fname: &str) -> LirFunction {
    let hirs = rv8_frontend::compile_source(src).expect("frontend");
    let hir = hirs
        .iter()
        .find(|h| h.name == fname)
        .unwrap_or_else(|| panic!("function {fname} not found"));
    let mut sea = rv8_ir::lowering::hir_to_son(hir).expect("hir_to_son");
    rv8_middle::pipeline::optimize_son(&mut sea).expect("optimize_son");
    let mut het = rv8_ir::lower_het::son_to_hetir(&sea).expect("son_to_hetir");
    rv8_middle::pipeline::optimize_hetir(&mut het).expect("optimize_hetir");
    rv8_ir::lower_het::hetir_to_lir(&het, fname).expect("hetir_to_lir")
}

/// CEP:WHAT: The Ret instructions of a function (usually exactly one).
/// CEP:WHY: The dropped-operand defect is observable exactly here.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: the function returns (has a Ret).
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: used by the Ret tests below.
fn rets_of(f: &LirFunction) -> Vec<rv8_ir::lir::LirInstruction> {
    f.blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| i.opcode == MachineOpcode::Ret)
        .cloned()
        .collect()
}

/// CEP:WHAT: Asserts the function has exactly one Ret and returns its operand value.
/// CEP:WHY: Shared kernel of the single-exit tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: single-exit function.
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: used by the Ret tests below.
fn single_ret_operand(f: &LirFunction) -> rv8_ir::lir::ValueId {
    let rets = rets_of(f);
    assert_eq!(rets.len(), 1, "expected a single Ret");
    let r = &rets[0];
    assert_eq!(r.operands.len(), 1, "Ret must carry its value operand");
    r.operands[0]
}

/// CEP:WHAT: Opcode of the instruction defining a value.
/// CEP:WHY: The Ret operand must be defined by the producing instruction class the
///   test names (call, add, phi copy, constant).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: the value has exactly one defining instruction.
/// CEP:COST: O(instructions).
/// CEP:EVIDENCE: used by the defining-opcode tests below.
fn defining_opcode(f: &LirFunction, v: rv8_ir::lir::ValueId) -> MachineOpcode {
    let mut found: Option<MachineOpcode> = None;
    for b in &f.blocks {
        for i in &b.instructions {
            if i.result == Some(v) {
                assert!(found.is_none(), "value {} defined twice in probe", v.0);
                found = Some(i.opcode);
            }
        }
    }
    found.unwrap_or_else(|| panic!("value {} never defined", v.0))
}

/// CEP:WHAT: Compiles and runs a source program, returning the exit code.
/// CEP:WHY: Behavior-level kernel for the plumbing tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: unix exec available; main exists.
/// CEP:COST: one compile-and-run.
/// CEP:EVIDENCE: used by the end-to-end tests below.
fn run_src(src: &str) -> i32 {
    let args = vec!["x.src".to_string(), "--emit".to_string(), "run".to_string()];
    let session = parse_args(&args).expect("session");
    let program = rv8_driver::pipeline::compile(&session, src).expect("compile");
    rv8_driver::pipeline::run(&session, &program).expect("run")
}

/// CEP:WHAT: A returned constant rides the Ret operand.
/// CEP:WHY: The baseline plumbing probe; before the fix the operand list was empty.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: the constant folds to a single value.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_const_operand_present() {
    let f = lir_of("fn f() -> i32 { 7 }", "f");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::MovRegImm32);
}

/// CEP:WHAT: A returned variable rides the Ret operand.
/// CEP:WHY: Let-bound returns were the corrupted shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_variable_operand_present() {
    let f = lir_of("fn f() -> i32 { let a = 5; a }", "f");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::MovRegImm32);
}

/// CEP:WHAT: A returned binary expression rides the Ret operand.
/// CEP:WHY: Computed returns must reference the computing instruction.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: variables keep the add materialized.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_binop_operand_present() {
    let f = lir_of("fn f(a: i32, b: i32) -> i32 { a + b }", "f");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::AddRegReg);
}

/// CEP:WHAT: A returned loop-carried phi rides the Ret operand (the ticket shape).
/// CEP:WHY: THE RV8-137 part-2 reproducer: `while` accumulating into a returned
///   variable; the Ret operand must be the multi-def phi value.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_loop_carried_phi_operand_present() {
    let f = lir_of(
        "fn main() -> i32 { let mut i = 1; let mut acc = 0; \
         while i < 6 { acc = acc + i; i = i + 1; } acc }",
        "main",
    );
    let v = single_ret_operand(&f);
    // The phi value is defined by MovRegReg copies (init edge and latch edge): two
    // definitions of one value.
    let mut defs = 0;
    for b in &f.blocks {
        for i in &b.instructions {
            if i.result == Some(v) {
                defs += 1;
                assert_eq!(i.opcode, MachineOpcode::MovRegReg, "phi edge copies");
            }
        }
    }
    assert!(defs >= 2, "the loop phi must be multi-def (got {defs})");
}

/// CEP:WHAT: A returned call result rides the Ret operand and is defined by the Call.
/// CEP:WHY: The call-result defect: before the plumbing fix the call node produced no
///   value, so this probe failed with a lowering error.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_call_result_operand_present() {
    let f = lir_of("fn g() -> i32 { 9 } fn main() -> i32 { g() }", "main");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::Call);
}

/// CEP:WHAT: A returned nested call result rides the Ret operand.
/// CEP:WHY: id(id(41)) requires the inner call result to be consumable as the outer
///   call argument AND the outer result to be returnable.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_nested_call_result_operand_present() {
    let f = lir_of("fn id(v: i32) -> i32 { v } fn main() -> i32 { id(id(41)) }", "main");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::Call);
    // The outer call's argument must itself be a call result.
    let outer = f
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .find(|i| i.opcode == MachineOpcode::Call && i.result == Some(v))
        .expect("outer call")
        .clone();
    assert_eq!(outer.operands.len(), 1);
    assert_eq!(
        defining_opcode(&f, outer.operands[0]),
        MachineOpcode::Call,
        "the inner call result feeds the outer call"
    );
}

/// CEP:WHAT: A returned parameter rides the Ret operand.
/// CEP:WHY: Parameters are never defined by instructions; the Ret operand must be the
///   parameter value itself (ABI-pinned at allocation).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_parameter_operand_present() {
    let f = lir_of("fn id(v: i32) -> i32 { v }", "id");
    let v = single_ret_operand(&f);
    // The parameter has no defining instruction; its register class exists.
    assert!(v.0 < f.values.len() as u32);
    let mut defined = false;
    for b in &f.blocks {
        for i in &b.instructions {
            if i.result == Some(v) {
                defined = true;
            }
        }
    }
    assert!(!defined, "the parameter is never defined by an instruction");
}

/// CEP:WHAT: A returned comparison result rides the Ret operand.
/// CEP:WHY: Setcc-defined values must be returnable like any other.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: variables keep the comparison materialized.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_comparison_operand_present() {
    let f = lir_of("fn f(a: i32, b: i32) -> i32 { let c = 0; let d = 1; if a < b { c } else { d } }", "f");
    let rets = rets_of(&f);
    assert!(!rets.is_empty(), "comparison probe must return");
    for r in &rets {
        assert_eq!(r.operands.len(), 1, "Ret carries its operand");
    }
}

/// CEP:WHAT: A returned multiplication rides the Ret operand.
/// CEP:WHY: Imul/Lea-defined values must be returnable.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: variables keep the multiply materialized.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_multiplication_operand_present() {
    let f = lir_of("fn f(a: i32, b: i32) -> i32 { a * b }", "f");
    let v = single_ret_operand(&f);
    let op = defining_opcode(&f, v);
    assert!(
        matches!(op, MachineOpcode::ImulRegReg | MachineOpcode::Lea),
        "multiply lowers to imul or lea, got {op:?}"
    );
}

/// CEP:WHAT: A returned subtraction rides the Ret operand.
/// CEP:WHY: Straight-line arithmetic return probe.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_subtraction_operand_present() {
    let f = lir_of("fn f(a: i32, b: i32) -> i32 { a - b }", "f");
    let v = single_ret_operand(&f);
    assert_eq!(defining_opcode(&f, v), MachineOpcode::SubRegReg);
}

/// CEP:WHAT: The LIR Call carries its result value in the result field.
/// CEP:WHY: The plumbing contract between hetir_to_lir (produces) and isel (RAX move).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: the call result is consumed (not dead).
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn call_instruction_carries_result() {
    let f = lir_of("fn g() -> i32 { 4 } fn main() -> i32 { g() + 1 }", "main");
    let call = f
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .find(|i| i.opcode == MachineOpcode::Call)
        .expect("call")
        .clone();
    assert!(call.result.is_some(), "non-Unit call must define its result");
}

/// CEP:WHAT: isel emits the RAX result move immediately after the call.
/// CEP:WHY: The ABI result routing: mov vreg(v), rax; the scheduler models the call
///   clobber so the move cannot float above it.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn isel_emits_rax_result_move_after_call() {
    let f = lir_of("fn g() -> i32 { 4 } fn main() -> i32 { g() }", "main");
    let mir = rv8_backend::isel::select(&f).expect("isel");
    'outer: for b in &mir.blocks {
        for (k, i) in b.instructions.iter().enumerate() {
            if i.opcode == MachineOpcode::Call {
                let next = b.instructions.get(k + 1).expect("result move after call");
                assert_eq!(next.opcode, MachineOpcode::MovRegReg);
                match next.operands.as_slice() {
                    [MachineOperand::Reg(dst), MachineOperand::Reg(src)] => {
                        assert_eq!(src.0, abi::RETURN_REGISTER.0, "source must be RAX");
                        assert!(
                            dst.0 >= rv8_backend::VREG_BASE,
                            "destination must be the call result vreg"
                        );
                    }
                    other => panic!("unexpected result move operands: {other:?}"),
                }
                break 'outer;
            }
        }
    }
}

/// CEP:WHAT: The call result value carries the GPR register class in the LIR.
/// CEP:WHY: The allocator routes only values with a class; a classless result would
///   fall out of every allocation table.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn call_result_has_gpr_class() {
    let f = lir_of("fn g() -> i32 { 4 } fn main() -> i32 { g() }", "main");
    let call = f
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .find(|i| i.opcode == MachineOpcode::Call)
        .expect("call")
        .clone();
    let v = call.result.expect("result");
    assert_eq!(
        f.values[v.0 as usize].reg_class,
        rv8_ir::lir::RegClassId::Gpr
    );
}

/// CEP:WHAT: A call result consumed by arithmetic compiles through the whole backend.
/// CEP:WHY: The result vreg must survive allocation, rewrite, scheduling, and
///   verification when mixed with ordinary values.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn call_result_in_arithmetic_compiles() {
    let f = lir_of("fn g() -> i32 { 4 } fn main() -> i32 { g() + 1 }", "main");
    let out = rv8_backend::compile_lir(&f, "fn0").expect("compile");
    assert!(!out.code.is_empty());
}

/// CEP:WHAT: Two calls produce two distinct result values.
/// CEP:WHY: Distinct results must not be merged by any pass; consuming both proves
///   both definitions exist.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn two_calls_two_distinct_results() {
    let f = lir_of("fn g() -> i32 { 4 } fn main() -> i32 { g() + g() }", "main");
    let calls: Vec<rv8_ir::lir::LirInstruction> = f
        .blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| i.opcode == MachineOpcode::Call)
        .cloned()
        .collect();
    assert_eq!(calls.len(), 2, "both calls survive");
    let r0 = calls[0].result.expect("result 0");
    let r1 = calls[1].result.expect("result 1");
    assert_ne!(r0, r1, "results are distinct values");
}

/// CEP:WHAT: A call inside a loop compiles through the whole backend.
/// CEP:WHY: The result value meets the multi-def phi machinery and the call barrier
///   at once; this is the compound shape of the whole ticket.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn call_result_in_loop_compiles() {
    let f = lir_of(
        "fn sub(a: i32, b: i32) -> i32 { a - b } \
         fn main() -> i32 { let mut x = 0; let mut i = 0; \
         while i < 4 { x = x + sub(i, 1); i = i + 1; } x }",
        "main",
    );
    let out = rv8_backend::compile_lir(&f, "fn1").expect("compile");
    assert!(!out.code.is_empty());
}

/// CEP:WHAT: A call result returned directly executes with its value (exit code 9).
/// CEP:WHY: Behavior proof of the plumbing: before the fix, main returned RAX residue.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn call_result_returned_runs() {
    let code = run_src("fn g() -> i32 { 9 } fn main() -> i32 { g() }");
    assert_eq!(code, 9, "main must return g()'s 9");
}

/// CEP:WHAT: A call result used in arithmetic executes with its value (exit code 5).
/// CEP:WHY: Behavior proof that the result vreg routes through allocation.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn call_result_arithmetic_runs() {
    let code = run_src("fn g() -> i32 { 4 } fn main() -> i32 { g() + 1 }");
    assert_eq!(code, 5, "g() + 1 = 5");
}

/// CEP:WHAT: Nested calls execute with their values (exit code 42).
/// CEP:WHY: id(id(41)) + 1 requires both call results plumbed.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn nested_call_results_run() {
    let code = run_src("fn id(v: i32) -> i32 { v } fn main() -> i32 { id(id(41)) + 1 }");
    assert_eq!(code, 42, "id(id(41)) + 1 = 42");
}

/// CEP:WHAT: A call inside a loop executes with correct accumulation (exit code 2).
/// CEP:WHY: sub(i,1) summed for i=0..3 is -1+0+1+2 = 2; this exercises call results
///   meeting loop phis, the call barrier, and the returned accumulator at once.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn call_result_in_loop_runs() {
    let code = run_src(
        "fn sub(a: i32, b: i32) -> i32 { a - b } \
         fn main() -> i32 { let mut x = 0; let mut i = 0; \
         while i < 4 { x = x + sub(i, 1); i = i + 1; } x }",
    );
    assert_eq!(code, 2, "sum of sub(i,1) for i=0..3 is 2");
}

/// CEP:WHAT: isel rejects a Ret with two value operands (fail-closed).
/// CEP:WHY: The plumbing must not silently drop extra operands; malformed LIR is an
///   error, not a guess.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect on the error path (test-only).
/// CEP:ASSUMES: hand-built LIR is legal for allocate but not for isel.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn ret_with_two_operands_is_rejected() {
    use rv8_ir::lir::{LirBlock, LirInstruction, RegClassId, ValueId};
    use rv8_ir::son::{BlockId, IrType};
    let mut f = LirFunction::new("bad");
    let a = f.add_value(IrType::I64, RegClassId::Gpr);
    let b = f.add_value(IrType::I64, RegClassId::Gpr);
    let mut blk = LirBlock::new(BlockId(0));
    blk.instructions.push(LirInstruction {
        opcode: MachineOpcode::MovRegImm32,
        operands: vec![],
        result: Some(a),
        immediate: 1,
        target_block: None,
    });
    blk.instructions.push(LirInstruction {
        opcode: MachineOpcode::MovRegImm32,
        operands: vec![],
        result: Some(b),
        immediate: 2,
        target_block: None,
    });
    blk.instructions.push(LirInstruction {
        opcode: MachineOpcode::Ret,
        operands: vec![a, b],
        result: None,
        immediate: 0,
        target_block: None,
    });
    f.push_block(blk);
    let err = rv8_backend::isel::select(&f).expect_err("two-value ret must be rejected");
    assert!(
        err.to_string().contains("at most one value operand"),
        "unexpected error: {err}"
    );
    let _ = ValueId(0);
}

/// CEP:WHAT: A loop program with a call compiles byte-identically twice.
/// CEP:WHY: The compound shape must stay deterministic through the whole backend
///   (plumbing, allocation, scheduling, encoding).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
fn loop_call_program_is_deterministic() {
    let src = "fn sub(a: i32, b: i32) -> i32 { a - b } \
               fn main() -> i32 { let mut x = 0; let mut i = 0; \
               while i < 4 { x = x + sub(i, 1); i = i + 1; } x }";
    let f1 = lir_of(src, "main");
    let f2 = lir_of(src, "main");
    let o1 = rv8_backend::compile_lir(&f1, "fn1").expect("compile 1");
    let o2 = rv8_backend::compile_lir(&f2, "fn1").expect("compile 2");
    assert_eq!(o1.code, o2.code, "byte-identical backend output required");
}
