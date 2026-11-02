import Lake
open Lake DSL

/-
CEP:FILE: verification/lean/lakefile.lean
CEP:WHAT: Lake package for the RV8FV Lean 4 formalization: zero external dependencies,
  one library (RV8FV), one test executable (rv8fvtest).
CEP:WHY: CEP&CC 54.5.1 requires version-pinned, reproducible proof builds with
  `lake build` and `lake test` as CI gates.
CEP:CLASS: FV-2
CEP:STATUS: complete
CEP:FAILURE: A toolchain other than the one pinned in lean-toolchain is a
  reproducibility violation and must fail the build.
CEP:ASSUMES: elan resolves the pinned toolchain; no network access needed after pin install.
CEP:FV-CLASS: FV-2
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/lakefile.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: Build orchestration only; carries no semantics.
CEP:FV-AXIOMS: none.
-/

package «rv8fv» where
  leanOptions := #[⟨`autoImplicit, false⟩]

lean_lib «RV8FV» where

@[test_driver]
lean_exe «rv8fvtest» where
  root := `SmokeMain
