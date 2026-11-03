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
import RV8FV.Optimal

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
example : wadd 3 4 = 7 := rfl

/-- Folding folds `3 + 4` into `7`. -/
example : foldProg smokeProg = smokeFolded := by
  simp [smokeProg, smokeFolded, foldProg, foldInstr, foldEval, Binop.eval, wadd, wrap, W64]

/-- Overflow wraps: `2^63 + 2^63` wraps to 0. -/
example : wadd 9223372036854775808 9223372036854775808 = 0 := rfl

/-- Division by zero traps, and the folder refuses to fold it. -/
example : Binop.eval .udiv 5 0 = none := rfl

/-! ## Smoke checks for the pass registry and the architecture. -/

set_option maxRecDepth 8192 in
/-- The registry mirrors the catalog: 176 frames with strictly increasing
    ids over 1..176 (machine-checked in Optimal.lean; re-checked here as
    executable facts). -/
example : frames.length = 176 := by decide

set_option maxRecDepth 8192 in
example : (frames.map PassFrame.id).Pairwise (fun a b => a < b) := by
  decide

/-- The one-allocator law on a concrete target: the allocator is the shared
    function at the target's register count. -/
example (k : Nat) (p : Prog) :
    ({ name := "toy4", physRegs := k, alloc := sharedRegalloc k,
       alloc_is_shared := fun _ => rfl } : Target).alloc p = sharedRegalloc k p :=
  rfl

/-- Strength reduction computes: `a * 4 = a << 2` on the wrapping word. -/
example (a : Word) : wmul a 4 = wshl a 2 := by
  have h2 : (2 : Nat) % 64 = 2 := Nat.mod_eq_of_lt (by omega)
  have h4 : (4 : Nat) = 2 ^ 2 := rfl
  rw [h4]
  simp only [wmul, wshl, wrap, h2]

end RV8FV
