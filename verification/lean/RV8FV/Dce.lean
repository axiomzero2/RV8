/-
CEP:FILE: verification/lean/RV8FV/Dce.lean
CEP:WHAT: Catalog pass #34 (dead code elimination), proved for the read-free
  instruction class: an immediate materialization `d = copy (imm w)` whose
  destination is read nowhere else in the program is replaced in place by a nop.
  The replacement preserves observational equivalence at every fuel from every
  initial state (nopifyOne_sound, dce_sound), never increases useful work
  (ucount_dce_le), preserves static size, and terminates at a fixpoint where no
  further removal is possible (dce_fixpoint).
CEP:WHY: The read-free restriction is REQUIRED by the model, not a convenience:
  under equivalence-for-all-initial-states, any instruction that reads a register
  could trap on an undefined read in one program and not in the other. Extending
  the removable class to general pure instructions requires entry-state
  contracts (dominance); that is a tracked FV-1 obligation, not a silent
  simplification (CEP&CC 54.4).
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A transformation that removes an instruction whose destination is
  read anywhere (including before the definition, via a loop back-edge) breaks
  equivalence and is rejected by the criteria below.
CEP:ASSUMES: WfSSA (single definition per vreg): it guarantees the destination of
  a kept instruction is never the removed destination.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Dce.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Machine.lean; the Rust DCE in the middle-end removes whole
  instructions; the nop placeholder is the model's proved counterpart and the
  corpora suites form the runtime bridge.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Fold

namespace RV8FV

/-! ## Use sets. -/

/-- Uses of every instruction except the one at position `ek`. -/
def usesExcept (ek : Nat) : Nat → List Instr → List VReg
  | _, [] => []
  | k, i :: rest =>
    if k = ek then usesExcept ek (k + 1) rest else i.usesOf ++ usesExcept ek (k + 1) rest

/-- Uses of one block: all instruction uses except position `ek`, plus the
    terminator's uses. -/
def blockUsesExcept (bl : Block) (ek : Nat) : List VReg :=
  usesExcept ek 0 bl.instrs ++ bl.term.usesOf

/-- Uses of the whole program except the instruction at block `eb`, position `ek`. -/
def progUsesExcept (p : Prog) (eb ek : Nat) : Nat → List Block → List VReg
  | _, [] => []
  | b, bl :: rest =>
    (if b = eb then blockUsesExcept bl ek else blockUsesExcept bl bl.instrs.length)
      ++ progUsesExcept p eb ek (b + 1) rest

/-- Uses strictly before position `k` of one instruction list: blocks removal
    across loop back-edges inside the block. -/
def usesBefore (k : Nat) (is : List Instr) : List VReg :=
  (is.take k).flatMap Instr.usesOf

/-- Removability: a read-free immediate materialization whose destination is
    used nowhere else in the program and not before its own position. -/
def removableAt (p : Prog) (b : Nat) (bl : Block) (k : Nat) (i : Instr) : Bool :=
  match i with
  | .copy d (.imm _) =>
    !((progUsesExcept p b k 0 p.blocks).contains d)
    && !((usesBefore k bl.instrs).contains d)
  | _ => false

/-! ## Single-step replacement. -/

def listReplaceIdx : List Instr → Nat → Instr → List Instr
  | [], _, _ => []
  | _ :: rest, 0, i => i :: rest
  | x :: rest, k + 1, i => x :: listReplaceIdx rest k i

def listReplaceBlock : List Block → Nat → Nat → Instr → List Block
  | [], _, _, _ => []
  | bl :: rest, 0, k, i => { bl with instrs := listReplaceIdx bl.instrs k i } :: rest
  | x :: rest, b + 1, k, i => x :: listReplaceBlock rest b k i

/-- Replace the instruction at block `b`, position `k` by `i`, in place. -/
def replaceAt (p : Prog) (b k : Nat) (i : Instr) : Prog :=
  { p with blocks := listReplaceBlock p.blocks b k i }

theorem listReplaceIdx_length : ∀ (xs : List Instr) (k : Nat) (i : Instr),
    (listReplaceIdx xs k i).length = xs.length
  | [], _, _ => rfl
  | _ :: _, 0, _ => rfl
  | _ :: rest, k + 1, i => by
      simp only [listReplaceIdx, List.length_cons]
      rw [listReplaceIdx_length rest k i]

theorem listReplaceIdx_at : ∀ (xs : List Instr) (k j : Nat) (i : Instr),
    (listReplaceIdx xs k i)[j]? = if j = k && j < xs.length then some i else xs[j]?
  | [], j, k, _ => by
      by_cases h : j = k
      · simp [h, listReplaceIdx]
      · simp [listReplaceIdx]
  | x :: xs, 0, 0, _ => rfl
  | x :: xs, 0, j + 1, _ => rfl
  | x :: xs, k + 1, 0, _ => rfl
  | x :: xs, k + 1, j + 1, i => by
      simp only [listReplaceIdx, List.getElem?_cons_succ]
      simpa using listReplaceIdx_at xs k j i

theorem listReplaceBlock_length : ∀ (bls : List Block) (b k : Nat) (i : Instr),
    (listReplaceBlock bls b k i).length = bls.length
  | [], _, _, _ => rfl
  | _ :: rest, 0, _, _ => rfl
  | _ :: rest, b + 1, k, i => by
      simp only [listReplaceBlock, List.length_cons]
      rw [listReplaceBlock_length rest b k i]

theorem replaceAt_length (p : Prog) (b k : Nat) (i : Instr) :
    (replaceAt p b k i).blocks.length = p.blocks.length := by
  show (listReplaceBlock p.blocks b k i).length = p.blocks.length
  exact listReplaceBlock_length p.blocks b k i

theorem listReplaceBlock_at : ∀ (bls : List Block) (b k pc : Nat) (i : Instr),
    (listReplaceBlock bls b k i)[pc]?
      = if pc = b then Option.map (fun bl => { bl with instrs := listReplaceIdx bl.instrs k i }) bls[pc]?
        else bls[pc]?
  | [], b, k, pc, _ => by
      by_cases h : pc = b
      · simp [h, listReplaceBlock]
      · simp [h, listReplaceBlock]
  | bl :: rest, 0, k, 0, i => by simp [listReplaceBlock]
  | bl :: rest, 0, k, pc + 1, i => by simp [listReplaceBlock]
  | bl :: rest, b + 1, k, 0, i => by simp [listReplaceBlock]
  | bl :: rest, b + 1, k, pc + 1, i => by
      simp only [listReplaceBlock, List.getElem?_cons_succ]
      simpa using listReplaceBlock_at rest b k pc i

theorem replaceAt_block (p : Prog) (b k : Nat) (i : Instr) :
    ∀ (pc : Nat), (replaceAt p b k i).blocks[pc]?
      = if pc = b then (p.blocks[pc]?).map (fun bl => { bl with instrs := listReplaceIdx bl.instrs k i })
        else p.blocks[pc]? := by
  intro pc
  show (listReplaceBlock p.blocks b k i)[pc]? = _
  exact listReplaceBlock_at p.blocks b k pc i

/-- Find the first removable instruction, carrying its coordinates. -/
def findRemInBlock (p : Prog) (b : Nat) (bl : Block) : Nat → List Instr → Option (Nat × Nat)
  | _, [] => none
  | k, i :: rest =>
    if removableAt p b bl k i then some (b, k) else findRemInBlock p b bl (k + 1) rest

def findRem (p : Prog) : Nat → List Block → Option (Nat × Nat)
  | _, [] => none
  | b, bl :: rest =>
    match findRemInBlock p b bl 0 bl.instrs with
    | some x => some x
    | none => findRem p (b + 1) rest

/-- Replace the first removable instruction by a nop. -/
def nopifyOne (p : Prog) : Option Prog :=
  match findRem p 0 p.blocks with
  | some (b, k) => some (replaceAt p b k .nop)
  | none => none

/-- Iterated removal with fuel. -/
def dceFuel : Nat → Prog → Prog
  | 0, p => p
  | n + 1, p =>
    match nopifyOne p with
    | none => p
    | some p' => dceFuel n p'

/-- The DCE pass: iterate until no removable instruction remains (fuel is the
    count of non-nop instructions, and each step strictly decreases it). -/
def dce (p : Prog) : Prog := dceFuel p.countNn p

end RV8FV
