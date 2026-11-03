/-
CEP:FILE: verification/lean/RV8FV/Pass.lean
CEP:WHAT: The pass-contract algebra of RV8: what it means for a program
  transformation to be semantics-preserving (`Sound`), conditionally
  semantics-preserving (`SoundOn`), SSA-preserving (`KeepSSA`),
  block-count-preserving (`KeepBlocks`), and size-non-increasing (`NoGrow`);
  the composition theorems that let per-pass theorems assemble into pipeline
  theorems; and the `PassFrame` record every catalog pass instantiates.
CEP:WHY: The design spec requires all 176 optimization passes to be
  mathematically defined. This file fixes the contract vocabulary those
  definitions use, with the composition laws machine-checked so that a
  pipeline of proved passes is itself proved by construction.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A pass claimed sound whose definition does not satisfy the
  contract it is registered under is rejected by the build.
CEP:ASSUMES: none beyond the model (Spec.lean, Machine.lean).
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Pass.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: contracts are over the LIR model of Machine.lean; Rust-side
  passes are bridged by the deterministic differential suites.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Catalog

namespace RV8FV

/-! ## Pass contracts. -/

/-- Unconditional semantic preservation at every fuel and initial state. -/
def Sound (f : Prog → Prog) : Prop := ∀ p, SemEquiv p (f p)

/-- Conditional semantic preservation under a precondition on the program. -/
def SoundOn (pre : Prog → Prop) (f : Prog → Prog) : Prop :=
  ∀ p, pre p → SemEquiv p (f p)

/-- Preservation of SSA single-definition well-formedness. -/
def KeepSSA (f : Prog → Prog) : Prop :=
  ∀ p, WfSSA p → WfSSA (f p)

/-- Preservation of the block count. -/
def KeepBlocks (f : Prog → Prog) : Prop :=
  ∀ p, (f p).blocks.length = p.blocks.length

/-- Static size never increases. -/
def NoGrow (f : Prog → Prog) : Prop :=
  ∀ p, (f p).size ≤ p.size

/-! ## Composition laws (machine-checked). -/

theorem sound_id : Sound id := fun _ _ _ => ObsEquiv_refl _

theorem sound_comp {f g : Prog → Prog} (hf : Sound f) (hg : Sound g) :
    Sound (g ∘ f) :=
  fun p n m => ObsEquiv_trans _ _ _ (hf p n m) (hg (f p) n m)

theorem soundOn_weaken {pre pre' : Prog → Prop} {f : Prog → Prog}
    (h : SoundOn pre f) (hle : ∀ p, pre' p → pre p) : SoundOn pre' f :=
  fun p hp => h p (hle p hp)

theorem soundOn_comp {pre : Prog → Prop} {f g : Prog → Prog}
    (hf : SoundOn pre f) (hg : Sound g) (hpre : ∀ p, pre p → pre (f p)) :
    SoundOn pre (g ∘ f) :=
  fun p hp n m => ObsEquiv_trans _ _ _ (hf p hp n m) (hg (f p) n m)

theorem keepSSA_comp {f g : Prog → Prog} (hf : KeepSSA f) (hg : KeepSSA g) :
    KeepSSA (g ∘ f) :=
  fun p hp => hg (f p) (hf p hp)

theorem keepBlocks_comp {f g : Prog → Prog} (hf : KeepBlocks f) (hg : KeepBlocks g) :
    KeepBlocks (g ∘ f) :=
  fun p => (hg (f p)).trans (hf p)

theorem noGrow_comp {f g : Prog → Prog} (hf : NoGrow f) (hg : NoGrow g) :
    NoGrow (g ∘ f) :=
  fun p => Nat.le_trans (hg (f p)) (hf p)

/-- A `Sound` pass is `SoundOn` any precondition. -/
theorem sound_soundOn {f : Prog → Prog} (h : Sound f) (pre : Prog → Prop) :
    SoundOn pre f := fun p _ => h p

/-! ## Pass frames. -/

/-- The registration record of a catalog pass: which mathematical object is
    the pass, in which file it is defined, and which contracts are
    machine-checked versus tracked obligations (CEP&CC 54.2.2). -/
instance : Inhabited FVClass := ⟨.fv0⟩
instance : Inhabited PassKind := ⟨.valueRewrite⟩

structure PassFrame where
  id : Nat
  name : String
  category : Nat
  kind : PassKind
  fv : FVClass
  spec : String
  status : String
  deriving Inhabited

end RV8FV
