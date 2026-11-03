/-
CEP:FILE: verification/lean/RV8FV/Opt.lean
CEP:WHAT: Formal specification of the fused in-block value-optimization sweep
  implementing catalog passes #20 (conditional branch folding), #35 (local CSE),
  #38 (copy propagation), #39 (constant folding), and #40 (constant propagation):
  one forward sweep per block with a substitution environment, a
  common-subexpression table, and a defined-before-use guard set. Division is
  never common-subexpressed (it can trap), and folding a division by a zero
  immediate is refused.
CEP:WHY: The user's design spec requires these passes to be mathematically
  defined. This file is that definition. The soundness proof obligations for
  the sweep are stated in the Catalog and are the tracked FV-0 upgrade path.
CEP:CLASS: FV-1
CEP:STATUS: partial
CEP:FAILURE: The sweep is restricted to one block at a time; cross-block value
  propagation needs dominance information and is not claimed here.
CEP:ASSUMES: WfSSA (single definition per vreg). Calls clear the environment and
  table because they clobber registers. Environment and table entries are
  recorded only when their source registers are defined earlier in the block.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Opt.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: Machine-checked: length preservation, append composition,
  defined-set monotonicity, and the #20 terminator agreement law
  (`termStep` semantics, `tick_term_agree` bridge, `optTerm_step_agree`
  per-step agreement under a consistent environment, `tick_of_term_replace`
  corollary).  Still open (tracked FV-1 obligations): the per-tick execution
  agreement of the *instruction* sweep (`Instr.process`) and the end-to-end
  SemEquiv theorem for the full sweep; the proved subset (constant folding,
  RV8FV/Fold.lean; dead immediate-load elimination, RV8FV/Dce.lean) carries
  the program-level FV-0 theorems.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Exec

namespace RV8FV

/-! ## Sweep state. -/

/-- Sweep state: substitution env, CSE table, and vregs defined so far in the block. -/
abbrev SweepState := CEnv × CTab × List VReg

/-- Guard: every vreg in the list is defined earlier in the block. -/
def argsDefined (allDefs : List VReg) (xs : List VReg) : Bool :=
  xs.all (fun x => allDefs.contains x)

/-! ## The sweep step: substitute, fold, common-subexpress. -/

/-- Process one instruction: substitute known values into operands (#40, #38),
    fold constant operations (#39), and rewrite a repeated pure computation to a
    copy of the earlier result (#35). Division by a zero immediate is never
    folded, and division is never common-subexpressed: both can trap, and the
    trap class must be preserved exactly. -/
def Instr.process (env : CEnv) (tbl : CTab) : Instr → Instr
  | .nop => .nop
  | .copy d a => .copy d (a.subst env)
  | .binop d o a b =>
    let a' := a.subst env
    let b' := b.subst env
    match a', b' with
    | .imm x, .imm y =>
      if o = .udiv && y = 0 then .binop d .udiv (.imm x) (.imm y)
      else match foldEval o x y with
        | some r => .copy d (.imm r)
        | none => .binop d o (.imm x) (.imm y)
    | a'', b'' =>
      if o = .udiv then .binop d .udiv a'' b''
      else match tabLookup tbl o a'' b'' with
        | some d2 => .copy d (.reg d2)
        | none => .binop d o a'' b''
  | .load d a => .load d (a.subst env)
  | .store a b => .store (a.subst env) (b.subst env)
  | .call d f args => .call d f (args.map (Arg.subst env))

/-- Record the processed instruction into the sweep state. Calls clear the env
    and table (register clobber). Entries are recorded only when every source
    register was defined earlier in the block; the destination is always added
    to the defined set. -/
def Instr.record (env : CEnv) (tbl : CTab) (allDefs : List VReg) : Instr → SweepState
  | .nop => (env, tbl, allDefs)
  | .copy d (.imm w) => ((d, .imm w) :: env, tbl, d :: allDefs)
  | .copy d (.reg s) =>
    if allDefs.contains s then ((d, .reg s) :: env, tbl, d :: allDefs)
    else (env, tbl, d :: allDefs)
  | .binop d o a b =>
    if o = .udiv then (env, tbl, d :: allDefs)
    else if argsDefined allDefs (a.regsOf ++ b.regsOf) then
      (env, (o, a, b, d) :: tbl, d :: allDefs)
    else (env, tbl, d :: allDefs)
  | .load d _ => (env, tbl, d :: allDefs)
  | .store _ _ => (env, tbl, allDefs)
  | .call d _ _ => ([], [], d :: allDefs)

/-- Forward sweep: returns the rewritten instruction list and the final state. -/
def sweepFrom (s : SweepState) : List Instr → List Instr × SweepState
  | [] => ([], s)
  | i :: rest =>
    let i3 := Instr.process s.1 s.2.1 i
    let s' := Instr.record s.1 s.2.1 s.2.2 i3
    let (rest', s'') := sweepFrom s' rest
    (i3 :: rest', s'')

def optBlock (is : List Instr) : List Instr := (sweepFrom ([], [], []) is).1

/-- Sweep state in force at position `k`, sweeping `is` from base state `s`. -/
def sweepStateF (s : SweepState) (is : List Instr) (k : Nat) : SweepState :=
  (sweepFrom s (is.take k)).2

def envAtF (s : SweepState) (is : List Instr) (k : Nat) : CEnv := (sweepStateF s is k).1
def tblAtF (s : SweepState) (is : List Instr) (k : Nat) : CTab := (sweepStateF s is k).2.1

/-- The defined-set after sweeping `is` from `s`, defined directly by recursion
    (equal to `(sweepFrom s is).2.2` by `defsFrom_eq`). -/
def defsFrom (s : SweepState) : List Instr → List VReg
  | [] => s.2.2
  | i :: rest => defsFrom (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)) rest

def allDefsAtF (s : SweepState) (is : List Instr) (k : Nat) : List VReg :=
  defsFrom s (is.take k)

/-- The sweep state at position `k` of a block swept from fresh state. -/
def sweepStateAt (is : List Instr) (k : Nat) : SweepState := sweepStateF ([], [], []) is k
def envAt (is : List Instr) (k : Nat) : CEnv := envAtF ([], [], []) is k
def tblAt (is : List Instr) (k : Nat) : CTab := tblAtF ([], [], []) is k
def allDefsAt (is : List Instr) (k : Nat) : List VReg := allDefsAtF ([], [], []) is k

/-- Terminator optimization: substitute known values, then fold a constant
    conditional branch into a jump (#20). -/
def optTerm (env : CEnv) : Term → Term
  | .jmp l => .jmp l
  | .br a l r =>
    match a.subst env with
    | .imm w => .jmp (if w = 0 then r else l)
    | a' => .br a' l r
  | .ret a =>
    .ret (match a with
      | none => none
      | some c => some (c.subst env))

/-- The whole-program optimization: the sweep per block, terminator optimization
    with the full-block environment. -/
def opt (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl =>
    { instrs := optBlock bl.instrs
      term := optTerm (envAt bl.instrs bl.instrs.length) bl.term }) }

/-! ## Terminator execution and the #20 agreement law (machine-checked). -/

/-- One-step execution of a terminator in a program with `L` blocks: exactly
    the terminator branch of `tick` (jump, branch-on-word, return). -/
def termStep (L : Nat) (m : Machine) : Term → Step
  | .jmp l => if l < L then .next { m with pc := l, iip := 0 } else .stuck .badTarget
  | .br c l r =>
    match c.eval m with
    | none => .stuck .undefRead
    | some w =>
      let l' := if w = 0 then r else l
      if l' < L then .next { m with pc := l', iip := 0 } else .stuck .badTarget
  | .ret a =>
    match a with
    | none => .halt m none
    | some c => match c.eval m with
      | none => .stuck .undefRead
      | some w => .halt m (some w)

/-- A tick at the end of a block is the term step of that block's terminator:
    `tick` executes the terminator exactly when the cursor is past the last
    instruction. -/
theorem tick_term_agree (p : Prog) (m : Machine) (bl : Block)
    (hbl : p.block? m.pc = some bl) (hiip : m.iip = bl.instrs.length) :
    tick p m = termStep p.blocks.length m bl.term := by
  simp only [tick, hbl]
  rw [hiip]
  simp
  cases bl.term <;> rfl

/-- **The #20 agreement law.**  Substituting operands by a consistent
    environment and folding a constant conditional branch into a jump
    preserves one-step terminator execution *literally*: same successor
    machine, same trap class, same halt value. -/
theorem optTerm_step_agree (env : CEnv) (t : Term) (L : Nat) (m : Machine)
    (h : EnvCons m env) :
    termStep L m (optTerm env t) = termStep L m t := by
  cases t with
  | jmp l => rfl
  | br a l r =>
    simp only [optTerm]
    cases hs : a.subst env with
    | imm w =>
      have hae : a.eval m = some w := by
        rw [← Arg.eval_subst m env h a, hs]
        rfl
      simp only [termStep, hae]
    | reg x =>
      have hae : a.eval m = m.regs x := by
        rw [← Arg.eval_subst m env h a, hs]
        rfl
      simp only [termStep, hae]
      rfl
  | ret a =>
    cases a with
    | none => rfl
    | some c =>
      simp only [optTerm, termStep]
      rw [Arg.eval_subst m env h c]

/-- Replacing one block's terminator by its `optTerm` version (with a
    consistent environment) leaves the tick literally unchanged: same
    successor machine, same trap, same halt. -/
theorem tick_of_term_replace (p p' : Prog) (m : Machine) (bl bl' : Block)
    (hbl : p.block? m.pc = some bl) (hbl' : p'.block? m.pc = some bl')
    (hiip : m.iip = bl.instrs.length) (hinstr : bl'.instrs = bl.instrs)
    (hlen : p'.blocks.length = p.blocks.length)
    (hterm : bl'.term = optTerm (envAt bl.instrs bl.instrs.length) bl.term)
    (henv : EnvCons m (envAt bl.instrs bl.instrs.length)) :
    tick p' m = tick p m := by
  rw [tick_term_agree p' m bl' hbl' (by rw [hinstr]; exact hiip)]
  rw [hterm, optTerm_step_agree _ _ _ m henv, hlen,
    ← tick_term_agree p m bl hbl hiip]

/-! ## Structural lemmas (proved). -/

theorem take_full (is : List Instr) : ∀ n, is.length ≤ n → is.take n = is
  | 0, h => by cases is with
    | nil => rfl
    | cons x xs => simp at h
  | n + 1, h => by
      cases is with
      | nil => rfl
      | cons x xs =>
        have hx : xs.length ≤ n := by simp at h; omega
        have e1 : (x :: xs).take (n + 1) = x :: xs.take n := rfl
        rw [e1, take_full xs n hx]

theorem take_succ (xs : List Instr) (k : Nat) (h : k < xs.length) :
    xs.take (k + 1) = xs.take k ++ [xs[k]] := by
  induction xs generalizing k with
  | nil => simp at h
  | cons x rest ih =>
    cases k with
    | zero => rfl
    | succ k' =>
      have hk : k' < rest.length := by simp at h; omega
      have e1 : (x :: rest).take (k' + 1 + 1) = x :: rest.take (k' + 1) := rfl
      have e2 : (x :: rest).take (k' + 1) = x :: rest.take k' := rfl
      have e3 : (x :: rest)[k' + 1] = rest[k'] := rfl
      rw [e1, e2, e3, ih k' hk]
      rfl

theorem sweepFrom_length : ∀ (is : List Instr) (s : SweepState),
    (sweepFrom s is).1.length = is.length
  | [], _ => rfl
  | i :: rest, s => by
      have h := sweepFrom_length rest
        (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i))
      simp only [sweepFrom, List.length_cons]
      rw [h]

theorem optBlock_length (is : List Instr) : (optBlock is).length = is.length :=
  sweepFrom_length is ([], [], [])

/-- Sweeping `a ++ b` sweeps `a`, then sweeps `b` from the resulting state. -/
theorem sweepFrom_append : ∀ (s : SweepState) (a b : List Instr),
    (sweepFrom s (a ++ b)).1 = (sweepFrom s a).1 ++ (sweepFrom (sweepFrom s a).2 b).1
    ∧ (sweepFrom s (a ++ b)).2 = (sweepFrom (sweepFrom s a).2 b).2
  | s, [], b => by simp [sweepFrom]
  | s, i :: rest, b => by
      obtain ⟨h1, h2⟩ := sweepFrom_append
        (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)) rest b
      simp [sweepFrom, h1, h2]

/-- The record step either keeps the defined-set or prepends one vreg to it. -/
theorem recordDefs_keeps (st : SweepState) (i3 : Instr) :
    (Instr.record st.1 st.2.1 st.2.2 i3).2.2 = st.2.2
    ∨ ∃ d, (Instr.record st.1 st.2.1 st.2.2 i3).2.2 = d :: st.2.2 := by
  cases i3 with
  | nop => exact Or.inl rfl
  | copy d a =>
    cases a with
    | imm w => exact Or.inr ⟨d, rfl⟩
    | reg s =>
      by_cases hs : st.2.2.contains s = true
      · exact Or.inr ⟨d, by simp only [Instr.record, ite_eq_left hs]⟩
      · exact Or.inr ⟨d, by simp only [Instr.record, ite_eq_right hs]⟩
  | binop d o a b =>
    by_cases ho : o = .udiv
    · exact Or.inr ⟨d, by simp only [Instr.record, ite_eq_left ho]⟩
    · by_cases hg : argsDefined st.2.2 (a.regsOf ++ b.regsOf) = true
      · exact Or.inr ⟨d, by simp only [Instr.record, ite_eq_right ho, ite_eq_left hg]⟩
      · exact Or.inr ⟨d, by simp only [Instr.record, ite_eq_right ho, ite_eq_right hg]⟩
  | load d a => exact Or.inr ⟨d, rfl⟩
  | store a b => exact Or.inl rfl
  | call d f args => exact Or.inr ⟨d, rfl⟩

theorem defsFrom_eq : ∀ (is : List Instr) (s : SweepState),
    defsFrom s is = (sweepFrom s is).2.2.2
  | [], _ => rfl
  | i :: rest, s => by
      simp only [defsFrom, sweepFrom]
      exact defsFrom_eq rest (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i))

/-- The defined-set only grows along the sweep. -/
theorem defsFrom_ge_initial : ∀ (is : List Instr) (s : SweepState) (x : VReg),
    (s.2.2).contains x → (defsFrom s is).contains x
  | [], _, _, h => h
  | i :: rest, s, x, h => by
      have hseed : ((Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)).2.2).contains x := by
        rcases recordDefs_keeps s (Instr.process s.1 s.2.1 i) with hhe | ⟨d, hhe⟩
        · rw [hhe]; exact h
        · rw [hhe]
          simp only [List.contains_cons, Bool.or_eq_true, beq_iff_eq]
          exact Or.inr h
      simp only [defsFrom]
      exact defsFrom_ge_initial rest
        (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)) x hseed

/-- The defined-set is preserved when sweeping a suffix. -/
theorem defsFrom_append_ge (s : SweepState) : ∀ (a b : List Instr) (x : VReg),
    (defsFrom s a).contains x → (defsFrom s (a ++ b)).contains x
  | [], b, x, h => by
      rw [List.nil_append]
      exact defsFrom_ge_initial b s x h
  | i :: rest, b, x, h => by
      have hrec := defsFrom_append_ge
        (Instr.record s.1 s.2.1 s.2.2 (Instr.process s.1 s.2.1 i)) rest b x h
      exact hrec

/-- The defined-set grows monotonically along the sweep. -/
theorem allDefsAtF_mono (s : SweepState) (is : List Instr) :
    ∀ k x, (allDefsAtF s is k).contains x → (allDefsAtF s is (k + 1)).contains x := by
  intro k x h
  by_cases hk : k < is.length
  · have htk : is.take (k + 1) = is.take k ++ [is[k]] := take_succ is k hk
    simp only [allDefsAtF]
    rw [htk]
    exact defsFrom_append_ge s (is.take k) [is[k]] x h
  · have h1 : is.take (k + 1) = is := take_full is (k + 1) (by simp at hk; omega)
    have h2 : is.take k = is := take_full is k (by simp at hk; omega)
    simp only [allDefsAtF, h1, h2] at h ⊢
    exact h

end RV8FV
