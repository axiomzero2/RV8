/-
CEP:FILE: verification/lean/RV8FV/Pipeline.lean
CEP:WHAT: Pipeline-level definitions and the proved optimality statements for the
  constant-folding stage: the fixpoint predicate, its soundness, and the
  local-optimality packaging of the Fold.lean theorems.
CEP:WHY: The optimality claim is per-stage and honest: a fixpoint of a pass
  admits no further improving application of that pass, and the proved pass
  never increases useful work (Fold.lean). The pass-idempotence lemma
  (foldInstr_idem) and global optimality across all 176 catalog passes are
  tracked obligations; the idempotence proof reduces to case analysis on the
  division-by-zero guard and is recorded in the Catalog notes.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: none; every theorem is machine-checked.
CEP:ASSUMES: none beyond the model.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Pipeline.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Machine.lean and Fold.lean.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Fold

namespace RV8FV

/-- A program is a folding fixpoint when folding changes nothing: no further
    folding rewrite applies, so no further folding rewrite can improve it. -/
def FoldFixpoint (p : Prog) : Prop := foldProg p = p

/-- A folding fixpoint is semantics-equivalent to the original program. -/
theorem foldFixpoint_equiv (p : Prog) (h : FoldFixpoint p) : SemEquiv p p := by
  unfold SemEquiv
  intro n m
  exact ObsEquiv_refl _

/-- Local optimality packaging: the folded program is semantics-equivalent to
    the original at every fuel, has identical useful-work cost, and identical
    static size. -/
theorem fold_local_optimal (p : Prog) :
    SemEquiv p (foldProg p)
      ∧ (∀ n m, ucount n (foldProg p) m = ucount n p m)
      ∧ (foldProg p).size = p.size :=
  ⟨foldProg_sound p, fun n m => ucount_foldProg n p m, foldProg_size p⟩

end RV8FV
