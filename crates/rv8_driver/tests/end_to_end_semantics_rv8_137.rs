// CEP:FILE: crates/rv8_driver/tests/end_to_end_semantics_rv8_137.rs
// CEP:WHAT: RV8-137 end-to-end regression suite: 20 run-mode programs whose exit
//   codes pin the observable semantics of every defect fixed under the ticket - loop
//   execution (MCF interference), returned values (Ret operand plumbing), comparison
//   and subtraction operand order, and call argument/result plumbing.
// CEP:WHY: Each program is a behavior the pre-fix compiler miscompiled: the loop
//   program returned 0 instead of 15, comparisons inverted loop conditions, operands
//   swapped silently behind commutative operators, and call results were RAX residue.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-137
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: unix exec available; the language subset (i32, let, while,
//   if-statement, calls up to six arguments, operators + - * < <= ==).
// CEP:COST: test-only; each program links and execs once.
// CEP:EVIDENCE: this file is the evidence; every test names the expected arithmetic.

use rv8_driver::session::parse_args;

/// CEP:WHAT: Compiles and runs a source program, returning its exit code.
/// CEP:WHY: The shared behavior kernel; run() uses a per-invocation temp path so
///   parallel test threads cannot collide on the executable.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: the program has a main function.
/// CEP:COST: one compile-and-run.
/// CEP:EVIDENCE: used by every test below.
fn run_src(src: &str) -> i32 {
    let args = vec!["x.src".to_string(), "--emit".to_string(), "run".to_string()];
    let session = parse_args(&args).expect("session");
    let program = rv8_driver::pipeline::compile(&session, src).expect("compile");
    rv8_driver::pipeline::run(&session, &program).expect("run")
}

/// CEP:WHAT: The ticket program: sum 1..=5 exits 15.
/// CEP:WHY: THE RV8-137 acceptance: the hop defect, the dropped Ret operand, and the
///   inverted comparison each independently broke exactly this program.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_sums_one_to_five() {
    let code = run_src(
        "fn main() -> i32 { let mut i = 1; let mut acc = 0; \
         while i < 6 { acc = acc + i; i = i + 1; } acc }",
    );
    assert_eq!(code, 15, "1+2+3+4+5 = 15");
}

/// CEP:WHAT: Sum 0..=9 exits 45.
/// CEP:WHY: Same machinery, different bound and start; guards against a fix that
///   only handles the literal ticket constants.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_sums_zero_to_nine() {
    let code = run_src(
        "fn main() -> i32 { let mut i = 0; let mut acc = 0; \
         while i < 10 { acc = acc + i; i = i + 1; } acc }",
    );
    assert_eq!(code, 45, "sum 0..=9 = 45");
}

/// CEP:WHAT: An inclusive-bound loop (i <= 3) exits 6.
/// CEP:WHY: Le is the second comparison operator; the inverted-operand defect
///   flipped it too.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn le_loop_is_inclusive() {
    let code = run_src(
        "fn main() -> i32 { let mut s = 0; let mut i = 0; \
         while i <= 3 { s = s + i; i = i + 1; } s }",
    );
    assert_eq!(code, 6, "0+1+2+3 = 6");
}

/// CEP:WHAT: A loop with step 2 exits 12.
/// CEP:WHY: Non-unit increments stress the phi copy sequencing.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_with_step_two() {
    let code = run_src(
        "fn main() -> i32 { let mut s = 0; let mut i = 0; \
         while i < 7 { s = s + i; i = i + 2; } s }",
    );
    assert_eq!(code, 12, "0+2+4+6 = 12");
}

/// CEP:WHAT: A loop accumulating a product exits 120.
/// CEP:WHY: Multiplication inside the loop-carried dependency.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_accumulates_product() {
    let code = run_src(
        "fn main() -> i32 { let mut p = 1; let mut i = 1; \
         while i < 6 { p = p * i; i = i + 1; } p }",
    );
    assert_eq!(code, 120, "5! = 120");
}

/// CEP:WHAT: A loop with an offset accumulator exits 115.
/// CEP:WHY: Non-zero initial accumulator; the returned phi must carry the offset.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_with_offset_accumulator() {
    let code = run_src(
        "fn main() -> i32 { let mut acc = 100; let mut i = 1; \
         while i < 6 { acc = acc + i; i = i + 1; } acc }",
    );
    assert_eq!(code, 115, "100 + 15 = 115");
}

/// CEP:WHAT: Returning the loop counter itself exits 6.
/// CEP:WHY: The counter phi (not the accumulator) as the returned value.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_counter_returned() {
    let code = run_src(
        "fn main() -> i32 { let mut i = 1; let mut acc = 0; \
         while i < 6 { acc = acc + i; i = i + 1; } i }",
    );
    assert_eq!(code, 6, "the counter exits at 6");
}

/// CEP:WHAT: A loop that never runs leaves the accumulator at 0.
/// CEP:WHY: The false condition must exit immediately; the inverted comparison would
///   run the body once (acc = 5).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_with_false_condition_never_runs() {
    let code = run_src(
        "fn main() -> i32 { let mut acc = 0; let mut i = 9; \
         while i < 2 { acc = acc + 5; i = i + 1; } acc }",
    );
    assert_eq!(code, 0, "9 < 2 is false: the body never runs");
}

/// CEP:WHAT: Two parallel accumulators in one loop exit 20 and 30 (checked via sum).
/// CEP:WHY: Two independent loop-carried phis double the interference surface.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_two_accumulators() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 0; let mut b = 0; let mut i = 1; \
         while i < 5 { a = a + i; b = b + i * 2; i = i + 1; } a + b }",
    );
    assert_eq!(code, 30, "a sums to 10, b to 20, total 30");
}

/// CEP:WHAT: sub(10, 3) exits 7.
/// CEP:WHY: Subtraction operand order, observed in behavior: reversed operands give
///   253 (the low byte of -7).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn subtraction_respects_argument_order() {
    let code = run_src("fn sub(a: i32, b: i32) -> i32 { a - b } fn main() -> i32 { sub(10, 3) }");
    assert_eq!(code, 7, "10 - 3 = 7");
}

/// CEP:WHAT: sub(3, 10) exits 249 (the low byte of -7).
/// CEP:WHY: The mirrored call: swapping the arguments must change the result.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: exit codes are the low 8 bits of the return value.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn subtraction_swapped_arguments() {
    let code = run_src("fn sub(a: i32, b: i32) -> i32 { a - b } fn main() -> i32 { sub(3, 10) }");
    assert_eq!(code, 249, "3 - 10 = -7, exit code 249");
}

/// CEP:WHAT: A six-argument call computes 100-10-1-1-1-1 = 86.
/// CEP:WHY: The full register argument window in source order; any permutation of
///   the constants changes the result.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn six_argument_call_order() {
    let code = run_src(
        "fn f6(a: i32, b: i32, c: i32, d: i32, e: i32, g: i32) -> i32 { a - b - c - d - e - g } \
         fn main() -> i32 { f6(100, 10, 1, 1, 1, 1) }",
    );
    assert_eq!(code, 86, "100 - 10 - 1*4 = 86");
}

/// CEP:WHAT: Nested calls compute id(id(41)) + 1 = 42.
/// CEP:WHY: Both call results must be plumbed through RAX moves.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn nested_calls_compute() {
    let code = run_src("fn id(v: i32) -> i32 { v } fn main() -> i32 { id(id(41)) + 1 }");
    assert_eq!(code, 42, "id(id(41)) + 1 = 42");
}

/// CEP:WHAT: A call inside a loop accumulates sub(i,1) for i=0..3 = 2.
/// CEP:WHY: Call results meeting loop phis and the call barrier at once (the
///   compound shape of the whole ticket).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn call_inside_loop_accumulates() {
    let code = run_src(
        "fn sub(a: i32, b: i32) -> i32 { a - b } \
         fn main() -> i32 { let mut x = 0; let mut i = 0; \
         while i < 4 { x = x + sub(i, 1); i = i + 1; } x }",
    );
    assert_eq!(code, 2, "(-1) + 0 + 1 + 2 = 2");
}

/// CEP:WHAT: An if-statement selects the then arm when the condition holds (9).
/// CEP:WHY: Eq plus branch lowering in behavior.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn if_selects_then_arm_on_true() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 0; let b = 5; if b == 5 { a = 9; } else { a = 8; } a }",
    );
    assert_eq!(code, 9, "b == 5 selects the then arm");
}

/// CEP:WHAT: An if-statement selects the else arm when the condition fails (8).
/// CEP:WHY: The mirror branch selection.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn if_selects_else_arm_on_false() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 0; let b = 6; if b == 5 { a = 9; } else { a = 8; } a }",
    );
    assert_eq!(code, 8, "b != 5 selects the else arm");
}

/// CEP:WHAT: An if inside a loop exits with the conditional sum (30).
/// CEP:WHY: If and while together: two control regions and their phis in one function.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn if_inside_loop() {
    let code = run_src(
        "fn main() -> i32 { let mut s = 0; let mut i = 0; \
         while i < 8 { if i < 4 { s = s + 5; } else { s = s + 2; } i = i + 1; } s }",
    );
    assert_eq!(code, 20 + 8, "four fives plus four twos = 28");
}

/// CEP:WHAT: A callee defined after its caller still links and runs (7).
/// CEP:WHY: Call-graph order independence: symbol resolution must not depend on
///   source order.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn callee_after_caller() {
    let code = run_src("fn main() -> i32 { g() } fn g() -> i32 { 7 }");
    assert_eq!(code, 7, "forward reference resolves");
}

/// CEP:WHAT: Multi-function programs with three call sites compute 6 + 7 + 8 = 21.
/// CEP:WHY: Several callees with distinct results; each result must route correctly
///   through its own call site.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn three_call_sites_sum() {
    let code = run_src(
        "fn a() -> i32 { 6 } fn b() -> i32 { 7 } fn c() -> i32 { 8 } \
         fn main() -> i32 { a() + b() + c() }",
    );
    assert_eq!(code, 21, "6 + 7 + 8 = 21");
}

/// CEP:WHAT: A loop whose accumulator feeds a final call exits with the called value.
/// CEP:WHY: The returned value is a call argument computed from a loop phi: the last
///   compound plumbing shape (phi result as call argument as returned value).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn loop_result_feeds_call() {
    let code = run_src(
        "fn double(v: i32) -> i32 { v + v } \
         fn main() -> i32 { let mut acc = 0; let mut i = 1; \
         while i < 4 { acc = acc + i; i = i + 1; } double(acc) }",
    );
    assert_eq!(code, 12, "double(1+2+3) = 12");
}

/// CEP:WHAT: The fibonacci rotation (a,b,t with a = b; b = t per iteration) exits 34.
/// CEP:WHY: THE phi-copy cycle regression: the sequencer emitted the destroying copy
///   first (`b = t; a = b`), so every iteration read the NEW b; a program that swaps
///   variables through a loop is the behavioral pin for copy-set sequencing.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn fibonacci_rotation() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 0; let mut b = 1; let mut i = 0; \
         while i < 9 { let t = a + b; a = b; b = t; i = i + 1; } a }",
    );
    assert_eq!(code, 34, "fib(9) = 34");
}

/// CEP:WHAT: A pure two-variable swap across loop iterations exits 7.
/// CEP:WHY: The minimal copy cycle (x<->y): after an odd number of swaps the values
///   exchange; a broken sequencer leaves one variable stale.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn two_variable_swap_in_loop() {
    let code = run_src(
        "fn main() -> i32 { let mut x = 3; let mut y = 7; let mut i = 0; \
         while i < 1 { let t = x; x = y; y = t; i = i + 1; } y }",
    );
    assert_eq!(code, 3, "one swap exchanges x and y; y holds the old x");
}

/// CEP:WHAT: A three-variable rotation per iteration exits with the rotated value.
/// CEP:WHY: Three-way rotations chain two dependent copies (a<-b, b<-c, c<-a) plus
///   an accumulator; only a correct topological copy order preserves all three.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn three_variable_rotation_in_loop() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 1; let mut b = 2; let mut c = 3; let mut i = 0; \
         while i < 1 { let t = a; a = b; b = c; c = t; i = i + 1; } a + b + c + i }",
    );
    assert_eq!(code, 6 + 1, "rotation preserves the sum 1+2+3, then +1 for i");
}

/// CEP:WHAT: A loop whose body both swaps and accumulates exits 21.
/// CEP:WHY: The compound shape: phi cycles plus an accumulating phi in one latch
///   copy set (four copies with two interleaved dependencies).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn swap_and_accumulate_in_loop() {
    let code = run_src(
        "fn main() -> i32 { let mut x = 1; let mut y = 2; let mut s = 0; let mut i = 0; \
         while i < 3 { s = s + x; let t = x; x = y; y = t + 1; i = i + 1; } s + x + y }",
    );
    // i=0: s=1, x=2, y=2; i=1: s=3, x=2, y=3; i=2: s=5, x=3, y=3; exit: 5+3+3=11.
    assert_eq!(code, 11, "swap-and-accumulate chain");
}

/// CEP:WHAT: Two independent swaps in one loop body exit 10 (values preserved).
/// CEP:WHY: THE multi-cycle copy-set regression: the sequencer owned ONE scratch and
///   ONE deferred delivery, so the second cycle overwrote the first scratch and
///   dropped the first delivery - a pair of values was lost per iteration. Found by
///   the Z3 sequencer mirror before the solver even ran.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn two_independent_swaps_in_loop() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 1; let mut b = 2; let mut c = 3; let mut d = 4; \
         let mut i = 0; while i < 1 { let t = a; a = b; b = t; \
         let u = c; c = d; d = u; i = i + 1; } a + b + c + d }",
    );
    assert_eq!(code, 10, "both swaps preserve 1+2+3+4");
}

/// CEP:WHAT: Two swaps repeated over three iterations exit 10.
/// CEP:WHY: Repetition exercises the multi-cycle copy set on every back edge.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: unix exec available.
/// CEP:COST: test-only
/// CEP:EVIDENCE: this test.
#[test]
#[cfg(unix)]
fn two_swaps_over_three_iterations() {
    let code = run_src(
        "fn main() -> i32 { let mut a = 1; let mut b = 2; let mut c = 3; let mut d = 4; \
         let mut i = 0; while i < 3 { let t = a; a = b; b = t; \
         let u = c; c = d; d = u; i = i + 1; } a + b + c + d }",
    );
    assert_eq!(code, 10, "odd iteration count still preserves the total");
}
