/-
CEP:FILE: verification/lean/RV8FV.lean
CEP:WHAT: Root module of the RV8FV Lean 4 formalization of the RV8 compiler core.
CEP:WHY: Single import point for the library; `lake build` checks every theorem.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: Build failure of any imported module fails the CI gate.
CEP:ASSUMES: none.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see per-module headers.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Spec
import RV8FV.Exec
import RV8FV.Machine
import RV8FV.Fold
import RV8FV.Dce
import RV8FV.DceProof
import RV8FV.Opt
import RV8FV.Pipeline
import RV8FV.Catalog
import RV8FV.Smoke
