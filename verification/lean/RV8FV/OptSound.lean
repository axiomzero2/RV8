/-
CEP:FILE: verification/lean/RV8FV/OptSound.lean
CEP:WHAT: The FV-0 soundness theorem for the fused in-block value sweep `opt`
  of RV8FV/Opt.lean (catalog #20 conditional-branch folding, #35 local CSE,
  #38 copy propagation, #39 constant folding, #40 constant propagation):
  on single-assignment blocks the optimized program runs literally
  identically to the original — same step, same trap, same halt — from
  every block-entry state.  Chain: `process_step_agree` (one processed
  instruction steps like the original), the recorded-state domain lemmas,
  `sweep_cons_step` (the sweep-state consistency invariant is preserved by
  execution), `tick_opt_agree` (one tick agrees), `opt_run_eq` (fuel
  induction), `opt_run_eq_entry` and `opt_sound_entry` (block-entry
  corollaries).
CEP:WHY: CEP&CC 54.2 requires the passes' soundness claims to be
  machine-checked.  This discharges the per-tick instruction-sweep
  agreement obligation that RV8FV/Opt.lean tracks, and lifts it to the
  program level.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: The theorems are stated for single-assignment blocks
  (`BlockSA`, implied by Spec's `WfSSA` via `wssa_blockSA`); on
  re-assignment the recorded environment may become stale and the sweep is
  not claimed sound.  Mid-block entry states are covered only under the
  invariant `RunInv` (consistent sweep state); the block-entry corollaries
  need only `iip = 0`.
CEP:ASSUMES: BlockSA/WfSSA (single assignment).  Calls clear the sweep
  state (register clobber).  Division is never folded, substituted, or
  common-subexpressed, so its trap class is preserved exactly.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/OptSound.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: `opt` is the per-block sweep of RV8FV/Opt.lean; the
  terminator half of the agreement is `optTerm_step_agree` (Opt.lean); the
  DCE proof reuses the same `EqExcept` machinery (DceProof.lean).
CEP:FV-AXIOMS: none.
-/

import RV8FV.Opt

namespace RV8FV

/-! ## Single assignment and basic facts. -/

/-- Block-level single assignment, in program order: no later instruction
    redefines an earlier definition. -/
def BlockSA : List Instr → Prop
  | [] => True
  | i :: rest =>
      (∀ j ∈ rest, ∀ d, i.defOf = some d → j.defOf ≠ some d) ∧ BlockSA rest

/-- The count of definitions of `d` in one block. -/
def blockDefCount (is : List Instr) (d : VReg) : Nat :=
  is.countP (fun i => i.defOf = some d)

/-- The program-wide definition count dominates every block's count:
    `countDefs` folds the per-block counts over all blocks, and the fold is
    monotone in the accumulator, so a member block's count is below the sum. -/
theorem foldl_count_ge_init (d : VReg) : ∀ (bs : List Block) (c : Nat),
    c ≤ bs.foldl (fun acc b => acc + b.instrs.countP (fun i => i.defOf = some d)) c
  | [], _ => Nat.le_refl _
  | b :: rest, c => by
    simp only [List.foldl_cons]
    exact Nat.le_trans (Nat.le_add_right _ _) (foldl_count_ge_init d rest _)

theorem foldl_count_ge (d : VReg) : ∀ (bs : List Block) (c : Nat) (bl : Block), bl ∈ bs →
    c + bl.instrs.countP (fun i => i.defOf = some d) ≤
      bs.foldl (fun acc b => acc + b.instrs.countP (fun i => i.defOf = some d)) c
  | [], _, _, hb => absurd hb (by simp)
  | b :: rest, c, bl, hb => by
    rcases List.mem_cons.mp hb with rfl | hmem
    · simp only [List.foldl_cons]
      exact foldl_count_ge_init d rest _
    · have ih := foldl_count_ge d rest (c + b.instrs.countP (fun i => i.defOf = some d)) bl
        hmem
      simp only [List.foldl_cons]
      omega

/-- Spec's whole-program definition count dominates a member block's count. -/
theorem countDefs_ge (p : Prog) (d : VReg) (bl : Block) (hb : bl ∈ p.blocks) :
    bl.instrs.countP (fun i => i.defOf = some d) ≤ p.countDefs d := by
  have h := foldl_count_ge d p.blocks 0 bl hb
  have hd : p.countDefs d
      = p.blocks.foldl (fun acc b => acc + b.instrs.countP (fun i => i.defOf = some d)) 0 :=
    rfl
  rw [hd]
  simpa using h

/-- Block-level single definition (the block-local form of `WfSSA`) implies
    block-level single assignment. -/
theorem blockWfSSA_blockSA : ∀ (is : List Instr),
    (∀ v, is.countP (fun i => i.defOf = some v) ≤ 1) → BlockSA is
  | [], _ => trivial
  | i :: rest, h => by
    refine ⟨?_, blockWfSSA_blockSA rest (fun v => ?_)⟩
    · intro j hj d hd1 hd2
      have h1 : 0 < rest.countP (fun i' => i'.defOf = some d) :=
        List.countP_pos_iff.mpr ⟨j, hj, by simp [hd2]⟩
      have htwo : 2 ≤ (i :: rest).countP (fun i' => i'.defOf = some d) := by
        rw [List.countP_cons_of_pos (by simp [hd1])]
        omega
      exact absurd (h d) (by omega)
    · have hv := h v
      by_cases hiv : i.defOf = some v
      · rw [List.countP_cons_of_pos (by simp [hiv])] at hv
        omega
      · rw [List.countP_cons_of_neg (by simp [hiv])] at hv
        exact hv

/-- Spec's whole-program `WfSSA` implies block-level single assignment. -/
theorem wssa_blockSA (p : Prog) (h : WfSSA p) : ∀ bl : Block, bl ∈ p.blocks →
    BlockSA bl.instrs := by
  intro bl hb
  refine blockWfSSA_blockSA _ (fun v => ?_)
  have hc := countDefs_ge p v bl hb
  have hle := h v
  omega

/-- The sweep-state consistency invariant at cursor `k` of block `is`. -/
def SweepConsAt (m : Machine) (is : List Instr) (k : Nat) : Prop :=
  EnvCons m (envAt is k) ∧ TabCons m (tblAt is k)

/-- Only `udiv` can fail; every other operation is total on words. -/
theorem binop_eval_some {o : Binop} {a b : Word} (ho : o ≠ .udiv) :
    ∃ w, o.eval a b = some w := by
  cases o with
  | udiv => exact absurd rfl ho
  | add => exact ⟨wadd a b, rfl⟩
  | sub => exact ⟨wsub a b, rfl⟩
  | mul => exact ⟨wmul a b, rfl⟩
  | and => exact ⟨wand a b, rfl⟩
  | or => exact ⟨wor a b, rfl⟩
  | xor => exact ⟨wxor a b, rfl⟩
  | shl => exact ⟨wshl a b, rfl⟩
  | lshr => exact ⟨wlshr a b, rfl⟩

/-- The processed instruction has the same destination as the original. -/
theorem process_defOf (env : CEnv) (tbl : CTab) (i : Instr) :
    (Instr.process env tbl i).defOf = i.defOf := by
  cases i with
  | binop d o a b =>
    simp only [Instr.process]
    cases hs_a : a.subst env <;> cases hs_b : b.subst env <;>
      (try split) <;> (try split) <;> (try split) <;> rfl
  | _ => rfl

/-! ## Sweep recurrence and indexing. -/

/-- The k-th swept instruction is the processed k-th instruction. -/
theorem sweepFrom_get (s : SweepState) : ∀ (is : List Instr) (k : Nat) (hk : k < is.length),
    (sweepFrom s is).1[k]? = some (Instr.process (sweepStateF s is k).1
      (sweepStateF s is k).2.1 (is[k]'hk))
  | [], k, hk => absurd hk (by simp)
  | i :: rest, 0, hk => by
      simp only [sweepFrom, List.getElem?_cons_zero]
      rfl
  | i :: rest, k + 1, hk => by
      have hr : k < rest.length := by simpa using hk
      have ih := sweepFrom_get (Instr.record s.1 s.2.1 s.2.2
        (Instr.process s.1 s.2.1 i)) rest k hr
      simp only [sweepFrom, List.getElem?_cons_succ]
      -- the sweep state of `i :: rest` at k+1 is the state of `rest` at k
      have hstate : sweepStateF (Instr.record s.1 s.2.1 s.2.2
          (Instr.process s.1 s.2.1 i)) rest k
          = sweepStateF s (i :: rest) (k + 1) := by
        show (sweepFrom _ (rest.take k)).2 = (sweepFrom s ((i :: rest).take (k + 1))).2
        rfl
      rw [List.getElem_cons_succ, ← hstate]
      exact ih

theorem optBlock_at (is : List Instr) (k : Nat) (hk : k < is.length) :
    (optBlock is)[k]? = some (Instr.process (envAt is k) (tblAt is k) is[k]) :=
  sweepFrom_get ([], [], []) is k hk

/-- The sweep state at `k+1` is the record of the processed `k`-th
    instruction over the state at `k`. -/
theorem sweepStateAt_succ (is : List Instr) (k : Nat) (hk : k < is.length) :
    sweepStateAt is (k + 1)
      = Instr.record (envAt is k) (tblAt is k) (allDefsAt is k)
          (Instr.process (envAt is k) (tblAt is k) is[k]) := by
  have htk : is.take (k + 1) = is.take k ++ [is[k]] := take_succ is k hk
  show (sweepFrom ([], [], []) (is.take (k + 1))).2 = _
  rw [htk]
  obtain ⟨_, h2⟩ := sweepFrom_append ([], [], []) (is.take k) [is[k]]
  rw [h2]
  simp only [allDefsAt, allDefsAtF, defsFrom_eq]
  rfl

theorem envAt_succ (is : List Instr) (k : Nat) (hk : k < is.length) :
    envAt is (k + 1)
      = (Instr.record (envAt is k) (tblAt is k) (allDefsAt is k)
          (Instr.process (envAt is k) (tblAt is k) is[k])).1 := by
  rw [← sweepStateAt_succ is k hk]
  rfl

theorem tblAt_succ (is : List Instr) (k : Nat) (hk : k < is.length) :
    tblAt is (k + 1)
      = (Instr.record (envAt is k) (tblAt is k) (allDefsAt is k)
          (Instr.process (envAt is k) (tblAt is k) is[k])).2.1 := by
  rw [← sweepStateAt_succ is k hk]
  rfl

theorem allDefsAt_succ (is : List Instr) (k : Nat) (hk : k < is.length) :
    allDefsAt is (k + 1)
      = (Instr.record (envAt is k) (tblAt is k) (allDefsAt is k)
          (Instr.process (envAt is k) (tblAt is k) is[k])).2.2 := by
  have htk : is.take (k + 1) = is.take k ++ [is[k]] := take_succ is k hk
  show defsFrom ([], [], []) (is.take (k + 1)) = _
  rw [htk, defsFrom_eq]
  obtain ⟨_, h2⟩ := sweepFrom_append ([], [], []) (is.take k) [is[k]]
  rw [h2]
  simp only [allDefsAt, allDefsAtF, defsFrom_eq]
  rfl

/-! ## Domains of the recorded state. -/

/-- Every environment key is a vreg defined earlier in the block, and every
    register an environment argument reads is defined earlier too. -/
theorem envAt_domains : ∀ (is : List Instr) (k : Nat) (x : VReg) (a : Arg),
    envLookup (envAt is k) x = some a → x ∈ allDefsAt is k
    ∧ (∀ r, r ∈ a.regsOf → r ∈ allDefsAt is k)
  | is, 0, x, a, hl => by
      exfalso
      have h0 : envAt is 0 = ([] : CEnv) := rfl
      rw [h0] at hl
      simp only [envLookup] at hl
      exact absurd hl (by simp)
  | is, k + 1, x, a, hl => by
      by_cases hkl : k < is.length
      · rw [allDefsAt_succ is k hkl]
        rw [envAt_succ is k hkl] at hl
        cases hi3 : Instr.process (envAt is k) (tblAt is k) is[k] with
        | nop =>
          rw [Instr.record]
          simp only [hi3, Instr.record] at hl
          exact envAt_domains is k x a hl
        | copy d b =>
          cases b with
          | imm w =>
            rw [Instr.record]
            simp [hi3, Instr.record, envLookup] at hl
            by_cases hxd : x = d
            · rw [ite_eq_left hxd] at hl
              have haw : a = Arg.imm w := by injection hl with h; exact h.symm
              cases haw
              refine ⟨?_, ?_⟩
              · simp only [hxd]
                exact List.mem_cons_self
              · intro r hr
                simp [Arg.regsOf] at hr
            · rw [ite_eq_right hxd] at hl
              obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                List.mem_cons_of_mem _ (h2 r hr)⟩
          | reg s =>
            rw [Instr.record]
            simp only [hi3, Instr.record] at hl
            by_cases hcond : (allDefsAt is k).contains s = true
            · have hs_mem : s ∈ allDefsAt is k := List.contains_iff_mem.mp hcond
              rw [ite_eq_left hcond] at hl ⊢
              simp [envLookup] at hl
              by_cases hxd : x = d
              · rw [ite_eq_left hxd] at hl
                have haw : a = Arg.reg s := by injection hl with h; exact h.symm
                cases haw
                refine ⟨?_, ?_⟩
                · simp only [hxd]
                  exact List.mem_cons_self
                · intro r hr
                  simp only [Arg.regsOf, List.mem_cons] at hr
                  rcases hr with rfl | hm
                  · exact List.mem_cons_of_mem _ hs_mem
                  · exact absurd hm (by simp)
              · rw [ite_eq_right hxd] at hl
                refine ⟨?_, ?_⟩
                · obtain ⟨h1, -⟩ := envAt_domains is k x a hl
                  exact List.mem_cons_of_mem _ h1
                · intro r hr
                  obtain ⟨-, h2⟩ := envAt_domains is k x a hl
                  exact List.mem_cons_of_mem _ (h2 r hr)
            · rw [ite_eq_right hcond] at hl ⊢
              obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                List.mem_cons_of_mem _ (h2 r hr)⟩
        | binop d o b1 b2 =>
          rw [Instr.record]
          simp only [hi3, Instr.record] at hl
          by_cases ho : o = .udiv
          · rw [ite_eq_left ho] at hl ⊢
            simp at hl
            obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
            exact ⟨List.mem_cons_of_mem _ h1, fun r hr => List.mem_cons_of_mem _ (h2 r hr)⟩
          · rw [ite_eq_right ho] at hl ⊢
            by_cases hargs : argsDefined (allDefsAt is k) (b1.regsOf ++ b2.regsOf) = true
            · rw [ite_eq_left hargs] at hl ⊢
              simp at hl
              obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr => List.mem_cons_of_mem _ (h2 r hr)⟩
            · rw [ite_eq_right hargs] at hl ⊢
              simp at hl
              obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr => List.mem_cons_of_mem _ (h2 r hr)⟩
        | load d b =>
          rw [Instr.record]
          simp only [hi3, Instr.record] at hl
          obtain ⟨h1, h2⟩ := envAt_domains is k x a hl
          exact ⟨List.mem_cons_of_mem _ h1, fun r hr => List.mem_cons_of_mem _ (h2 r hr)⟩
        | store b1 b2 =>
          rw [Instr.record]
          simp only [hi3, Instr.record] at hl
          exact envAt_domains is k x a hl
        | call d f args =>
          exfalso
          simp [hi3, Instr.record, envLookup] at hl
      · -- past the end of the block the sweep state is stationary
        have h1 : is.take (k + 1) = is := take_full is (k + 1) (by simp at hkl; omega)
        have h2 : is.take k = is := take_full is k (by simp at hkl; omega)
        have hEq1 : envAt is (k + 1) = envAt is k := by
          simp only [envAt, envAtF, sweepStateF]
          rw [h1, h2]
        have hEq2 : allDefsAt is (k + 1) = allDefsAt is k := by
          simp only [allDefsAt, allDefsAtF]
          rw [h1, h2]
        rw [hEq1] at hl
        rw [hEq2]
        exact envAt_domains is k x a hl

/-! ## Recorded-state shapes and freshness. -/

/-- The defined-set after recording either stays or gets the instruction's
    destination prepended. -/
theorem record_defs_cons (st : SweepState) (i3 : Instr) :
    (Instr.record st.1 st.2.1 st.2.2 i3).2.2 = st.2.2
    ∨ ∃ e, i3.defOf = some e
      ∧ (Instr.record st.1 st.2.1 st.2.2 i3).2.2 = e :: st.2.2 := by
  cases i3 with
  | nop => exact Or.inl rfl
  | copy e a =>
    cases a with
    | imm w => exact Or.inr ⟨e, rfl, rfl⟩
    | reg s =>
      simp only [Instr.record]
      split <;> exact Or.inr ⟨e, rfl, rfl⟩
  | binop e o b1 b2 =>
    simp only [Instr.record]
    (try split) <;> (try split) <;> exact Or.inr ⟨e, rfl, rfl⟩
  | load e a => exact Or.inr ⟨e, rfl, rfl⟩
  | store a v => exact Or.inl rfl
  | call e f args => exact Or.inr ⟨e, rfl, rfl⟩

/-- Everything in the defined-set is either inherited from the seed state or
    defined by some instruction of the swept block. -/
theorem defsFrom_mem : ∀ (is : List Instr) (s : SweepState) (x : VReg),
    x ∈ defsFrom s is → x ∈ s.2.2 ∨ x ∈ is.filterMap Instr.defOf
  | [], s, x, h => Or.inl h
  | i :: rest, s, x, h => by
      have hrec := defsFrom_mem rest
        (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)) x h
      rcases record_defs_cons s (Instr.process s.1 s.2.1 i) with hshape | ⟨e, hdef, hshape2⟩
      · rw [hshape] at hrec
        rcases hrec with hm | hm
        · exact Or.inl hm
        · obtain ⟨a, hamem, hadef⟩ := List.mem_filterMap.mp hm
          exact Or.inr (List.mem_filterMap.mpr ⟨a, List.mem_cons_of_mem _ hamem,
            hadef⟩)
      · rw [hshape2] at hrec
        rcases hrec with hm | hm
        · rcases List.mem_cons.mp hm with rfl | hm2
          · exact Or.inr (List.mem_filterMap.mpr ⟨i, List.mem_cons_self, by
              rw [← process_defOf s.1 s.2.1 i]; exact hdef⟩)
          · exact Or.inl hm2
        · obtain ⟨a, hamem, hadef⟩ := List.mem_filterMap.mp hm
          exact Or.inr (List.mem_filterMap.mpr ⟨a, List.mem_cons_of_mem _ hamem,
            hadef⟩)

/-- Every table key is a vreg defined earlier in the block, and every
    register a table entry's operands read is defined earlier too. -/
theorem tblAt_domains : ∀ (is : List Instr) (k : Nat) (o : Binop) (b1 b2 : Arg) (d : VReg),
    tabLookup (tblAt is k) o b1 b2 = some d → d ∈ allDefsAt is k
    ∧ (∀ r, r ∈ b1.regsOf ++ b2.regsOf → r ∈ allDefsAt is k)
  | is, 0, o, b1, b2, d, hl => by
      exfalso
      have h0 : tblAt is 0 = ([] : CTab) := rfl
      rw [h0] at hl
      simp [tabLookup] at hl
  | is, k + 1, o, b1, b2, d, hl => by
      by_cases hkl : k < is.length
      · rw [allDefsAt_succ is k hkl]
        rw [tblAt_succ is k hkl] at hl
        cases hi3 : Instr.process (envAt is k) (tblAt is k) is[k] with
        | nop =>
          simp only [hi3, Instr.record] at hl ⊢
          exact tblAt_domains is k o b1 b2 d hl
        | copy e a =>
          cases a with
          | imm w =>
            simp only [hi3, Instr.record] at hl ⊢
            obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
            exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
              List.mem_cons_of_mem _ (h2 r hr)⟩
          | reg s =>
            simp only [hi3, Instr.record] at hl ⊢
            by_cases hcond : (allDefsAt is k).contains s = true
            · rw [ite_eq_left hcond] at hl ⊢
              obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                List.mem_cons_of_mem _ (h2 r hr)⟩
            · rw [ite_eq_right hcond] at hl ⊢
              obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                List.mem_cons_of_mem _ (h2 r hr)⟩
        | binop e ob c1 c2 =>
          simp only [hi3, Instr.record] at hl ⊢
          by_cases hou : ob = .udiv
          · rw [ite_eq_left hou] at hl ⊢
            obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
            exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
              List.mem_cons_of_mem _ (h2 r hr)⟩
          · rw [ite_eq_right hou] at hl ⊢
            by_cases hargs : argsDefined (allDefsAt is k) (c1.regsOf ++ c2.regsOf) = true
            · rw [ite_eq_left hargs] at hl ⊢
              simp only [tabLookup] at hl
              by_cases hmatch : (o = ob && b1 = c1 && b2 = c2) = true
              · rw [ite_eq_left hmatch] at hl
                have hed : e = d := by injection hl with h
                subst hed
                have hexp : (o = ob ∧ b1 = c1) ∧ b2 = c2 := by simpa using hmatch
                obtain ⟨⟨hob, hc1⟩, hc2⟩ := hexp
                refine ⟨List.mem_cons_self, ?_⟩
                intro r hr
                rw [hc1, hc2] at hr
                exact List.mem_cons_of_mem _
                  (List.contains_iff_mem.mp (List.all_eq_true.mp hargs r hr))
              · rw [ite_eq_right hmatch] at hl
                obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
                exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                  List.mem_cons_of_mem _ (h2 r hr)⟩
            · rw [ite_eq_right hargs] at hl ⊢
              obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
              exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
                List.mem_cons_of_mem _ (h2 r hr)⟩
        | load e a =>
          simp only [hi3, Instr.record] at hl ⊢
          obtain ⟨h1, h2⟩ := tblAt_domains is k o b1 b2 d hl
          exact ⟨List.mem_cons_of_mem _ h1, fun r hr =>
            List.mem_cons_of_mem _ (h2 r hr)⟩
        | store a1 a2 =>
          simp only [hi3, Instr.record] at hl ⊢
          exact tblAt_domains is k o b1 b2 d hl
        | call e f as =>
          exfalso
          simp [hi3, Instr.record, tabLookup] at hl
      · -- past the end of the block the sweep state is stationary
        have h1 : is.take (k + 1) = is := take_full is (k + 1) (by simp at hkl; omega)
        have h2 : is.take k = is := take_full is k (by simp at hkl; omega)
        have hEq1 : tblAt is (k + 1) = tblAt is k := by
          simp only [tblAt, tblAtF, sweepStateF]
          rw [h1, h2]
        have hEq2 : allDefsAt is (k + 1) = allDefsAt is k := by
          simp only [allDefsAt, allDefsAtF]
          rw [h1, h2]
        rw [hEq1] at hl
        rw [hEq2]
        exact tblAt_domains is k o b1 b2 d hl

/-! ## The per-instruction agreement law. -/

/-- The CSE branch of the binop processing steps like the original binop:
    a table hit reproduces the computed value by table consistency, and the
    trap classes agree (a tabled op is never `udiv`, so it never yields
    `divZero`). -/
theorem step_ab_branch (m : Machine) (tbl : CTab) (d : VReg) (o : Binop)
    (a'' b'' a b : Arg) (hea : a''.eval m = a.eval m) (heb : b''.eval m = b.eval m)
    (ho : o ≠ .udiv) (ht : TabCons m tbl) :
    stepInstr m (match tabLookup tbl o a'' b'' with
      | some d2 => Instr.copy d (Arg.reg d2)
      | none => Instr.binop d o a'' b'')
      = stepInstr m (Instr.binop d o a b) := by
  cases htl : tabLookup tbl o a'' b'' with
  | none => simp only [stepInstr, hea, heb]
  | some d2 =>
    have htab := ht o a'' b'' d2 htl
    simp only [hea, heb] at htab
    simp only [stepInstr]
    rw [show (Arg.reg d2).eval m = m.regs d2 from rfl, htab]
    cases hx : a.eval m with
    | none => simp only [binopEvalO]
    | some va =>
      cases hy : b.eval m with
      | none => simp only [binopEvalO]
      | some vb =>
        simp only [binopEvalO]
        obtain ⟨w, hw⟩ := binop_eval_some (a := va) (b := vb) ho
        rw [hw]

/-- **The per-instruction agreement law**: one sweep-processed instruction
    steps exactly like the original instruction.  Substitution preserves
    operand values (`EnvCons`), a CSE hit reproduces the computed value
    (`TabCons`), folding computes the very same value (`foldEval`), and
    division is never folded, substituted, or common-subexpressed, so its
    `divZero` trap and the `undefRead` traps are preserved exactly. -/
theorem process_step_agree (m : Machine) (env : CEnv) (tbl : CTab)
    (he : EnvCons m env) (ht : TabCons m tbl) (i : Instr) :
    stepInstr m (Instr.process env tbl i) = stepInstr m i := by
  cases i with
  | nop => rfl
  | copy d a =>
      simp only [Instr.process, stepInstr, Arg.eval_subst m env he a]
  | binop d o a b =>
      have hcongr : ∀ (o1 o2 : Binop) (a1 b1 a2 b2 : Arg), o1 = o2 →
          a1.eval m = a2.eval m → b1.eval m = b2.eval m →
          stepInstr m (Instr.binop d o1 a1 b1) = stepInstr m (Instr.binop d o2 a2 b2) := by
        intro o1 o2 a1 b1 a2 b2 ho h1 h2
        simp only [stepInstr, ho, h1, h2]
      simp only [Instr.process]
      cases hs_a : a.subst env with
      | imm x =>
        have hae : a.eval m = some x := by
          rw [← Arg.eval_subst m env he a, hs_a]
          rfl
        cases hs_b : b.subst env with
        | imm y =>
          have hbe : b.eval m = some y := by
            rw [← Arg.eval_subst m env he b, hs_b]
            rfl
          dsimp only
          by_cases hdv : o = .udiv ∧ y = 0
          · rw [ite_eq_left (by simpa using hdv)]
            obtain ⟨ho, hy⟩ := hdv
            exact hcongr .udiv o (.imm x) (.imm y) a b ho.symm hae.symm hbe.symm
          · rw [ite_eq_right (by simpa using hdv)]
            cases hfo : foldEval o x y with
            | none =>
              exact hcongr o o (.imm x) (.imm y) a b rfl hae.symm hbe.symm
            | some r =>
              have hO : o.eval x y = some r := hfo
              have hR : stepInstr m (Instr.binop d o a b)
                  = Except.ok (setReg m d (some r)) := by
                simp only [stepInstr, hae, hbe, hO]
              rw [show stepInstr m (Instr.copy d (Arg.imm r))
                    = Except.ok (setReg m d (some r)) from rfl, hR]
        | reg y =>
          have hbe : b.eval m = m.regs y := by
            rw [← Arg.eval_subst m env he b, hs_b]
            rfl
          dsimp only
          by_cases ho : o = .udiv
          · rw [ite_eq_left ho]
            exact hcongr .udiv o (.imm x) (.reg y) a b ho.symm hae.symm hbe.symm
          · rw [ite_eq_right ho]
            exact step_ab_branch m tbl d o (Arg.imm x) (Arg.reg y) a b
              hae.symm hbe.symm ho ht
      | reg x =>
        have hae : a.eval m = m.regs x := by
          rw [← Arg.eval_subst m env he a, hs_a]
          rfl
        cases hs_b : b.subst env with
        | imm y =>
          have hbe : b.eval m = some y := by
            rw [← Arg.eval_subst m env he b, hs_b]
            rfl
          dsimp only
          by_cases ho : o = .udiv
          · rw [ite_eq_left ho]
            exact hcongr .udiv o (.reg x) (.imm y) a b ho.symm hae.symm hbe.symm
          · rw [ite_eq_right ho]
            exact step_ab_branch m tbl d o (Arg.reg x) (Arg.imm y) a b
              hae.symm hbe.symm ho ht
        | reg y =>
          have hbe : b.eval m = m.regs y := by
            rw [← Arg.eval_subst m env he b, hs_b]
            rfl
          dsimp only
          by_cases ho : o = .udiv
          · rw [ite_eq_left ho]
            exact hcongr .udiv o (.reg x) (.reg y) a b ho.symm hae.symm hbe.symm
          · rw [ite_eq_right ho]
            exact step_ab_branch m tbl d o (Arg.reg x) (Arg.reg y) a b
              hae.symm hbe.symm ho ht
  | load d a =>
      simp only [Instr.process, stepInstr, Arg.eval_subst m env he a]
  | store a v =>
      simp only [Instr.process, stepInstr, Arg.eval_subst m env he a,
        Arg.eval_subst m env he v]
  | call d f args =>
      simp only [Instr.process, stepInstr, Arg.evalList_subst m env he args]

end RV8FV
