# RV8FV: The Lean 4 Formal Model of the RV8 Core

## Abstract

RV8FV is a Lean 4 formalization of the RV8 LIR value language and of the first
proved optimization passes. It defines the machine semantics (64-bit wrapping
words with explicit overflow, fail-closed traps, total word memory), a
fuel-bounded executor, strict observational equivalence, the constant-folding
pass with its complete machine-checked soundness theorem, the dead-code
elimination criterion with its congruence machinery, and the formal catalog of
all 176 optimization passes from design.md. Inputs: RV8FV source modules under
`verification/lean/`. Outputs: compiled theorems checked by `lake build` and
executable smoke checks run by `lake test`.

## Formal Definitions

The core types (`RV8FV/Spec.lean`):

```lean
abbrev Word := Nat                    -- reduced modulo W64 = 2^64 by every operation
abbrev VReg := Nat                    -- dense register identity
inductive Arg | reg : VReg -> Arg | imm : Word -> Arg
inductive Instr
  | nop | copy : VReg -> Arg -> Instr | binop : VReg -> Binop -> Arg -> Arg -> Instr
  | load : VReg -> Arg -> Instr | store : Arg -> Arg -> Instr
  | call : VReg -> Nat -> List Arg -> Instr
inductive Term | jmp : BlockId -> Term | br : Arg -> BlockId -> BlockId -> Term
  | ret : Option Arg -> Term
structure Block | instrs : List Instr | term : Term
structure Prog | blocks : List Block
```

Domain constraints: every word operation reduces modulo `W64` (`wrap`,
`wadd`, `wsub`, `wmul`, `wshl`); `Binop.eval` returns `none` exactly for the
division-by-zero case, which raises the `divZero` trap. `WfSSA p` states that
every vreg has at most one definition.

The semantics (`RV8FV/Machine.lean`): a `Machine` carries a partial register
file, a total memory, a block program counter, and an instruction cursor. An
undefined read raises the `undefRead` trap; an out-of-range block index raises
`badBlock`; an out-of-range target raises `badTarget`. `run n p m` executes at
most `n` ticks; fuel exhaustion is the `Outcome.fuel` outcome.

Equivalence: `ObsEquiv` compares the trap class, the returned value, and the
final memory pointwise; register cells, pc, and the cursor are unobservable.
`SemEquiv p q` is strict: the outcomes agree at every fuel and from every
initial state. All proved passes preserve the fuel profile exactly, so no
quotienting by fuel and no classical choice is used anywhere in the
development.

## Algorithm

Each proved pass is an in-place rewrite preserving the step count:

1. Constant folding (`RV8FV/Fold.lean`, catalog #39): `d = op imm imm` becomes
   `d = copy (imm (op.eval a b))`. A division by a zero immediate is never
   folded (it must keep trapping). Complexity: O(size p). Theorem
   `foldProg_run_eq`: the folded program's execution trace is literally
   identical to the original's; `foldProg_sound`: strict `SemEquiv`;
   `ucount_foldProg`: identical useful-work cost; `foldProg_size`: identical
   static size.
2. Dead-code elimination (`RV8FV/Dce.lean`, catalog #34): a read-free
   immediate materialization `d = copy (imm w)` whose destination is used
   nowhere in the program and not before its own position is replaced in place
   by a `nop`. The read-free restriction is required by
   equivalence-for-all-initial-states: any instruction that reads a register
   could trap on an undefined read in one program and not in the other.
3. The fused in-block sweep (`RV8FV/Opt.lean`): the formal specification
   implementing catalog #20 (branch folding), #35 (local CSE), #38 (copy
   propagation), #39 (constant folding), and #40 (constant propagation) as one
   forward sweep with a substitution environment, a common-subexpression
   table, and a defined-before-use guard set. Division is never
   common-subexpressed and division by a zero immediate is never folded.

The catalog (`RV8FV/Catalog.lean`) lists all 176 passes of design.md with
their category, effect class, and formal-verification class; `catalog_length`
and `catalog_ids_strictly_increasing` are machine-checked.

## Invariants

1. Every proved pass preserves `SemEquiv` (strict observational equivalence at
   every fuel, from every initial state).
2. No proved pass increases useful work (`ucount`); folding preserves it
   exactly, DCE never increases it.
3. No proved pass increases static size.
4. Overflow is never simplified away: word operations reduce modulo 2^64.
5. The division-by-zero trap class is preserved exactly; a division by a zero
   immediate is never folded or common-subexpressed.
6. Zero `sorry`, zero domain axioms: the development uses only Lean's logical
   foundations.

## Integration Points

Depends on: nothing external (Lean 4 core only; the toolchain is pinned in
`verification/lean/lean-toolchain`).

Depended on by: the CI formal-verification gate
(`.github/workflows/ci.yml`, step "Formal verification") runs `lake build`,
`lake test`, and a zero-sorry grep; the Rust implementation is linked by the
runtime bridges described in each module's `CEP:FV-REFINEMENT` field (the
brute-force regalloc corpora and the end-to-end deterministic suites execute
the same fixture programs this model encodes in `RV8FV/Smoke.lean`).

## Known Limitations

1. The fused sweep (`Opt.lean`) is an FV-1 specification: its per-block
   structure, guard sets, and monotonicity are proved, but the end-to-end
   `SemEquiv` theorem for the full sweep is a tracked obligation.
2. DCE is proved for the read-free class only (see `Dce.lean`); extending the
   removable class requires entry-state contracts (dominance), which the model
   deliberately does not assume. The run-level theorems for DCE are specified
   in `DceProof.lean`'s obligations section with the per-instruction
   congruence machinery proved.
3. The model covers the LIR value language: calls are uninterpreted (they
   clobber all registers and return an opaque value in the destination), and
   loads are total reads. Inline assembly, vector operations, and
   floating-point are outside the model.
4. Global optimality across all 176 catalog passes is not claimed; the
   optimality statements are per-stage and fixpoint-based.
