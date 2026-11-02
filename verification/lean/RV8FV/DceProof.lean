/-
CEP:FILE: verification/lean/RV8FV/DceProof.lean
CEP:WHAT: Machine-checked infrastructure for the DCE pass: the except-agreement
  state relation, per-instruction execution congruence under it, the
  use-membership characterization of the sweep exceptions, and the
  NopifiableAt criterion. The remaining run-level theorems are fully specified
  as tracked obligations (see the obligations section below).
CEP:WHY: CEP&CC 54.2.1 transformation equivalence proofs factor through these
  lemmas. The except-agreement relation (states agree everywhere except the
  destination of the removed instruction) is exactly what dead-code elimination
  preserves: dead register cells are unobservable.
CEP:CLASS: FV-1
CEP:STATUS: partial
CEP:FAILURE: A transformation that removes an instruction whose destination is
  read anywhere (including before the definition, via a loop back-edge) breaks
  equivalence and is rejected by the NopifiableAt criteria.
CEP:ASSUMES: Loads never fault (see Machine.lean); consequently loads are never
  removed. Calls are never removed and clear nothing here: the criteria already
  exclude every destination that any call could read.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/DceProof.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: The run-level theorems (tick simulation, per-fuel observational
  equivalence, useful-work non-increase, fixpoint termination) are specified in
  the obligations section and tracked for the FV-0 upgrade; the per-instruction
  congruence they rest on is proved here. The Rust DCE is bridged by the
  regression corpora.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Dce

namespace RV8FV

/-! ## State agreement except one destination. -/

/-- States agree everywhere except possibly at register `d` (the destination of
    the removed instruction), and on memory, pc, and the cursor. -/
def EqExcept (d : VReg) (m m' : Machine) : Prop :=
  (∀ v, v ≠ d → m.regs v = m'.regs v) ∧ (∀ w, m.mem w = m'.mem w)
  ∧ m.pc = m'.pc ∧ m.iip = m'.iip

theorem EqExcept_refl (m : Machine) (d : VReg) : EqExcept d m m :=
  ⟨fun v _ => rfl, fun _ => rfl, rfl, rfl⟩

theorem setReg_eq_except {m m' : Machine} {d e : VReg} {r : Option Word}
    (h : EqExcept d m m') : EqExcept d (setReg m e r) (setReg m' e r) := by
  obtain ⟨hr, hm, hpc, hiip⟩ := h
  refine ⟨fun v hv => ?_, fun w => hm w, hpc, hiip⟩
  by_cases hve : v = e
  · simp [setReg, hve]
  · simp [setReg, hve]
    exact hr v hv

theorem setMem_eq_except {m m' : Machine} {d : VReg} {a w : Word}
    (h : EqExcept d m m') : EqExcept d (setMem m a w) (setMem m' a w) := by
  obtain ⟨hr, hm, hpc, hiip⟩ := h
  refine ⟨hr, fun w' => ?_, hpc, hiip⟩
  by_cases hw : w' = a
  · simp [setMem, hw]
  · simp [setMem, hw]
    exact hm w'

/-- Operand evaluation agrees when every register it reads differs from `d`. -/
theorem Arg.eval_eq_except (d : VReg) (m m' : Machine) (h : EqExcept d m m') :
    ∀ (a : Arg), (∀ x, x ∈ a.regsOf → x ≠ d) → a.eval m = a.eval m' := by
  intro a
  cases a with
  | imm w => intro _; rfl
  | reg r =>
    intro ha
    exact h.1 r (ha r (by simp [Arg.regsOf]))

theorem Arg.evalList_eq_except (d : VReg) (m m' : Machine) (h : EqExcept d m m') :
    ∀ (args : List Arg), (∀ x, x ∈ args.flatMap Arg.regsOf → x ≠ d) →
      Arg.evalList args m = Arg.evalList args m'
  | [], _ => rfl
  | a :: rest, ha => by
      simp only [Arg.evalList]
      rw [Arg.eval_eq_except d m m' h a (fun x hx => ha x (by
        show x ∈ a.regsOf ++ rest.flatMap Arg.regsOf
        exact List.mem_append_left _ hx))]
      rw [Arg.evalList_eq_except d m m' h rest (fun x hx => ha x (by
        show x ∈ a.regsOf ++ rest.flatMap Arg.regsOf
        exact List.mem_append_right _ hx))]

/-! ## Instruction-step congruence. -/

/-- Agreement of step results: same trap class, or except-equal machines. -/
def ExceptEx (d : VReg) : Except Trap Machine → Except Trap Machine → Prop
  | .error t, .error t' => t = t'
  | .ok m, .ok m' => EqExcept d m m'
  | _, _ => False

theorem mem_uses_copy (e : VReg) (a : Arg) (x : VReg) (h : x ∈ a.regsOf) :
    x ∈ (Instr.copy e a).usesOf := by
  cases a with
  | imm w => exact absurd h (by simp [Arg.regsOf])
  | reg r => simpa [Instr.usesOf] using h

theorem mem_uses_binop_left (e : VReg) (o : Binop) (a b : Arg) (x : VReg) (h : x ∈ a.regsOf) :
    x ∈ (Instr.binop e o a b).usesOf := by
  show x ∈ a.regsOf ++ b.regsOf
  exact List.mem_append_left _ h

theorem mem_uses_binop_right (e : VReg) (o : Binop) (a b : Arg) (x : VReg) (h : x ∈ b.regsOf) :
    x ∈ (Instr.binop e o a b).usesOf := by
  show x ∈ a.regsOf ++ b.regsOf
  exact List.mem_append_right _ h

theorem mem_uses_load (e : VReg) (a : Arg) (x : VReg) (h : x ∈ a.regsOf) :
    x ∈ (Instr.load e a).usesOf := by
  cases a with
  | imm w => exact absurd h (by simp [Arg.regsOf])
  | reg r => simpa [Instr.usesOf] using h

theorem mem_uses_store_left (a v : Arg) (x : VReg) (h : x ∈ a.regsOf) :
    x ∈ (Instr.store a v).usesOf := by
  show x ∈ a.regsOf ++ v.regsOf
  exact List.mem_append_left _ h

theorem mem_uses_store_right (a v : Arg) (x : VReg) (h : x ∈ v.regsOf) :
    x ∈ (Instr.store a v).usesOf := by
  show x ∈ a.regsOf ++ v.regsOf
  exact List.mem_append_right _ h

theorem mem_uses_call (e : VReg) (f : Nat) (args : List Arg) (x : VReg)
    (h : x ∈ args.flatMap Arg.regsOf) : x ∈ (Instr.call e f args).usesOf := by
  exact h

/-- Executing the same instruction from states that agree except at `d` gives
    agreed results, provided the instruction does not read `d`. -/
theorem stepInstr_eq_except (d : VReg) (m m' : Machine) (h : EqExcept d m m') (i : Instr)
    (hu : ∀ x, x ∈ i.usesOf → x ≠ d) :
    ExceptEx d (stepInstr m i) (stepInstr m' i) := by
  cases i with
  | nop => exact h
  | copy e a =>
      have hae : a.eval m = a.eval m' :=
        Arg.eval_eq_except d m m' h a (fun x hx => hu x (mem_uses_copy e a x hx))
      simp only [stepInstr, hae]
      split
      · simp [ExceptEx]
      · rename_i w
        exact setReg_eq_except h
  | binop e o a b =>
      have hae : a.eval m = a.eval m' :=
        Arg.eval_eq_except d m m' h a (fun x hx => hu x (mem_uses_binop_left e o a b x hx))
      have hbe : b.eval m = b.eval m' :=
        Arg.eval_eq_except d m m' h b (fun x hx => hu x (mem_uses_binop_right e o a b x hx))
      simp only [stepInstr, hae, hbe]
      split
      · simp [ExceptEx]
      · rename_i va
        split
        · simp [ExceptEx]
        · rename_i vb
          split
          · simp [ExceptEx]
          · rename_i r
            exact setReg_eq_except h
  | load e a =>
      have hae : a.eval m = a.eval m' :=
        Arg.eval_eq_except d m m' h a (fun x hx => hu x (mem_uses_load e a x hx))
      simp only [stepInstr, hae]
      split
      · simp [ExceptEx]
      · rename_i w _
        obtain ⟨hr, hm, hpc, hiip⟩ := h
        refine ⟨fun v hv => ?_, fun w' => ?_, hpc, hiip⟩
        · by_cases hve : v = e
          · simp [setReg, hve, hm w]
          · simp only [setReg, hve]
            exact hr v hv
        · simp only [setReg]
          exact hm w'
  | store a v =>
      have hae : a.eval m = a.eval m' :=
        Arg.eval_eq_except d m m' h a (fun x hx => hu x (mem_uses_store_left a v x hx))
      have hve : v.eval m = v.eval m' :=
        Arg.eval_eq_except d m m' h v (fun x hx => hu x (mem_uses_store_right a v x hx))
      simp only [stepInstr, hae, hve]
      split
      · simp [ExceptEx]
      · rename_i addr
        split
        · simp [ExceptEx]
        · rename_i w
          exact setMem_eq_except h
  | call e f args =>
      have hargs : Arg.evalList args m = Arg.evalList args m' :=
        Arg.evalList_eq_except d m m' h args (fun x hx => hu x (mem_uses_call e f args x hx))
      simp only [stepInstr, hargs]
      split
      · simp [ExceptEx]
      · rename_i ws
        obtain ⟨hr, hm, hpc, hiip⟩ := h
        refine ⟨fun v hv => ?_, fun w' => ?_, hpc, hiip⟩
        · by_cases hve : v = e
          · simp [setReg, hve]
          · simp [setReg, hve]
        · simp only [setReg]
          exact hm w'

/-! ## Use-membership in the sweep exceptions. -/

theorem usesExcept_mem : ∀ (j : Nat) (is : List Instr) (ek s : Nat) (i : Instr),
    is[j]? = some i → s + j ≠ ek → ∀ x, x ∈ i.usesOf → x ∈ usesExcept ek s is := by
  intro j
  induction j with
  | zero =>
    intro is ek s i hi hne x hx
    cases is with
    | nil => simp at hi
    | cons i0 rest =>
      by_cases h0 : s = ek
      · exact absurd h0 (by simp [Nat.add_zero] at hne; exact hne)
      · simp only [usesExcept, if_neg h0, List.mem_append]
        have hii : i0 = i := by simpa using hi
        rw [hii]; exact Or.inl hx
  | succ j' ih =>
    intro is ek s i hi hne x hx
    cases is with
    | nil => simp at hi
    | cons i0 rest =>
      have hmem : rest[j']? = some i := by simpa using hi
      by_cases h1 : s = ek
      · simp only [usesExcept, if_pos h1]
        exact ih rest ek (s + 1) i hmem (by omega) x hx
      · simp only [usesExcept, if_neg h1, List.mem_append]
        exact Or.inr (ih rest ek (s + 1) i hmem (by omega) x hx)

theorem blockUsesExcept_mem (blk : Block) (ek j : Nat) (i : Instr)
    (hj : blk.instrs[j]? = some i) (hne : j ≠ ek) :
    i.usesOf ⊆ blockUsesExcept blk ek := by
  intro x hx
  simp only [blockUsesExcept, List.mem_append]
  exact Or.inl (usesExcept_mem j blk.instrs ek 0 i hj (by simp only [Nat.zero_add]; exact hne) x hx)

theorem term_uses_in_blockUsesExcept (blk : Block) (ek : Nat) :
    blk.term.usesOf ⊆ blockUsesExcept blk ek := by
  intro x hx
  simp only [blockUsesExcept, List.mem_append]
  exact Or.inr hx

/-- Every instruction that is not the excluded one has all its uses inside
    `progUsesExcept`. The running block index is `s`; the excluded block is at
    absolute position `eb` with the excluded instruction position `ek`. -/
theorem progUsesExcept_mem (p : Prog) (eb ek : Nat) :
    ∀ (bls : List Block) (s pc : Nat) (blk : Block) (j : Nat) (i : Instr),
      bls[pc]? = some blk → blk.instrs[j]? = some i → ¬(s + pc = eb ∧ j = ek) →
      i.usesOf ⊆ progUsesExcept p eb ek s bls := by
  intro bls
  induction bls with
  | nil => intro s pc blk j i hb _ _; cases pc <;> simp at hb
  | cons bl0 rest ih =>
    intro s pc blk j i hb hi hnot
    intro x hx
    cases pc with
    | zero =>
      have hbl0 : bl0 = blk := by simpa using hb
      rw [hbl0]
      by_cases hse : s = eb
      · have hjk : j ≠ ek := by
          intro hc
          apply hnot
          constructor
          · simp [hse]
          · exact hc
        simp only [progUsesExcept, if_pos hse, List.mem_append]
        exact Or.inl (blockUsesExcept_mem blk ek j i hi hjk hx)
      · simp only [progUsesExcept, if_neg hse, List.mem_append]
        have hlen : j < blk.instrs.length := by
          by_cases hcl : j < blk.instrs.length
          · exact hcl
          · rw [List.getElem?_eq_none_iff.mpr (by omega)] at hi
            simp at hi
        exact Or.inl (blockUsesExcept_mem blk (blk.instrs.length) j i hi (by omega) hx)
    | succ pc' =>
      have hrb : rest[pc']? = some blk := by simpa using hb
      have hih : ¬(s + 1 + pc' = eb ∧ j = ek) := fun hc => hnot ⟨by omega, hc.2⟩
      simp only [progUsesExcept, List.mem_append]
      exact Or.inr (ih (s + 1) pc' blk j i hrb hi hih hx)

/-- The terminator's uses of the target block are always inside
    `progUsesExcept`: terminators are never removed. -/
theorem term_uses_in_progUsesExcept (p : Prog) (eb ek : Nat) :
    ∀ (bls : List Block) (s pc : Nat) (blk : Block),
      bls[pc]? = some blk → blk.term.usesOf ⊆ progUsesExcept p eb ek s bls := by
  intro bls
  induction bls with
  | nil => intro s pc blk hb; cases pc <;> simp at hb
  | cons bl0 rest ih =>
    intro s pc blk hb
    cases pc with
    | zero =>
      have hbl0 : bl0 = blk := by simpa using hb
      rw [hbl0]
      intro x hx
      by_cases hse : s = eb
      · simp only [progUsesExcept, if_pos hse, List.mem_append]
        exact Or.inl (term_uses_in_blockUsesExcept blk ek hx)
      · simp only [progUsesExcept, if_neg hse, List.mem_append]
        exact Or.inl (term_uses_in_blockUsesExcept blk (blk.instrs.length) hx)
    | succ pc' =>
      have hrb : rest[pc']? = some blk := by simpa using hb
      intro x hx
      simp only [progUsesExcept, List.mem_append]
      exact Or.inr (ih (s + 1) pc' blk hrb hx)

/-! ## The NopifiableAt criterion. -/

/-- The instruction at block `b`, position `k` is a read-free immediate
    materialization whose destination is used nowhere else in the program and
    not before its own position (the loop back-edge guard). -/
def NopifiableAt (p : Prog) (b k : Nat) (dv : VReg) : Prop :=
  ∃ blk v,
    p.blocks[b]? = some blk ∧
    blk.instrs[k]? = some (Instr.copy dv (Arg.imm v)) ∧
    dv ∉ progUsesExcept p b k 0 p.blocks ∧
    dv ∉ usesBefore k blk.instrs

theorem not_mem_of_not_contains {L : List VReg} {x : VReg}
    (hc : (!L.contains x) = true) : x ∉ L := by
  intro hx
  rw [List.contains_iff_mem.mpr hx] at hc
  simp at hc

theorem contains_false_of_not_mem {L : List VReg} {x : VReg} (h : x ∉ L) :
    (!L.contains x) = true := by
  cases hc : L.contains x with
  | false => rfl
  | true => exact absurd (List.contains_iff_mem.mp hc) h

/-- A removable instruction (as found by the Boolean search) yields a
    `NopifiableAt` witness. -/
theorem nopifiable_of_removable (p : Prog) (b k : Nat) (blk : Block) (i : Instr)
    (hb : p.blocks[b]? = some blk) (hi : blk.instrs[k]? = some i)
    (hr : removableAt p b blk k i = true) :
    NopifiableAt p b k (match i with | .copy d _ => d | _ => 0) := by
  cases i with
  | nop => simp [removableAt] at hr
  | binop e o a b' => simp [removableAt] at hr
  | load e a => simp [removableAt] at hr
  | store a v => simp [removableAt] at hr
  | call e f args => simp [removableAt] at hr
  | copy d a =>
    cases a with
    | reg s' => simp [removableAt] at hr
    | imm w =>
      rw [removableAt] at hr
      have hc1 : (progUsesExcept p b k 0 p.blocks).contains d = false := by
        cases h2 : (progUsesExcept p b k 0 p.blocks).contains d with
        | false => rfl
        | true => rw [h2] at hr; simp at hr
      have hc2 : (usesBefore k blk.instrs).contains d = false := by
        cases h3 : (usesBefore k blk.instrs).contains d with
        | false => rfl
        | true => rw [h3] at hr; simp at hr
      refine ⟨blk, w, hb, hi, ?_, ?_⟩
      · exact not_mem_of_not_contains (by simpa using hc1)
      · exact not_mem_of_not_contains (by simpa using hc2)

/-! ## Run-level proof obligations (tracked, not yet discharged).

The remaining theorems for full FV-0 coverage of DCE are:

  1. `tick_replace_sim`: one tick of the nopified program agrees with one tick
     of the original, under `EqExcept dv m m'` — via `stepInstr_eq_except` for
     every kept instruction (its uses avoid `dv` by `progUsesExcept_mem`), the
     nop-step for the removed position, and the terminator agreement (its uses
     avoid `dv` by `term_uses_in_progUsesExcept`).
  2. `run_replace_sim`: the fuel induction lifting (1) to `ObsEquiv` at every
     fuel, with the invariant `∀ m1 m2, EqExcept dv m1 m2 → ObsEquiv ...`.
  3. `nopifyOne_sound`, `dce_sound`: the finder specification plus (2).
  4. `ucount_dce_le`: useful-work non-increase (the removed position weighs 1
     in the original and 0 as a nop).
  5. `dce_fixpoint`: termination via the strictly decreasing `Prog.countNn`.

Each obligation is fully specified here and in the Catalog; the per-instruction
congruence machinery they rest on (`stepInstr_eq_except`) is proved above. -/

end RV8FV
