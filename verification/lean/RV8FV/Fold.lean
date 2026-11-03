/-
CEP:FILE: verification/lean/RV8FV/Fold.lean
CEP:WHAT: Catalog pass #39 (constant folding) as a whole-program in-place rewrite,
  with its complete machine-checked soundness proof: the folded program has
  LITERALLY identical execution traces (theorem foldProg_run_eq), hence is
  observationally equivalent at every fuel and from every initial state
  (foldProg_sound, which is SemEquiv), and the useful-work cost is identical
  (ucount_foldProg).
CEP:WHY: CEP&CC 54.2.1 lists compiler transformation equivalence proofs as FV-0.
  Folding replaces `d = op imm imm` by `d = copy imm result`; wrapping arithmetic
  and the division-by-zero trap are preserved exactly (a division by a zero
  immediate is never folded, and foldEval is the bounded Binop.eval, never
  unbounded integers).
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A folding rule that fires on a trapping operation, or that computes
  with unbounded integers, would break the equivalence and is rejected by the
  model.
CEP:ASSUMES: none beyond the model (no SSA hypothesis needed: folding is
  in-place and value-preserving on every path).
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Fold.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Machine.lean; the Rust constant folder in the middle-end is
  bridged to this theorem by the end-to-end deterministic suites.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Opt

namespace RV8FV

/-! ## The folding pass. -/

/-- Fold one instruction: a binary operation over two immediates becomes an
    immediate materialization of the wrapped result (#39). A division whose
    immediate divisor is zero is left alone: it must keep trapping. -/
def foldInstr : Instr → Instr
  | .binop d o (.imm x) (.imm y) =>
    if o = .udiv && y = 0 then .binop d .udiv (.imm x) (.imm y)
    else match foldEval o x y with
      | some r => .copy d (.imm r)
      | none => .binop d o (.imm x) (.imm y)
  | i => i

/-- The whole-program folding pass: every instruction of every block. -/
def foldProg (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl => { bl with instrs := bl.instrs.map foldInstr }) }

/-- Folding preserves the instruction weight: nops stay nops, real instructions
    stay real. -/
theorem instrNn_fold (i : Instr) : instrNn (foldInstr i) = instrNn i := by
  cases i with
  | nop => rfl
  | copy d a => rfl
  | load d a => rfl
  | store a b => rfl
  | call d f args => rfl
  | binop d o a b =>
    cases a with
    | reg x => rfl
    | imm x =>
      cases b with
      | reg y => rfl
      | imm y =>
        simp only [foldInstr]
        split
        · rfl
        · split <;> simp [instrNn]

/-- The one-instruction agreement: a folded instruction performs the same
    register write, the same memory effect, and the same trap class. -/
theorem fold_step_agree (m : Machine) (i : Instr) :
    stepInstr m (foldInstr i) = stepInstr m i := by
  cases i with
  | nop => rfl
  | copy d a => rfl
  | load d a => rfl
  | store a b => rfl
  | call d f args => rfl
  | binop d o a b =>
    cases a with
    | reg x => rfl
    | imm x =>
      cases b with
      | reg y => rfl
      | imm y =>
        simp only [foldInstr]
        by_cases hu : (o = .udiv && y = 0) = true
        · rw [if_pos hu]
          simp only [Bool.and_eq_true] at hu
          obtain ⟨ho, _⟩ := hu
          rw [of_decide_eq_true ho]
        · rw [if_neg hu]
          simp only [foldEval]
          cases hf : o.eval x y with
          | none => rfl
          | some r =>
            simp only [stepInstr, Arg.eval]
            rw [hf]

/-- The block at `pc` of the folded program is the folding of the block at `pc`. -/
theorem foldProg_block (p : Prog) (pc : Nat) :
    (foldProg p).block? pc =
      (p.block? pc).map (fun bl => { bl with instrs := bl.instrs.map foldInstr }) := by
  simp [foldProg, Prog.block?]

/-! ## The tick-level equality. -/

/-- One tick of the folded program IS one tick of the original program: the same
    instruction is executed with the same effect, and the terminator is
    untouched. -/
theorem tick_foldProg (p : Prog) (m : Machine) : tick (foldProg p) m = tick p m := by
  unfold tick
  rw [foldProg_block p m.pc]
  cases hb : p.block? m.pc with
  | none => rfl
  | some bl =>
    simp only [Option.map_some]
    have hterm : ({ bl with instrs := bl.instrs.map foldInstr }).term = bl.term := rfl
    cases hi : bl.instrs[m.iip]? with
    | some i =>
      rw [getElem?_map']
      rw [hi]
      simp only [Option.map_some]
      rw [fold_step_agree m i]
    | none =>
      have hlk : (bl.instrs.map foldInstr)[m.iip]?
          = Option.map foldInstr (bl.instrs[m.iip]?) :=
        getElem?_map' foldInstr bl.instrs m.iip
      rw [hlk, hi]
      rw [show (foldProg p).blocks.length = p.blocks.length from by simp [foldProg]]
      rfl

/-- The folded program and the original program have literally identical
    execution traces for every fuel and every initial state. -/
theorem foldProg_run_eq : ∀ (n : Nat) (p : Prog) (m : Machine),
    run n (foldProg p) m = run n p m
  | 0, _, _ => rfl
  | n + 1, p, m => by
      simp only [run]
      rw [tick_foldProg p m]
      cases ht : tick p m with
      | next m1 => exact foldProg_run_eq n p m1
      | stuck t => rfl
      | halt m1 v => rfl

/-! ## The FV-0 theorems. -/

/-- Constant folding is strictly semantics-preserving: observational equivalence
    at every fuel, from every initial state. -/
theorem foldProg_sound (p : Prog) : SemEquiv p (foldProg p) := by
  intro n m
  rw [foldProg_run_eq n p m]
  exact ObsEquiv_refl _

/-- The cursor weight of the folded program equals the original's. -/
theorem curNn_foldProg (p : Prog) (m : Machine) :
    curNn (foldProg p) m = curNn p m := by
  unfold curNn
  rw [foldProg_block p m.pc]
  cases hb : p.block? m.pc with
  | none => rfl
  | some bl =>
    simp only [Option.map_some]
    cases hi : bl.instrs[m.iip]? with
    | none =>
      have hlk : (bl.instrs.map foldInstr)[m.iip]?
          = Option.map foldInstr (bl.instrs[m.iip]?) :=
        getElem?_map' foldInstr bl.instrs m.iip
      rw [hlk, hi]
      rfl
    | some i =>
      have hlk : (bl.instrs.map foldInstr)[m.iip]?
          = Option.map foldInstr (bl.instrs[m.iip]?) :=
        getElem?_map' foldInstr bl.instrs m.iip
      rw [hlk, hi]
      simp only [Option.map_some, instrNn_fold]

/-- Folding never increases (in fact preserves) the useful-work cost. -/
theorem ucount_foldProg : ∀ (n : Nat) (p : Prog) (m : Machine),
    ucount n (foldProg p) m = ucount n p m
  | 0, _, _ => rfl
  | n + 1, p, m => by
      simp only [ucount]
      rw [tick_foldProg p m, curNn_foldProg]
      cases ht : tick p m with
      | next m1 =>
        simp only []
        rw [ucount_foldProg n p m1]
      | stuck t => rfl
      | halt m1 v => rfl

/-- Static size is preserved: folding is in-place. -/
theorem foldProg_size (p : Prog) : (foldProg p).size = p.size := by
  unfold Prog.size foldProg
  have h : ∀ (bls : List Block) (acc : Nat),
      (bls.map (fun bl => { bl with instrs := bl.instrs.map foldInstr })).foldl
        (fun a bl => a + bl.instrs.length) acc
      = bls.foldl (fun a bl => a + bl.instrs.length) acc := by
    intro bls
    induction bls with
    | nil => intro acc; rfl
    | cons bl rest ih =>
      intro acc
      simp only [List.map_cons, List.foldl_cons]
      have hlen : ({ bl with instrs := bl.instrs.map foldInstr }).instrs.length
          = bl.instrs.length := by simp
      rw [hlen, ih (acc + bl.instrs.length)]
  exact h p.blocks 0

end RV8FV
