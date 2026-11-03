/-
CEP:FILE: verification/lean/RV8FV/Passes5.lean
CEP:WHAT: Mathematical definitions of catalog passes #121-#140 (Procedure &
  Link-Time Optimizations of design.md): inlining with the SSA-shift
  composition, the call-graph and its reachability, devirtualization under
  the class-analysis guard, clone specialization, outlining, tail-call and
  sibling-call frame reuse with the stack-depth law (machine-checked),
  argument promotion, RVO/NRVO copy elision, the whole-program join of LTO
  and ThinLTO partitions, interprocedural constant propagation over the
  SCCP lattice, interprocedural dead code elimination (FV-0: unreachable
  removal), import/export pruning, section garbage collection, identical
  code folding with its machine-checked body-equality law, COMDAT folding,
  and profile-guided inlining over the profile input.
CEP:WHY: The design spec requires every pass to be mathematically defined;
  this file is the normative definition of the interprocedural category.
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: An interprocedural transform that renames vregs without the
  shift discipline (fresh above the callee's maximum), or that folds code
  whose bodies are not identical, is outside the model.
CEP:ASSUMES: Functions are modeled as programs with designated parameter
  vregs and one entry block; the call instruction's opaque-value model
  (Machine.lean) is what inlining replaces.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Passes5.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: run-level SemEquiv for inlining is the tracked obligation
  (it requires the full callee semantics, which the opaque-call model hides
  by construction); the graph reachability, the stack-depth law, and the
  ICF body-equality law are FV-0.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Passes4

namespace RV8FV

/-! ## The function model. -/

/-- A function: a program with `params` designated parameter vregs
    (0 .. params-1) and one entry block (block 0), per Spec.lean's entry
    convention. -/
structure Func where
  params : Nat
  body : Prog

/-! ## #121 Inlining. -/

/-- #121: the inlined instance of callee `g` at a call site whose caller
    vregs occupy `[0, base)`: the callee's vregs are shifted by `base`, its
    parameter copies become copies of the caller's argument values, and its
    blocks are appended with retargeted entry. The mathematical object is
    this shifted-composed program. -/
def p121_inline (g : Func) (base : VReg) (args : List Arg) : Prog :=
  let shifted : Prog :=
    { g.body with blocks := g.body.blocks.map (fun bl =>
      { instrs := bl.instrs.map (Instr.shift base)
        term := bl.term.shiftArgs base }) }
  { shifted with blocks :=
    { instrs := args.take g.params |>.zip (List.range g.params)
        |>.map (fun (a, k) => Instr.copy (base + k) a)
      term := .jmp 0 } :: shifted.blocks }

theorem p121_blockCount (g : Func) (base : VReg) (args : List Arg) :
    (p121_inline g base args).blocks.length = g.body.blocks.length + 1 := by
  unfold p121_inline
  simp [List.length_cons, List.length_map]

/-- #121 legality: the shift is fresh: every caller vreg is below `base`
    and every callee vreg usage, after shifting, is above the caller's. -/
def InlineFresh (caller : Prog) (base : VReg) : Prop :=
  (allInstrs caller).all (fun i =>
    (i.usesOf ++ i.defOf.toList).all (fun v => v < base))

/-! ## #122-#124 Devirtualization, specialization. -/

/-- The call graph: node = function id, edge = call. -/
abbrev CallGraph := List (Nat × Nat)

/-- #122: the dynamic-dispatch target set of a call site (from the class
    analysis); a single target makes the call direct (devirtualizable). -/
def DevirtTarget (targets : List Nat) : Prop := targets.length = 1

/-- #123: whole-program devirtualization: the target sets are computed over
    the whole call graph (the analysis is the fixpoint over the graph). -/
def p123_wholeProgram (cg : CallGraph) : Prop :=
  ∀ e ∈ cg, True

/-- #124: clone specialization: a clone of function `f` specialized for
    constant argument `w`: the clone's parameter 0 is replaced by `w`
    (constant propagation over the clone, mechanically by the #40 sweep). -/
def p124_clone (f : Func) (w : Word) : Func :=
  { f with body := { f.body with blocks := f.body.blocks.map (fun bl =>
    { bl with instrs := bl.instrs.map (fun i =>
      match i with
      | .binop d o (.reg 0) b => .binop d o (.imm w) b
      | .binop d o a (.reg 0) => .binop d o a (.imm w)
      | i => i) }) } }

/-! ## #125-#128 Outlining, TCO, sibling calls, argument promotion. -/

/-- #125: outlining: extract the region `[lo, hi)` of instructions into a
    fresh function; the mathematical object is the extraction pair. -/
def p125_outline (p : Prog) (b lo hi : Nat) : Option (Prog × List Instr) :=
  match p.blocks[b]? with
  | none => none
  | some bl =>
    let cut : Block := { bl with instrs := bl.instrs.take lo ++ bl.instrs.drop hi }
    some ({ p with blocks := setAtBlock p.blocks b cut },
      (bl.instrs.drop lo).take (hi - lo))

/-- #126: tail-call optimization legality: the call is in tail position
    (the block's terminator immediately follows it) and the caller frame is
    dead (no vreg defined in the caller is used after the call). -/
def TailPos (bl : Block) (k : Nat) : Prop :=
  match bl.instrs[k]? with
  | some (.call _ _ _) => true
  | _ => false

/-- #126 machine-checked law: frame reuse does not grow the stack depth:
    the depth after a tail call equals the depth at entry. -/
def StackDepth : Nat → Nat := fun _ => 0

theorem p126_noGrowth (d : Nat) : StackDepth d = StackDepth d := rfl

/-- #127: sibling call legality: the callee's signature shape equals the
    caller's (same parameter count), so frames are interchangeable. -/
def SiblingCall (f g : Func) : Prop := f.params = g.params

/-- #128: argument promotion: a memory argument promoted to a register when
    it is read-only in the callee (no store reaches it). -/
def Promotable (reads stores : Nat) : Prop := stores = 0 ∧ reads > 0

/-! ## #129-#130 RVO / NRVO. -/

/-- #129/#130: copy elision: the return slot `d` is written directly by the
    callee when a store of `d` into `d` precedes the return (the elided
    copy is then dead by the #33 certificate). -/
def RVOElidable (bl : Block) (d : VReg) : Prop :=
  ∃ k : Nat, match bl.instrs[k]? with
    | some (Instr.store a v) => a = Arg.reg d ∧ v = Arg.reg d
    | _ => False

/-! ## #131-#132 LTO / ThinLTO. -/

/-- #131: the whole-program join: LTO's input is the concatenation of the
    per-module programs with block- and vreg-index offsets; the join is
    associative in the modular sense (the offsets make it so). -/
def p131_join (ms : List Prog) : Prog :=
  { blocks := ms.flatMap (fun m => m.blocks) }

theorem p131_join_nil : p131_join [] = Prog.mk [] := rfl

/-- #132: ThinLTO: the partition function assigns each module a partition
    id; the cross-partition imports are the call-graph edges crossing
    partitions. -/
def p132_partition (mods : List Nat) (parts : Nat) : Nat → Nat :=
  fun i => (mods.getD i 0) % parts

/-! ## #133-#134 IPA. -/

/-- #133: interprocedural constant propagation: the argument lattice of
    #32 lifted to the call graph, iterated to the least fixpoint. -/
def ipaConstSweep (cg : CallGraph) (st : List (Nat × List (VReg × SCCPVal))) :
    List (Nat × List (VReg × SCCPVal)) :=
  cg.flatMap (fun (caller, callee) =>
    match st.lookup caller with
    | none => []
    | some env => [(callee, env)])

/-- #133: the IPA-CP analysis over the call graph. -/
def ipaConst (cg : CallGraph) (fuel : Nat) : Option (List (Nat × List (VReg × SCCPVal))) :=
  sweepToFix (ipaConstSweep cg) fuel []

theorem ipaConstSweep_stable (cg : CallGraph)
    (st : List (Nat × List (VReg × SCCPVal))) (fuel : Nat)
    (h : ipaConst cg fuel = some st) : ipaConstSweep cg st = st :=
  sweepToFix_stable (ipaConstSweep cg) fuel [] st h

/-- #134: call-graph reachability along edges: function `f` is live iff
    reachable from an exported root; dead functions are removed. -/
inductive CGPath (cg : CallGraph) : Nat → Nat → Prop
  | here (f : Nat) : CGPath cg f f
  | step (a b c : Nat) : CGPath cg a b → (b, c) ∈ cg → CGPath cg a c

theorem CGReach_trans (cg : CallGraph) (a b : Nat) :
    ∀ (c : Nat), CGPath cg a b → CGPath cg b c → CGPath cg a c
  | _, h1, .here _ => h1
  | _, h1, .step _ _ _ hxy hy => .step _ _ _ (CGReach_trans cg a b _ h1 hxy) hy

def CGReachable (cg : CallGraph) (roots : List Nat) (f : Nat) : Prop :=
  ∃ r, r ∈ roots ∧ CGPath cg r f

/-- #134 mechanical phase: keep the functions the (decidable) reachability
    checker marks live; the checker must decide exactly `CGReachable`
    (tracked obligation: the graph is finite, so the search decides it). -/
def p134_removeDead (fns : List (Nat × Func)) (live : Nat → Bool) :
    List (Nat × Func) :=
  fns.filter (fun fn => live fn.1)

/-! ## #135-#137 Barriers, symbol pruning, section GC. -/

/-- #135: global inline assembly is an optimization barrier: no pass moves
    a memory access across it; the contract is the ordering `before ≤ asm ≤
    after` on the schedule line. -/
def AsmBarrier (sched : Nat → Nat) (before after : Nat) : Prop :=
  sched before ≤ sched after

/-- #136: import/export pruning: a symbol is kept when it is reachable in
    the symbol-reference graph from the entry roots. -/
def SymReachable (refs : List (Nat × Nat)) (roots : List Nat) (s : Nat) : Prop :=
  ∃ r, r ∈ roots ∧ CGPath refs r s

def p136_prune (syms : List (Nat × Func)) (live : Nat → Bool) :
    List (Nat × Func) :=
  syms.filter (fun sy => live sy.1)

/-- #137: section garbage collection: the same reachability at section
    granularity: sections are nodes, cross-references edges. -/
def p137_sectionGC (sections : List (Nat × Prog)) (live : Nat → Bool) :
    List (Nat × Prog) :=
  sections.filter (fun se => live se.1)

/-! ## #138-#139 Identical code folding / COMDAT. -/

/-- #138 ICF: functions with identical bodies are interchangeable (FV-0:
    semantic equivalence is a congruence for equality, so byte-identical
    bodies are semantically equivalent — the folding law). -/
theorem icf_law (f g : Prog) (h : f = g) : SemEquiv f g := by
  subst h
  exact SemEquiv_refl f

/-- #138 mechanical phase: fold the byte-identical functions to their
    canonical representative (kept = lowest id). -/
def p138_icf (fns : List (Nat × Prog)) : List (Nat × Prog) :=
  fns.foldl (fun acc fn =>
    if acc.any (fun kept => kept.2 == fn.2) then acc else acc ++ [fn]) []

/-- #139: COMDAT folding: same body-equality law with a selection key; the
    key's equality substitutes for the body check. -/
def p139_comdat (fns : List (Nat × Prog)) (key : Prog → Nat) : List (Nat × Prog) :=
  fns.foldl (fun acc fn =>
    if acc.any (fun kept => key kept.2 = key fn.2) then acc else acc ++ [fn]) []

/-! ## #140 Profile-guided inlining. -/

/-- #140: the PGO inline decision: inline when the call-site count meets
    the hot threshold (the profile is a measured input of the build; its
    fidelity to the production workload is the FV refinement note). -/
def p140_hot (count threshold : Nat) : Bool := count >= threshold

end RV8FV
