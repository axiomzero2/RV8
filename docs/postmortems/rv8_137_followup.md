# RV8-137 Follow-Up Postmortem: Five Defects Remaining After the Parallel Fix

## Abstract

After the lane-private MCF fix landed (commit 81c5224) and the ISL target descriptions landed (commit 918a500), the loop program passed but five defect classes remained: call results produced no values, block layout was not an execution order, branch arms were read from block numbering, the phi-copy sequencer emitted destroying copies first with one scratch for all cycles, and spill scratches ignored liveness. This document records each root cause, the fix ported on top of the parallel work, and the evidence. The workspace now passes 484 tests with all gates green.

## Defects

### 1. Call results had no value

Symptom: `x = x + sub(i, 1)` inside a loop failed to compile with a missing-value error; a tail call returned RAX residue (documented as the convention in the parallel fix).

Root cause: `hetir_to_lir` never marked `ControlOp::Call` nodes as value-producing, so no consumer of a call result could resolve a virtual register.

Fix: calls with non-Unit results produce a value; the LIR Call carries `result`; isel emits the RAX result move immediately after the call (the scheduler models the caller-saved clobber, so the move cannot float above it); isel's emission dispatch routes result-bearing calls through the side-instruction path. Two regression tests that pinned the weaker bare-Ret convention were updated to the stronger contract (the call result rides the Ret operand).

### 2. Block layout was id-order, not an execution order

Symptom: if-inside-while failed with use-before-definition errors or hung.

Root cause: synthesized join blocks can carry block ids below the region blocks that must precede them, and the reachability renumbering preserved old-id order, so a merge with its phi copies could precede the arms defining those values.

Fix: reachable blocks are laid out in deterministic DFS reverse post-order from the entry; back edges become the only backward transfers, so every use follows a definition in layout order.

### 3. Branch arms were read from block numbering

Symptom: after any layout change, loops could exit immediately.

Root cause: the branch emission took the TRUE arm as the lower-numbered successor in the successor table, which block renumbering could silently swap.

Fix: branch arms derive from the Branch node's control out-edges ordered by target node id (the flattener creates the TRUE-arm node first), independent of block numbering; the fallthrough adaptation reads those arms.

### 4. The phi-copy sequencer emitted the destroying copy first, and owned one scratch

Symptom: the fibonacci rotation computed the wrong seed (every iteration read the NEW b); two independent swaps in one loop lost a pair per iteration.

Root cause: the ready condition was inverted (a copy was emitted when its SOURCE was not a pending DESTINATION), and the cycle breaker reused a single scratch value plus a single deferred delivery.

Fix: a copy is emitted when its DESTINATION is not a pending SOURCE; every pure cycle breaks through its own fresh scratch with its own deferred delivery. The block-local use-before-def check that initially rejected the correctly sequenced copies now validates against the global linearized first-definition table (multi-def values are legally read before their same-block redefinition). The Z3 sequencer mirror found the multi-cycle case before the solver ran.

### 5. Spill scratches ignored liveness

Symptom: a call inside a loop hung; peak-pressure spill definitions exhausted the scratch pool.

Root cause: scratch registers were chosen avoiding only the current instruction's operands; a scratch write is permanent, so it could destroy values read later in the same phi-copy run or on the next loop iteration (invisible to MIR order).

Fix: the rewriter computes exact backward liveness per MIR position (kills at destinations, gens at sources), initialized from lanes crossing the block end (the wrap-sound ranges make next-iteration reads visible). Spilled-source and memory scratches avoid the live-out set plus the instruction's reads; spilled-destination scratches avoid only live-out (x86 reads operands before writing the destination). A new `StoreStackImm` opcode (`mov qword [slot], imm32`) spills immediate definitions with no register at all, which keeps peak-pressure definitions feasible instead of failing closed.

### 6. The regalloc test module was orphaned

`crates/rv8_backend/src/regalloc/tests.rs` (426 lines, including the single-definition brute-force corpus and the spill-path acceptance) was never declared as a module, so its tests never executed; the file even carried a syntax error hidden by never being compiled. It is wired (`#[cfg(test)] mod tests;`), repaired, and green.

### 7. Parallel end-to-end tests raced on one temp executable

`pipeline::run` wrote a fixed `/tmp/rv8c_run_bin`; parallel test threads overwrote each other's executable, making the end-to-end suites flaky under `cargo test`. The path now includes the process id and an atomic counter.

## Verification

1. Workspace: 484 tests pass, zero ignored, including the previously orphaned regalloc module.
2. Behavior: the probe set (loop with zero start, fibonacci rotation, if-inside-while, double swap, call-inside-loop, nested loops, reversed loops, compound conditionals) all compute hand-checked exit codes.
3. Gates: build with `-D warnings`, clippy, rustdoc with `-D warnings`, and the CEP lint (S0=S1=0) all pass.
4. Z3: `tools/z3_verify/z3_rv8_137.py` proves the lane model, the sequencer (7382 copy-set instances), and hop-freedom interference arguments by refutation.

## Known Limitations

1. The multi-commodity optimum is still approximated (lane-private job pairs plus the improvement pass); single-commodity flow cannot carry lane identity through a shared single-take edge.
2. If-as-value is not in the language subset; an if used as a function tail silently defaults to 0.
3. Store-immediates cover only `MovRegImm32` definitions; other peak-pressure spill shapes can still fail closed (scratch exhaustion) rather than miscompile.
