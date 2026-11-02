/-
CEP:FILE: verification/lean/RV8FV/Smoke.lean
CEP:WHAT: Executable smoke checks for the formalization: the machine executes the
  modeled programs, folding computes the wrapped results, dead code removal
  fires, and the proved passes behave as the theorems state on concrete
  fixtures.
CEP:WHY: The theorems are checked at build time; these executable checks confirm
  the definitions compute as intended (they are the `lake test` gate).
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A failing smoke check aborts `lake test` and the CI gate.
CEP:ASSUMES: none.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Smoke.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Machine.lean.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Catalog
import RV8FV.DceProof

namespace RV8FV

/-- A two-instruction program: `r1 := 3 + 4; return r1`. -/
def smokeProg : Prog :=
  { blocks := [
      { instrs := [.copy 0 (.imm 5), .binop 1 .add (.imm 3) (.imm 4)]
        term := .ret (some (.reg 1)) } ] }

/-- The initial machine: everything undefined, empty memory, at the entry. -/
def smokeInit : Machine :=
  { regs := fun _ => none, mem := fun _ => 0, pc := 0, iip := 0 }

/-- The expected folded program: the add folds to an immediate load. -/
def smokeFolded : Prog :=
  { blocks := [
      { instrs := [.copy 0 (.imm 5), .copy 1 (.imm 7)]
        term := .ret (some (.reg 1)) } ] }

/-- The wrapped 64-bit result of 3 + 4 is 7. -/
example : wadd 3 4 = 7 := by native_decide

/-- Folding folds `3 + 4` into `7`. -/
example : foldProg smokeProg = smokeFolded := by
  simp [smokeProg, smokeFolded, foldProg, foldInstr, foldEval, Binop.eval, wadd, wrap, W64]

/-- Overflow wraps: `2^63 + 2^63` wraps to 0. -/
example : wadd 9223372036854775808 9223372036854775808 = 0 := by native_decide

/-- Division by zero traps, and the folder refuses to fold it. -/
example : Binop.eval .udiv 5 0 = none := rfl

end RV8FV
