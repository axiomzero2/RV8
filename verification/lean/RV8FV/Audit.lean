/-
CEP:FILE: verification/lean/RV8FV/Audit.lean
CEP:WHAT: The FV-0 axiom audit gate.  For every flagship theorem of the
  formalization the gate walks its transitive axiom dependencies with
  `Lean.collectAxioms` and fails the build if any dependency is outside the
  whitelist {propext, Classical.choice, Quot.sound} — in particular it fails
  on `sorryAx`, so a single `sorry` anywhere below an audited theorem breaks
  `lake build`.
CEP:WHY: CEP&CC 54.2 requires FV-0 results to be machine-checked with zero
  `sorry`.  A build-log grep alone is weak evidence; this gate makes the
  whitelist a *hard elaboration error*, so the guarantee is enforced by the
  compiler itself on every build and every CI run.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: If an audited constant is missing (renamed or deleted) the audit
  fails; if an audited theorem (transitively) depends on a non-whitelisted
  axiom the audit fails.  Both fail `lake build`.
CEP:ASSUMES: Lean's logical foundation is exactly {propext,
  Classical.choice, Quot.sound}; no custom `axiom` declarations are permitted
  in RV8FV (the ISL declares hardware reality as *data*, not as axioms).
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Audit.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: the audited theorems are the flagship results of Wrd, CFG,
  Pass, Dataflow, Passes1-6, RegAlloc, Arch and Optimal (one or two per
  module); the whitelist is the standard axiomatic basis of Lean 4.
CEP:FV-AXIOMS: none (the audit's whole purpose is to prove none exist).
-/

import Lean
import RV8FV.Spec
import RV8FV.Exec
import RV8FV.Machine
import RV8FV.Wrd
import RV8FV.Fold
import RV8FV.Dce
import RV8FV.DceProof
import RV8FV.Opt
import RV8FV.Pipeline
import RV8FV.Pass
import RV8FV.CFG
import RV8FV.Dataflow
import RV8FV.Passes1
import RV8FV.Passes2
import RV8FV.Passes3
import RV8FV.Passes4
import RV8FV.Passes5
import RV8FV.Passes6
import RV8FV.RegAlloc
import RV8FV.Arch
import RV8FV.Optimal
import RV8FV.Catalog
import RV8FV.Smoke

open Lean

namespace RV8FV

/-- The whitelist: the three standard axioms of Lean 4's logical foundation.
    Everything else — above all `sorryAx` — is a FV-0 violation. -/
def Audit.whitelist : List Name :=
  [`propext, `Classical.choice, `Quot.sound]

/-- The flagship theorems audited, one or two per module: the word algebra,
    the CFG path theory, the pass contracts, the fixpoint solver, the
    strength-reduction agreement, every solver stability theorem, the
    regalloc sharing law, the architecture laws, and the catalog laws. -/
def Audit.targets : List Name :=
  -- Machine semantics
  [`RV8FV.ObsEquiv_trans, `RV8FV.SemEquiv_trans, `RV8FV.Arg.eval_subst,
   -- constant folding
   `RV8FV.fold_step_agree, `RV8FV.foldProg_run_eq, `RV8FV.foldProg_sound,
   -- DCE
   `RV8FV.stepInstr_eq_except, `RV8FV.nopifiable_of_removable,
   -- the generic optimizer sweep
   `RV8FV.optBlock_length, `RV8FV.foldFixpoint_equiv, `RV8FV.fold_local_optimal,
   -- the word algebra (the rewrite laws the value passes use)
   `RV8FV.wadd_comm, `RV8FV.wmul_comm, `RV8FV.wmul_wadd_distr,
   `RV8FV.wxor_invol, `RV8FV.wmul_pow2_eq_wshl,
   -- the CFG theory
   `RV8FV.Reach_trans, `RV8FV.reach_path, `RV8FV.setTermBlocks_length,
   -- the pass contracts
   `RV8FV.sound_comp, `RV8FV.soundOn_comp, `RV8FV.keepSSA_comp,
   `RV8FV.noGrow_comp,
   -- the chaotic-iteration solver and the five dataflow analyses
   `RV8FV.sweepToFix_stable, `RV8FV.liveSweep_stable, `RV8FV.rdSweep_stable,
   `RV8FV.availSweep_stable, `RV8FV.vbSweep_stable,
   -- Passes #1-30
   `RV8FV.p01_blocks, `RV8FV.p02_length, `RV8FV.p06_same_domain,
   `RV8FV.p08_length, `RV8FV.p13_step_agree, `RV8FV.p18_length,
   `RV8FV.p23_length, `RV8FV.p24_length,
   -- Passes #31-70
   `RV8FV.sccpJoin_comm, `RV8FV.sccpJoin_idem, `RV8FV.sccpJoin_bot_left,
   `RV8FV.sccpSweep_stable, `RV8FV.intervalJoin_comm, `RV8FV.rangeSweep_stable,
   `RV8FV.andersenSweep_stable, `RV8FV.steensgaard_stable,
   `RV8FV.memVerSweep_stable, `RV8FV.taintSweep_stable, `RV8FV.p63_length,
   -- Passes #71-95
   `RV8FV.p72_length, `RV8FV.alignTo_aligned, `RV8FV.wadd_wsub_cancel,
   `RV8FV.deltaLoop_roundtrip, `RV8FV.delta_roundtrip,
   -- Passes #96-120
   `RV8FV.p103_fair, `RV8FV.wsum_lt, `RV8FV.wsum_append, `RV8FV.wsum_split,
   `RV8FV.vxor_pointwise,
   -- Passes #121-140
   `RV8FV.p121_blockCount, `RV8FV.p126_noGrowth, `RV8FV.p131_join_nil,
   `RV8FV.ipaConstSweep_stable, `RV8FV.CGReach_trans, `RV8FV.icf_law,
   -- Passes #141-176
   `RV8FV.p98_some_invar, `RV8FV.p151_terminates, `RV8FV.p151_deterministic,
   `RV8FV.thermalPeak_mono, `RV8FV.cache_hit_law, `RV8FV.p159_deterministic,
   -- the one shared register allocator
   `RV8FV.interfere_symm, `RV8FV.allocValid_id, `RV8FV.one_alloc_law,
   `RV8FV.targets_share_allocator,
   -- the architecture
   `RV8FV.pipeline_sound, `RV8FV.pipeline_ssa, `RV8FV.isel_optimal_within,
   `RV8FV.abi_deterministic, `RV8FV.emit_faithful,
   `RV8FV.gauntlet_sound_stages,
   -- the optimality and catalog laws
   `RV8FV.instrNn_le, `RV8FV.curNn_le, `RV8FV.ucount_le_fuel,
   `RV8FV.frames_length, `RV8FV.frames_ids_strictly_increasing,
   `RV8FV.frames_first_last, `RV8FV.fv0Frames_value, `RV8FV.catalog_length]

-- The gate itself: run at elaboration time, so `lake build` fails
-- immediately when the whitelist is violated.
open Lean Elab Command in
run_cmd do
  let env ← getEnv
  let wl : List Name := (RV8FV.Audit.whitelist : List Name)
  let mut audited := 0
  for n in (RV8FV.Audit.targets : List Name) do
    if !Lean.Environment.contains env n then
      throwError s!"audit: constant {n} not found (renamed or deleted?)"
    let ax ← liftCoreM (Lean.collectAxioms n)
    for a in ax do
      unless wl.contains a do
        throwError s!"audit: {n} depends on non-whitelisted axiom {a}"
    audited := audited + 1
  logInfo s!"axiom audit: {audited} theorems, whitelist propext/Classical.choice/Quot.sound"

end RV8FV
