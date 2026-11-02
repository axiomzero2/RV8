/-
CEP:FILE: verification/lean/RV8FV/Machine.lean
CEP:WHAT: Normative machine semantics: register file with fail-closed undefined reads,
  total word memory, trap classes, one-instruction stepping over blocks, fuel-bounded
  execution, observable behavior, and strict observational equivalence
  (equality of the outcome at every fuel).
CEP:WHY: CEP&CC 54.2.1 requires compiler transformation equivalence proofs against a
  model that matches operational reality. Undefined reads and division by zero are
  observable traps, not silent values; memory writes are observable; the returned
  value is observable. Register cells of dead values are deliberately NOT
  observable, which is exactly what makes dead-code elimination sound.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A transformation that changes the returned value, the final memory, the
  trap class, or the termination/fuel profile is inequivalent and rejected.
CEP:ASSUMES: Loads never fault: memory is a total function. Consequently no proved
  pass removes or introduces a load; loads are treated as effectful. Calls clobber
  all registers except the destination (which receives an opaque value) and are
  never removed; no proved pass exploits call purity.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Machine.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: Rust-side bridge: crates/rv8_backend end-to-end suites execute the
  same fixture programs; equivalence at the model level plus the deterministic
  differential suites form the runtime bridge required by 54.6.2.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Spec

namespace RV8FV

/-! ## Trap classes. -/

/-- Trap classes. Payload-free on purpose: any undefined read is the same observable
    fault class, any division by zero is the same fault class. -/
inductive Trap where
  | undefRead   -- read of a register that has no value on this path
  | divZero     -- division by zero
  | badBlock    -- program counter outside the block list
  | badTarget   -- jump or branch target outside the block list
  deriving DecidableEq, Repr

/-! ## Machine state. -/

/-- The abstract machine: register file (partial), total memory, block program
    counter, and instruction position inside the current block. -/
structure Machine where
  regs : VReg → Option Word
  mem  : Word → Word
  pc   : Nat
  iip  : Nat

/-- Pointwise machine equality. Deliberately a Prop over pointwise agreements:
    no function extensionality is needed anywhere in this development. -/
def Machine.Eq (m m' : Machine) : Prop :=
  (∀ v, m.regs v = m'.regs v) ∧ (∀ w, m.mem w = m'.mem w)
  ∧ m.pc = m'.pc ∧ m.iip = m'.iip

/-- Register update that writes `d := r`, leaving every other cell untouched. -/
def setReg (m : Machine) (d : VReg) (r : Option Word) : Machine :=
  { m with regs := fun v => if v = d then r else m.regs v }

/-- Memory update writing `w` at `addr`. -/
def setMem (m : Machine) (addr w : Word) : Machine :=
  { m with mem := fun a => if a = addr then w else m.mem a }

/-! ## Operand evaluation. -/

/-- Evaluate an operand in a machine state. An immediate never traps. -/
def Arg.eval (a : Arg) (m : Machine) : Option Word :=
  match a with
  | .reg r => m.regs r
  | .imm w => some w

/-- Evaluate a list of operands; `none` on the first undefined read. -/
def Arg.evalList : List Arg → Machine → Option (List Word)
  | [], _ => some []
  | a :: rest, m => match a.eval m with
    | none => none
    | some w => match Arg.evalList rest m with
      | none => none
      | some ws => some (w :: ws)

/-! ## One instruction step. -/

/-- Execute one instruction. All writes are explicit; every read failure is the
    `undefRead` trap; division by zero is the `divZero` trap. -/
def stepInstr (m : Machine) (i : Instr) : Except Trap Machine :=
  match i with
  | .nop => .ok m
  | .copy d a =>
    match a.eval m with
    | none => .error .undefRead
    | some w => .ok (setReg m d (some w))
  | .binop d o x y =>
    match x.eval m with
    | none => .error .undefRead
    | some a =>
      match y.eval m with
      | none => .error .undefRead
      | some b =>
        match o.eval a b with
        | none => .error .divZero
        | some r => .ok (setReg m d (some r))
  | .load d a =>
    match a.eval m with
    | none => .error .undefRead
    | some addr => .ok (setReg m d (some (m.mem addr)))
  | .store a v =>
    match a.eval m with
    | none => .error .undefRead
    | some addr =>
      match v.eval m with
      | none => .error .undefRead
      | some w => .ok (setMem m addr w)
  | .call d f args =>
    match Arg.evalList args m with
    | none => .error .undefRead
    | some _ =>
      -- Opaque call: clobbers every register; the destination receives an opaque
      -- value modeled as 0. The target is uninterpreted. Memory is unchanged;
      -- no proved pass relies on call purity (calls are never removed).
      .ok (setReg { m with regs := fun _ => none } d (some 0))

/-! ## One tick: one instruction, or the block terminator. -/

/-- The result of one tick: advance, trap, or halt with the final state and
    returned value. -/
inductive Step where
  | next : Machine → Step
  | stuck : Trap → Step
  | halt : Machine → Option Word → Step

/-- Fetch block `pc`, or `none` when out of range. -/
def Prog.block? (p : Prog) (pc : Nat) : Option Block := p.blocks[pc]?

/-- One tick of the machine: execute the instruction at the cursor, or the
    terminator when the cursor is past the end of the block. -/
def tick (p : Prog) (m : Machine) : Step :=
  match p.block? m.pc with
  | none => .stuck .badBlock
  | some bl =>
    match bl.instrs[m.iip]? with
    | some i =>
      match stepInstr m i with
      | .ok m' => .next { m' with pc := m.pc, iip := m.iip + 1 }
      | .error t => .stuck t
    | none =>
      match bl.term with
      | .jmp l =>
        if l < p.blocks.length then .next { m with pc := l, iip := 0 }
        else .stuck .badTarget
      | .br c l r =>
        match c.eval m with
        | none => .stuck .undefRead
        | some w =>
          let l' := if w = 0 then r else l
          if l' < p.blocks.length then .next { m with pc := l', iip := 0 }
          else .stuck .badTarget
      | .ret a =>
        match a with
        | none => .halt m none
        | some c => match c.eval m with
          | none => .stuck .undefRead
          | some w => .halt m (some w)

/-! ## Fuel-bounded execution. -/

/-- Total outcomes of a bounded run. -/
inductive Outcome where
  | fuel | stuck : Trap → Outcome | halt : Machine → Option Word → Outcome

/-- Execute at most `n` instructions/terminators. Fuel exhaustion is an outcome,
    never a silent success. -/
def run : Nat → Prog → Machine → Outcome
  | 0, _, _ => .fuel
  | n+1, p, m =>
    match tick p m with
    | .next m' => run n p m'
    | .stuck t => .stuck t
    | .halt m' v => .halt m' v

/-! ## Observable behavior and strict equivalence. -/

/-- Observational equivalence of two outcomes: the trap class, the returned value,
    and the final memory must agree. Register cells, pc, and the cursor are not
    observable. Fuel and fuel are trivially equivalent. -/
def ObsEquiv : Outcome → Outcome → Prop
  | .fuel, .fuel => True
  | .stuck t, .stuck t' => t = t'
  | .halt m v, .halt m' v' => v = v' ∧ ∀ w, m.mem w = m'.mem w
  | _, _ => False

/-- Strict semantic equivalence: at every fuel and from every initial state, the
    two programs have observationally equivalent outcomes. All proved passes
    preserve the fuel profile exactly (in-place rewrites and 1-for-1 nop
    replacement never change the step count), so no quotienting by fuel and no
    classical choice is needed anywhere in this development. -/
def SemEquiv (p q : Prog) : Prop := ∀ (n : Nat) (m : Machine), ObsEquiv (run n p m) (run n q m)

/-! ## Basic lemmas about the equivalence relation. -/

theorem ObsEquiv_refl : ∀ o, ObsEquiv o o
  | .fuel => trivial
  | .stuck _ => rfl
  | .halt _ _ => ⟨rfl, fun _ => rfl⟩

theorem ObsEquiv_symm : ∀ o o', ObsEquiv o o' → ObsEquiv o' o
  | .fuel, .fuel, _ => trivial
  | .stuck _, .stuck _, h => h.symm
  | .halt _ _, .halt _ _, h => ⟨h.1.symm, fun w => (h.2 w).symm⟩
  | .fuel, .stuck _, h => by cases h
  | .fuel, .halt _ _, h => by cases h
  | .stuck _, .fuel, h => by cases h
  | .stuck _, .halt _ _, h => by cases h
  | .halt _ _, .fuel, h => by cases h
  | .halt _ _, .stuck _, h => by cases h

theorem ObsEquiv_trans : ∀ a b c, ObsEquiv a b → ObsEquiv b c → ObsEquiv a c
  | .fuel, .fuel, .fuel, _, _ => trivial
  | .stuck _, .stuck _, .stuck _, h₁, h₂ => h₁.trans h₂
  | .halt _ _, .halt _ _, .halt _ _, h₁, h₂ => ⟨h₁.1.trans h₂.1, fun w => (h₁.2 w).trans (h₂.2 w)⟩
  | .fuel, .stuck _, _, h₁, _ => by cases h₁
  | .fuel, .halt _ _, _, h₁, _ => by cases h₁
  | .stuck _, .fuel, _, h₁, _ => by cases h₁
  | .stuck _, .halt _ _, _, h₁, _ => by cases h₁
  | .halt _ _, .fuel, _, h₁, _ => by cases h₁
  | .halt _ _, .stuck _, _, h₁, _ => by cases h₁

/-- `SemEquiv` is an equivalence relation. -/
theorem SemEquiv_refl (p : Prog) : SemEquiv p p := fun _ _ => ObsEquiv_refl _

theorem SemEquiv_symm {p q : Prog} (h : SemEquiv p q) : SemEquiv q p :=
  fun n m => ObsEquiv_symm _ _ (h n m)

theorem SemEquiv_trans {p q r : Prog} (h₁ : SemEquiv p q) (h₂ : SemEquiv q r) : SemEquiv p r :=
  fun n m => ObsEquiv_trans _ _ _ (h₁ n m) (h₂ n m)

end RV8FV
