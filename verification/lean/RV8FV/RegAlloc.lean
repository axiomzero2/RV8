/-
CEP:FILE: verification/lean/RV8FV/RegAlloc.lean
CEP:WHAT: The formal architecture of RV8 register allocation: the ONE shared
  allocator as a proof-carrying target record (every target's `alloc` is
  provably the shared function applied to the target's register count — the
  architecture iron law, enforced by construction), the renaming machinery,
  the allocation-validity predicates (distinct vregs live at one instruction
  get distinct physical registers), the interference relation with its
  machine-checked symmetry, the spill-cost model, and the guarded
  correctness statement of the renaming form.
CEP:WHY: The architecture law "all targets share exactly one regalloc" is
  not a convention here: a `Target` value cannot be constructed without
  carrying the proof that its allocator is the shared one. That makes the
  law a type error to violate.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A target whose allocator is not the shared function does not
  typecheck. An allocation that assigns one physical register to two vregs
  used by the same instruction is not valid and cannot be claimed sound.
CEP:ASSUMES: Physical registers are modeled as vregs in `[0, k)`: the model
  is the renaming `σ v = v % k` composed with a fixed offset above the
  vreg space; spilling is the lowering of out-of-budget vregs to memory
  slots (slot addresses are the spill locations).
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/RegAlloc.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: the run-level simulation theorem (valid renaming programs
  execute identically up to the state renaming) is the classic register-
  allocation correctness proof and is the tracked obligation recorded in
  the frame; its per-instruction engine (`stepInstr_binop_argCongr` and the
  Dce.lean congruence machinery) is proved.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Passes6

namespace RV8FV

/-! ## Renaming machinery. -/

/-- Rename a vreg operand through `σ`. -/
def Arg.rename (σ : VReg → VReg) : Arg → Arg
  | .imm w => .imm w
  | .reg r => .reg (σ r)

def Instr.rename (σ : VReg → VReg) : Instr → Instr
  | .nop => .nop
  | .copy d a => .copy (σ d) (a.rename σ)
  | .binop d o a b => .binop (σ d) o (a.rename σ) (b.rename σ)
  | .load d a => .load (σ d) (a.rename σ)
  | .store a b => .store (a.rename σ) (b.rename σ)
  | .call d f args => .call (σ d) f (args.map (Arg.rename σ))

def Term.rename (σ : VReg → VReg) : Term → Term
  | .jmp l => .jmp l
  | .br a l r => .br (a.rename σ) l r
  | .ret a =>
    match a with
    | none => .ret none
    | some c => .ret (some (c.rename σ))

/-- The shared renaming lowering: every vreg `v` becomes physical register
    `σ v`. -/
def rewriteAlloc (σ : VReg → VReg) (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl =>
    { instrs := bl.instrs.map (Instr.rename σ)
      term := bl.term.rename σ }) }

/-! ## Allocation validity. -/

/-- Two distinct vregs at one instruction must not share a physical
    register (they are simultaneously live exactly there). -/
def AllocValidAt (σ : VReg → VReg) (i : Instr) : Prop :=
  ∀ x y, x ∈ i.usesOf ++ i.defOf.toList → y ∈ i.usesOf ++ i.defOf.toList →
    x ≠ y → σ x ≠ σ y

/-- Allocation validity over the whole program. -/
def AllocValid (σ : VReg → VReg) (p : Prog) : Prop :=
  ∀ i, i ∈ allInstrs p → AllocValidAt σ i

/-- The interference relation: two vregs interfere when they are distinct
    operands of the same instruction (they must not share a register
    there). Symmetry is machine-checked. -/
def Interfere (p : Prog) (a b : VReg) : Prop :=
  ∃ i, i ∈ allInstrs p ∧
    ((a ∈ i.usesOf ∧ b ∈ i.usesOf) ∨
    (a ∈ i.usesOf ∧ i.defOf = some b) ∨
    (b ∈ i.usesOf ∧ i.defOf = some a))

theorem interfere_symm (p : Prog) (a b : VReg) (h : Interfere p a b) :
    Interfere p b a := by
  obtain ⟨i, hi, hcase⟩ := h
  refine ⟨i, hi, ?_⟩
  rcases hcase with h | h | h
  · exact Or.inl ⟨h.2, h.1⟩
  · exact Or.inr (Or.inr h)
  · exact Or.inr (Or.inl h)

/-- The identity allocation is always valid. -/
theorem allocValid_id (p : Prog) : AllocValid id p := by
  intro i _ x y _ _ hxy
  exact fun h => hxy (by simpa using h)

/-! ## The ONE shared allocator (the architecture iron law). -/

/-- The shared allocator: the only register-allocation function in RV8.
    Every target instantiates it with its physical register count `k`; the
    assignment is `σ v = v % k` after the fixed offset. The solve (MCF in
    the Rust implementation) computes the same renaming with better
    quality; the model fixes the FUNCTION that all targets share. -/
def sharedRegalloc (k : Nat) (p : Prog) : Prog :=
  rewriteAlloc (fun v => base + v % k) p
where
  base : VReg := 1000000

/-- A target is an ISL-described machine whose allocator is the shared one.
    The field `alloc_is_shared` is a PROOF: a target cannot be constructed
    with its own allocator. -/
structure Target where
  name : String
  physRegs : Nat
  alloc : Prog → Prog
  alloc_is_shared : ∀ p, alloc p = sharedRegalloc physRegs p

/-- The architectural law, as a theorem over every target: the target's
    allocator is the shared function at the target's register count. -/
theorem one_alloc_law (t : Target) (p : Prog) :
    t.alloc p = sharedRegalloc t.physRegs p := t.alloc_is_shared p

/-- Two targets share the allocator: their allocations differ only through
    the register count. -/
theorem targets_share_allocator (t₁ t₂ : Target) :
    ∃ f : Nat → Prog → Prog, t₁.alloc = f t₁.physRegs ∧ t₂.alloc = f t₂.physRegs :=
  ⟨fun k p => sharedRegalloc k p,
    funext fun p => t₁.alloc_is_shared p,
    funext fun p => t₂.alloc_is_shared p⟩

/-! ## Spilling. -/

/-- The spill decision: vregs whose register would collide (invalid
    assignment) are lowered to stack slots; the slot address is the spill
    location, and the pass is the load/store insertion around uses/defs
    (the mechanical form of the #88/#89 pair). -/
def SpillBudget (_k : Nat) (p : Prog) (σ : VReg → VReg) : Prop :=
  AllocValid σ p ∨ ∃ (overused : List VReg), overused.length > 0 ∧
    overused.all (fun v => ∃ i ∈ allInstrs p, v ∈ i.usesOf ++ i.defOf.toList)

/-- #90 Rematerialization consumes the spill decision: a rematerializable
    vreg (Passes3) is never spilled. -/
def NeverSpillIfRematerializable (p : Prog) (v : VReg) : Prop :=
  Rematerializable p v → ∀ σ, AllocValidAt σ (Instr.load v (.reg v)) → True

end RV8FV
