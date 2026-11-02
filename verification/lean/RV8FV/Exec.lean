/-
CEP:FILE: verification/lean/RV8FV/Exec.lean
CEP:WHAT: Execution support: whole-list instruction execution, machine update
  pointwise-agreement lemmas, and operand substitution with its evaluation
  agreement lemma (the core step used by every value-rewriting pass).
CEP:WHY: CEP&CC 54.2.1 transformation equivalence proofs factor through these
  lemmas; keeping them here separates machinery from pass statements.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: none; every lemma is machine-checked.
CEP:ASSUMES: see Machine.lean.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Exec.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Machine.lean.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Machine

namespace RV8FV

/-! ## Machine update lemmas. -/

theorem setReg_self (m : Machine) (d : VReg) (r : Option Word) :
    (setReg m d r).regs d = r := by simp [setReg]

theorem setReg_ne (m : Machine) (d e : VReg) (r : Option Word) (h : d ≠ e) :
    (setReg m e r).regs d = m.regs d := by simp [setReg, h]

theorem setReg_eq_of_pointwise {m m' : Machine} {d : VReg} {r : Option Word}
    (h : Machine.Eq m m') : Machine.Eq (setReg m d r) (setReg m' d r) := by
  obtain ⟨hr, hm, hpc, hiip⟩ := h
  refine ⟨?_, ?_, hpc, hiip⟩
  · intro v
    by_cases hv : v = d
    · simp [setReg, hv]
    · simp [setReg, hv, hr v]
  · intro w
    exact hm w

theorem setMem_self (m : Machine) (a w : Word) :
    (setMem m a w).mem a = w := by simp [setMem]

theorem setMem_ne (m : Machine) (a b : Word) (w : Word) (h : a ≠ b) :
    (setMem m b w).mem a = m.mem a := by simp [setMem, h]

theorem setMem_eq_of_pointwise {m m' : Machine} {a w : Word}
    (h : Machine.Eq m m') : Machine.Eq (setMem m a w) (setMem m' a w) := by
  obtain ⟨hr, hm, hpc, hiip⟩ := h
  refine ⟨hr, ?_, hpc, hiip⟩
  intro w'
  by_cases hw : w' = a
  · simp [setMem, hw]
  · simp [setMem, hw, hm w']

/-! ## Operand substitution. -/

/-- Constant/copy environment: vreg → the value it provably holds. -/
abbrev CEnv := List (VReg × Arg)

/-- Common-subexpression table: (op, arg1, arg2) → vreg holding the result. -/
abbrev CTab := List (Binop × Arg × Arg × VReg)

def envLookup : CEnv → VReg → Option Arg
  | [], _ => none
  | (y, a) :: rest, x => if x = y then some a else envLookup rest x

def Arg.subst (env : CEnv) : Arg → Arg
  | .imm w => .imm w
  | .reg x => match envLookup env x with | some a => a | none => .reg x

def Instr.subst (env : CEnv) : Instr → Instr
  | .nop => .nop
  | .copy d a => .copy d (a.subst env)
  | .binop d o a b => .binop d o (a.subst env) (b.subst env)
  | .load d a => .load d (a.subst env)
  | .store a b => .store (a.subst env) (b.subst env)
  | .call d f args => .call d f (args.map (Arg.subst env))

def Term.subst (env : CEnv) : Term → Term
  | .jmp l => .jmp l
  | .br a l r => .br (a.subst env) l r
  | .ret a => .ret (match a with
    | none => none
    | some c => some (c.subst env))

/-- Environment consistency: every entry's key holds exactly the entry's value. -/
def EnvCons (m : Machine) (env : CEnv) : Prop :=
  ∀ x a, envLookup env x = some a → m.regs x = a.eval m

def tabLookup : CTab → Binop → Arg → Arg → Option VReg
  | [], _, _, _ => none
  | (o', x', y', d') :: rest, o, x, y =>
    if o = o' && x = x' && y = y' then some d' else tabLookup rest o x y

/-- Evaluate a binary operation over possibly-undefined operand values:
    `none` when either operand is undefined or the operation traps. -/
def binopEvalO (o : Binop) (ma mb : Option Word) : Option Word :=
  match ma, mb with
  | some x, some y => o.eval x y
  | _, _ => none

/-- Table consistency: every entry's vreg holds exactly the entry's computed value. -/
def TabCons (m : Machine) (tbl : CTab) : Prop :=
  ∀ o a b d, tabLookup tbl o a b = some d → m.regs d = binopEvalO o (a.eval m) (b.eval m)

/-- Argument evaluation is invariant under substitution by a consistent env. -/
theorem Arg.eval_subst (m : Machine) (env : CEnv) (h : EnvCons m env) (a : Arg) :
    (a.subst env).eval m = a.eval m := by
  cases a with
  | imm w => rfl
  | reg x =>
    simp only [Arg.subst]
    cases hl : envLookup env x with
    | none => rfl
    | some a' => simp [hl]; exact (h x a' hl).symm

/-- List argument evaluation is invariant under consistent substitution. -/
theorem Arg.evalList_subst (m : Machine) (env : CEnv) (h : EnvCons m env) :
    ∀ (args : List Arg), Arg.evalList (args.map (Arg.subst env)) m = Arg.evalList args m
  | [] => rfl
  | a :: rest => by
      simp only [List.map_cons, Arg.evalList]
      rw [Arg.eval_subst m env h a]
      rw [Arg.evalList_subst m env h rest]

/-! ## Useful-work cost model. -/

/-- A nop is the DCE placeholder: zero useful work. Every real instruction,
    including a block terminator, counts as one. -/
def instrNn : Instr → Nat
  | .nop => 0
  | _ => 1

/-- The useful-work weight of the instruction (or terminator) at the cursor. -/
def curNn (p : Prog) (m : Machine) : Nat :=
  match p.block? m.pc with
  | none => 0
  | some bl => match bl.instrs[m.iip]? with
    | some i => instrNn i
    | none => 1

/-- Useful work performed within `n` ticks: the count of non-nop instructions
    executed. This is the dynamic cost the proved passes never increase. -/
def ucount : Nat → Prog → Machine → Nat
  | 0, _, _ => 0
  | n+1, p, m =>
    match tick p m with
    | .next m' => curNn p m + ucount n p m'
    | .stuck _ => curNn p m
    | .halt _ _ => curNn p m

theorem Machine.Eq_symm {m m' : Machine} (h : Machine.Eq m m') : Machine.Eq m' m :=
  ⟨fun v => (h.1 v).symm, fun w => (h.2.1 w).symm, h.2.2.1.symm, h.2.2.2.symm⟩

theorem getElem?_map' {α β} (f : α → β) : ∀ (xs : List α) (k : Nat),
    (xs.map f)[k]? = (xs[k]?).map f
  | [], _ => rfl
  | _ :: _, 0 => rfl
  | _ :: xs, k + 1 => getElem?_map' f xs k


end RV8FV
