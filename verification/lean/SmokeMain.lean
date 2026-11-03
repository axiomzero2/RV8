/-
CEP:FILE: verification/lean/SmokeMain.lean
CEP:WHAT: The `lake test` entry point: executes the smoke checks and exits
  nonzero on any failure.
CEP:WHY: CEP&CC 54.5.1 requires `lake test` as a CI gate.
CEP:CLASS: FV-2
CEP:STATUS: complete
CEP:FAILURE: Any assertion failure prints the check name and exits with code 1.
CEP:ASSUMES: none.
CEP:FV-CLASS: FV-2
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/SmokeMain.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: build-orchestration only.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Smoke

open RV8FV

def check (name : String) (b : Bool) : IO Unit := do
  if b then
    IO.println s!"ok: {name}"
  else
    throw (IO.userError s!"FAILED: {name}")

def outcomeTag : Outcome → String
  | .fuel => "out of fuel"
  | .stuck t => "stuck: " ++ toString (repr t)
  | .halt _ (some w) => "halted with " ++ toString (repr w)
  | .halt _ none => "halted without a value"

def headInstrs (p : Prog) : List Instr :=
  match p.blocks[0]? with
  | some b => b.instrs
  | none => []

def firstInstr (p : Prog) : Option Instr :=
  match p.blocks[0]? with
  | some b => b.instrs[0]?
  | none => none

def main : IO Unit := do
  -- the machine executes the program and returns 7
  match run 100 smokeProg smokeInit with
  | .halt _ (some w) => check "run returns 7" (w = 7)
  | other => check s!"run returns 7 (got {outcomeTag other})" false
  -- folding produces the folded program
  check "foldProg computes" (headInstrs (foldProg smokeProg) == headInstrs smokeFolded)
  -- the folded program returns the same value
  match run 100 (foldProg smokeProg) smokeInit with
  | .halt _ (some w) => check "folded run returns 7" (w = 7)
  | other => check s!"folded run returns 7 (got {outcomeTag other})" false
  -- overflow wraps
  check "overflow wraps" (wadd 9223372036854775808 9223372036854775808 = 0)
  -- division by zero traps
  check "div-by-zero traps" (Binop.eval .udiv 5 0 = none)
  -- the catalog is complete
  check "catalog has 176 passes" (catalog.length == 176)
  check "fv0 passes are 34 and 39" (fv0Passes == [34, 39])
  -- DCE removes a dead immediate load
  let deadProg : Prog :=
    { blocks := [
        { instrs := [.copy 0 (.imm 5), .copy 1 (.imm 7)]
          term := .ret (some (.reg 1)) } ] }
  check "dce fires on dead load" (firstInstr (match nopifyOne deadProg with
    | some p' => p'
    | none => deadProg) == some .nop)
  check "dce preserves the return value" (match nopifyOne deadProg with
    | some p' => match run 100 p' smokeInit with
      | .halt _ (some w) => w = 7
      | _ => false
    | none => false)
  -- the formal pass registry and the architecture
  check "frames registry has 176 passes" (frames.length == 176)
  check "frames ids start at 1 and end at 176"
    (frames.head!.id == 1 && frames.getLast!.id == 176)
  check "one shared regalloc law holds on a concrete target"
    (({ name := "toy4", physRegs := 16, alloc := sharedRegalloc 16,
        alloc_is_shared := fun q => rfl } : Target).alloc smokeProg
      == sharedRegalloc 16 smokeProg)
  check "strength reduction: 6 * 4 = 6 << 2" (wmul 6 4 == wshl 6 2)
  check "delta encoding round-trips" (deltaDecode (deltaEncode [5, 9, 20]) == [5, 9, 20])
  IO.println "smoke: all checks passed"
