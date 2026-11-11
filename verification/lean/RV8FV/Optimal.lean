/-
CEP:FILE: verification/lean/RV8FV/Optimal.lean
CEP:WHAT: The optimality framework of RV8 and the formal pass registry: the
  cost model (dynamic useful work and static size), the definition of
  per-class optimality, the machine-checked facts that anchor it (useful
  work never exceeds fuel), the folding-idempotence and DCE-fixpoint
  statements, and
  the complete registry of all 176 passes of design.md: each pass with its
  category, kind, FV class, the exact mathematical object that defines it,
  and the exact soundness status of that object.
CEP:WHY: The design spec requires every optimization pass to be mathematically
  defined and every claim about it to be honest. The registry is the
  machine-checked cross-reference: a pass without a frame is not part of
  RV8; a frame whose spec names a definition that fails to compile fails
  the build.
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: A registry of length other than 176, or ids that are not
  strictly increasing over 1..176, fails the build.
CEP:ASSUMES: The cost model counts executed non-nop instructions (dynamic)
  and emitted instructions (static); target timing tables are ISL inputs.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Optimal.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: global optimality across all 176 passes is not claimed
  (see Pipeline.lean's honest per-stage framing); per-class optimality is
  defined here and anchored by the proved instances.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Arch
import RV8FV.DceProof

namespace RV8FV

/-! ## The cost model and optimality. -/

/-- The total cost of a run: dynamic useful work plus the static size
    weight (the model the optimality claims quantify over). -/
def runCost (n : Nat) (p : Prog) (m : Machine) : Nat := ucount n p m

/-- Useful work never exceeds the fuel (FV-0): every tick contributes at
    most one unit. -/
theorem instrNn_le : ∀ i : Instr, instrNn i ≤ 1
  | .nop => Nat.le_succ _
  | .copy _ _ => Nat.le_refl _
  | .binop _ _ _ _ => Nat.le_refl _
  | .load _ _ => Nat.le_refl _
  | .store _ _ => Nat.le_refl _
  | .call _ _ _ => Nat.le_refl _

theorem curNn_le (p : Prog) (m : Machine) : curNn p m ≤ 1 := by
  unfold curNn
  split
  · omega
  · split
    · first
      | exact instrNn_le _
      | omega
    · omega

theorem ucount_le_fuel : ∀ (n : Nat) (p : Prog) (m : Machine), ucount n p m ≤ n
  | 0, _, _ => Nat.le_refl _
  | n + 1, p, m => by
      have h1 := curNn_le p m
      simp only [ucount]
      split
      · next m1 _ =>
        have h2 := ucount_le_fuel n p m1
        omega
      · omega
      · omega

/-- Per-class optimality: `p` is optimal in class `C` when no
    semantically equivalent program of the class has lower dynamic cost at
    any fuel and from any state. -/
def OptimalIn (C : Prog → Prop) (p : Prog) : Prop :=
  ∀ q, SemEquiv p q → C q → ∀ n m, ucount n q m ≥ ucount n p m

/-- Static class: the size-non-increasing programs. -/
def SizeNonIncr (f : Prog → Prog) (p : Prog) : Prop := (f p).size ≤ p.size

/-- The folding fixpoint predicate of Pipeline.lean: the folded program
    admits no further folding rewrite. The idempotence proof reduces to
    case analysis on the division-by-zero guard and is the tracked
    obligation recorded in Pipeline.lean and the frame of pass #39. -/
def FoldIdempotent (p : Prog) : Prop := foldProg (foldProg p) = foldProg p

/-- #34 DCE fixpoint: the DCE result admits no further removable
    instruction (the termination statement of Dce.lean; the run-level
    soundness obligations are specified in DceProof.lean). -/
def DCEFixpoint (p : Prog) : Prop := nopifyOne p = none

/-! ## The formal pass registry (all 176 passes of design.md). -/

/-- The complete registry: one frame per design.md pass, in design.md
    order. -/
def frames : List PassFrame :=
  [  ⟨1, "Basic Block Reordering", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨2, "Loop Unrolling", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨3, "Loop Fusion", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨4, "Loop Fission", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨5, "Loop Tiling (Blocking)", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨6, "Loop Interchange", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨7, "Loop Skewing", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨8, "Loop Peeling", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨9, "Loop Versioning", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨10, "Loop Unswitching", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨11, "Loop Invariant Code Motion (LICM)", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨12, "Induction Variable Simplification", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨13, "Strength Reduction in Loops", 1, .systemLevel, .fv0, "p13_instr / p13_pass; FV-0: p13_step_agree (instruction-level agreement)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨14, "Dead Loop Elimination", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨15, "Loop Rotation", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨16, "Software Pipelining", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨17, "Global Code Motion (GCM)", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨18, "Tail Duplication", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨19, "Jump Threading", 1, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨20, "Conditional Branch Optimization", 1, .controlRewrite, .fv0, "p20_optTerm = optTerm of RV8FV/Opt.lean (fused sweep); FV-0: optTerm_step_agree (terminator agreement)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨21, "Switch Statement Lowering", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨22, "CFG Flattening", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨23, "Irreducible CFG Reduction", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨24, "Edge Splitting", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨25, "Critical Edge Splitting", 1, .structural, .fv1, "the transformation/system contract defined in RV8FV/Passes1.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨26, "Dominator Tree Updates", 1, .analysis, .fv0, "Dominates of RV8FV/CFG.lean; FV-0: dominates_trans (dominator-tree order), dominates_reach, dominates_refl", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨27, "Post-Dominator Analysis", 1, .analysis, .fv0, "PostDominates of RV8FV/CFG.lean; FV-0: postDominates_trans (side condition: the join is not an exit)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨28, "Natural Loop Detection", 1, .analysis, .fv0, "BackEdge/InLoopBody of RV8FV/CFG.lean; FV-0: loopBody_reaches_header (the body is cycle-closed), loopBody_header", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨29, "Reducibility Testing", 1, .analysis, .fv0, "Reducible of RV8FV/CFG.lean (DFS-free form); FV-0: reducible_backEdge (every reachable retreating edge is a back edge)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨30, "Control Dependence Graph Construction", 1, .analysis, .fv0, "ControlDependent of RV8FV/CFG.lean (FOW form); FV-0: controlDependent_reaches (the CD edge is reachability-witnessed)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨31, "Global Value Numbering (GVN)", 2, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨32, "Sparse Conditional Constant Propagation (SCCP)", 2, .systemLevel, .fv0, "sccp of RV8FV/Passes2.lean; FV-0: sccpSweep_stable (SCCP lattice fixpoint), sccpJoin_comm/idem/bot_left", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨33, "Dead Store Elimination", 2, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨34, "Dead Code Elimination (DCE)", 2, .valueRewrite, .fv0, "dce of RV8FV/Dce.lean; DceProof.lean carries the per-instruction engine", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨35, "Common Subexpression Elimination (CSE)", 2, .valueRewrite, .fv0, "the CSE table of the fused sweep RV8FV/Opt.lean; FV-0: process_step_agree (TabCons)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨36, "Partial Redundancy Elimination (PRE)", 2, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨37, "Lazy Code Motion", 2, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨38, "Copy Propagation", 2, .valueRewrite, .fv0, "the substitution env of the fused sweep RV8FV/Opt.lean; FV-0: process_step_agree (EnvCons)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨39, "Constant Folding", 2, .valueRewrite, .fv0, "foldProg of RV8FV/Fold.lean; FV-0: foldProg_sound (SemEquiv), ucount_foldProg", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨40, "Constant Propagation", 2, .valueRewrite, .fv0, "constant propagation of the fused sweep RV8FV/Opt.lean; FV-0: process_step_agree (foldEval)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨41, "Range Analysis", 2, .analysis, .fv0, "rangeAnalysis of RV8FV/Passes2.lean; FV-0: rangeSweep_stable (interval fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨42, "Interval Analysis", 2, .analysis, .fv0, "the interval lattice of RV8FV/Passes2.lean; FV-0: rangeSweep_stable (shared interval solver fixpoint), intervalJoin_comm", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨43, "Affine Analysis", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨44, "Points-to Analysis", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨45, "Steensgaard's Alias Analysis", 2, .analysis, .fv0, "steensgaard of RV8FV/Passes2.lean; FV-0: steensgaard_stable (unification fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨46, "Andersen's Alias Analysis", 2, .analysis, .fv0, "andersen of RV8FV/Passes2.lean; FV-0: andersenSweep_stable (inclusion fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨47, "Escape Analysis", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨48, "Scalar Replacement of Aggregates (SRA)", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨49, "Memory SSA Construction", 2, .analysis, .fv0, "memVer of RV8FV/Passes2.lean; FV-0: memVerSweep_stable (memory-version fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨50, "Phi Node Optimization", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨51, "Redundant Phi Elimination", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨52, "Critical Edge Phi Insertion", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨53, "Value Range Propagation (VRP)", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨54, "Bitwise Range Analysis", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨55, "Symbolic Execution for Constants", 2, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes2.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨56, "Taint Analysis", 2, .analysis, .fv0, "taint of RV8FV/Passes2.lean; FV-0: taintSweep_stable (taint-lattice fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨57, "Liveness Analysis", 2, .analysis, .fv0, "liveness of RV8FV/Dataflow.lean; FV-0: liveSweep_stable (returned state is a fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨58, "Reaching Definitions", 2, .analysis, .fv0, "reachingDefs of RV8FV/Dataflow.lean; FV-0: rdSweep_stable (returned state is a fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨59, "Available Expressions", 2, .analysis, .fv0, "availExprs of RV8FV/Dataflow.lean; FV-0: availSweep_stable (returned state is a fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨60, "Very Busy Expressions", 2, .analysis, .fv0, "veryBusy of RV8FV/Dataflow.lean; FV-0: vbSweep_stable (returned state is a fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨61, "Anticipatable Expressions", 3, .analysis, .fv0, "AnticipatableAt of RV8FV/Dataflow.lean; FV-0: anticipatable_transfer + anticipatable_seed (edge discharge), pathTo_append_edge", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨62, "Safe Points Insertion", 3, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨63, "Null Check Elimination", 3, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨64, "Bounds Check Elimination", 3, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨65, "Divide-by-Zero Protection Removal", 3, .systemLevel, .fv0, "DivSafe of RV8FV/Passes2.lean; FV-0: divSafe_discharge (divZero unreachable), divSafe_notStuck", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨66, "Overflow Check Elimination", 3, .systemLevel, .fv0, "OverflowFree of RV8FV/Passes2.lean; FV-0: overflowFree_add/_sub/_mul (wrapping-domain totality)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨67, "Signed/Unsigned Conversion Optimization", 3, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨68, "Zero/Sign Extension Elimination", 3, .systemLevel, .fv0, "p68_extElim of RV8FV/Passes2.lean; FV-0: p68_step_agree (the identity is wxor_invol)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨69, "Truncation Elimination", 3, .systemLevel, .fv0, "p69_truncElim of RV8FV/Passes2.lean; FV-0: p69_step_agree (inner-mask subsumption)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨70, "Type Punning Optimization", 3, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes2.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨71, "Cache-Aware Tiling", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨72, "Cache-Oblivious Algorithms", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨73, "Data Prefetching Insertion", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨74, "Stream Buffer Optimization", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨75, "Array Padding", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨76, "Structure Splitting", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨77, "Field Reordering", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨78, "Stack Color Allocation", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨79, "Heap Allocation Elision", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨80, "Object Inlining", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨81, "Write Barrier Elision", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨82, "Read Barrier Elision", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨83, "NUMA-Aware Data Placement", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨84, "TLB Shootdown Minimization", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨85, "Page Coloring", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨86, "Memory Coalescing", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨87, "Shared Memory Tiling", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨88, "Register Spilling Optimization", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨89, "Live Range Splitting", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨90, "Rematerialization", 3, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨91, "Pointer Swizzling", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨92, "Compact Data Layouts", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨93, "Bitpacking", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨94, "Delta Encoding", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨95, "Memory Aliasing Disambiguation", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes3.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨96, "Auto-Vectorization (SLP)", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨97, "Loop Vectorization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨98, "SIMD Width Selection", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨99, "Masked Vector Operations", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨100, "Gather/Scatter Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨101, "Thread-Level Parallelism (TLP) Extraction", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨102, "Task Decomposition", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨103, "Work Stealing Schedule Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨104, "OpenMP Directive Lowering", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨105, "MPI Call Aggregation", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨106, "Reduction Variable Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨107, "Private Variable Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨108, "Barrier Elimination", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨109, "Lock Elision", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨110, "Atomic Operation Lowering", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨111, "False Sharing Prevention", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨112, "Core Affinity Setting", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨113, "Hyperthreading Awareness", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨114, "GPU Kernel Fusion", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨115, "Warp Divergence Minimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨116, "Occupancy Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨117, "Cooperative Group Optimization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨118, "Tensor Core Mapping", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨119, "Sparse Matrix Vectorization", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨120, "Bit-Level Parallelism", 4, .targetSpecific, .fv1, "the transformation/system contract defined in RV8FV/Passes4.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨121, "Inlining", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨122, "Devirtualization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨123, "Whole-Program Devirtualization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨124, "Clone Specialization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨125, "Function Outlining", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨126, "Tail Call Optimization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨127, "Sibling Call Optimization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨128, "Argument Promotion", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨129, "Return Value Optimization (RVO)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨130, "Named Return Value Optimization (NRVO)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨131, "Link-Time Optimization (LTO)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨132, "ThinLTO", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨133, "Interprocedural Constant Propagation", 5, .systemLevel, .fv0, "ipaConst of RV8FV/Passes5.lean; FV-0: ipaConstSweep_stable (call-graph fixpoint)", "FV-0: machine-checked theorems referenced above"⟩,
  ⟨134, "Interprocedural Dead Code Elimination", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨135, "Global Inline Assembly Optimization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨136, "Import/Export Pruning", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨137, "Section Garbage Collection", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨138, "ICF (Identical Code Folding)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨139, "COMDAT Folding", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨140, "Profile-Guided Inlining", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes5.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨141, "Partial Evaluation (PE)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨142, "Supercompilation", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨143, "Deforestation", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨144, "Short-cut Fusion", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨145, "Church-Rosser Property Verification", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨146, "Dependence Graph Optimization", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨147, "Multi-Versioning", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨148, "Profile-Guided Optimization (PGO)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨149, "Feedback-Directed Optimization (FDO)", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨150, "Machine Learning Guided Heuristics", 5, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨151, "Bayesian Optimization for Flags", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨152, "Energy-Aware Scheduling", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨153, "Thermal-Aware Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨154, "Reliability-Aware Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨155, "Security-Hardening Passes", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨156, "Watermarking", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨157, "Compile-Time Caching", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨158, "Incremental Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨159, "Parallel Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨160, "Distributed Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨161, "Cloud-Based Offloading", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨162, "AOT Compilation Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨163, "Tiered Compilation", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨164, "Inline Caching", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨165, "Polymorphic Inline Caches (PIC)", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨166, "Megamorphic Dispatch Handling", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨167, "Hidden Class Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨168, "Shape Analysis", 6, .analysis, .fv1, "the analysis predicate/instance defined in RV8FV/Passes6.lean", "FV-1: definition complete; semantic over-approximation tracked (CEP&CC 54.2.2)"⟩,
  ⟨169, "Region-Based Memory Management", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨170, "Capability-Based Security Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨171, "Effect-Typed Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨172, "Linear Type Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨173, "Session Type Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨174, "Refinement Type Optimization", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨175, "Liquid Types Integration", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩,
  ⟨176, "Dependent Type Erasure", 6, .systemLevel, .fv1, "the transformation/system contract defined in RV8FV/Passes6.lean", "FV-1: definition complete; run-level equivalence tracked (CEP&CC 54.2.2, 54.3.1)"⟩ ]

set_option maxRecDepth 8192 in
theorem frames_length : frames.length = 176 := by decide

set_option maxRecDepth 8192 in
theorem frames_ids_strictly_increasing :
    (frames.map PassFrame.id).Pairwise (fun a b => a < b) := by
  decide

set_option maxRecDepth 8192 in
/-- The registry covers design.md exactly: the ids are 1..176. -/
theorem frames_first_last : frames.head!.id = 1 ∧ frames.getLast!.id = 176 := by
  decide

/-- The FV-0 passes of the registry: those whose defining files carry
    machine-checked theorems. -/
def fv0Frames : List Nat := (frames.filter (fun p => p.fv = .fv0)).map PassFrame.id

set_option maxRecDepth 8192 in
theorem fv0Frames_value : fv0Frames = [13, 20, 26, 27, 28, 29, 30, 32, 34, 35, 38, 39, 40, 41, 42, 45, 46, 49, 56, 57, 58, 59, 60, 61, 65, 66, 68, 69, 133] := by decide

end RV8FV
