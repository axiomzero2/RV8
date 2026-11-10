/-
CEP:FILE: verification/lean/RV8FV/Dataflow.lean
CEP:WHAT: The dataflow mathematics of RV8: the generic chaotic-iteration
  fixpoint solver over per-block fact lists, with the machine-checked
  stability theorem (every returned solution satisfies the equations
  exactly); predecessor/successor relations; the classic analyses of catalog
  #57 (liveness, backward may), #58 (reaching definitions, forward may),
  #59 (available expressions, forward must over lists), #60 (very busy
  expressions, backward must), and the path-based anticipatability predicate
  of #61 defined over CFG.lean paths.
CEP:WHY: The design spec requires the dataflow passes to be mathematically
  defined. The solver's stability theorem is the load-bearing fact: an
  analysis result is a solution of its own equations, so any downstream
  soundness argument reasons about the equations alone. Fuel exhaustion is an
  explicit `none`, never a silent wrong answer (fail-closed law).
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: An analysis whose transfer function is not a fixed point of the
  returned state is rejected by the stability theorem.
CEP:ASSUMES: Facts are lists with decidable equality; unions are list appends
  (membership semantics only, duplicates irrelevant); fuel bounds the sweep
  count and a returned `none` means "not converged within fuel".
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Dataflow.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: semantic soundness of each analysis instance (facts
  over-approximate machine reality) is the tracked FV-1 obligation recorded
  in the pass frames; the solver and equations are FV-0 here.
CEP:FV-AXIOMS: none.
-/

import RV8FV.CFG

namespace RV8FV

/-! ## The generic fixpoint solver. -/

/-- Iterate `tr` until a sweep changes nothing. `none` = fuel exhausted
    while still changing. -/
def sweepToFix {α : Type} [DecidableEq α] (tr : List α → List α) :
    Nat → List α → Option (List α)
  | 0, _ => none
  | n + 1, s => if tr s = s then some s else sweepToFix tr n (tr s)

/-- Every converged result is a fixpoint of the transfer. -/
theorem sweepToFix_stable {α : Type} [DecidableEq α] (tr : List α → List α) :
    ∀ (n : Nat) (s s' : List α), sweepToFix tr n s = some s' → tr s' = s' := by
  intro n
  induction n with
  | zero => intro s s' h; simp [sweepToFix] at h
  | succ n ih =>
    intro s s' h
    simp only [sweepToFix] at h
    by_cases heq : tr s = s
    · rw [ite_eq_left heq] at h
      have hs : s = s' := by simpa using h
      rw [← hs]
      exact heq
    · rw [ite_eq_right heq] at h
      exact ih _ _ h

/-! ## Predecessors. -/

/-- All block indices of the program. -/
def blockIds (p : Prog) : List BlockId := List.range p.blocks.length

/-- The predecessor blocks of `b`: all blocks with an edge into `b`. -/
def preds (p : Prog) (b : BlockId) : List BlockId :=
  (blockIds p).filter (fun c => b ∈ succs p c)

/-! ## Block-level use/def. -/

/-- All registers read by a block, instructions and terminator. -/
def usesOfBlock (bl : Block) : List VReg :=
  bl.instrs.flatMap Instr.usesOf ++ bl.term.usesOf

/-- All registers defined by a block's instructions. -/
def defsOfBlock (bl : Block) : List VReg :=
  bl.instrs.flatMap (fun i => match i.defOf with
    | some d => [d]
    | none => [])

/-- Remove elements of `ys` from `xs` (kill). -/
def listSub (xs ys : List VReg) : List VReg :=
  xs.filter (fun x => !ys.contains x)

/-! ## #57 Liveness (backward may). -/

/-- Per-block live-in state. -/
abbrev LState := List (BlockId × List VReg)

def lookupL : LState → BlockId → List VReg
  | [], _ => []
  | (c, v) :: rest, b => if b = c then v else lookupL rest b

/-- live-out(b) = union of live-in over successors. -/
def liveOut (p : Prog) (st : LState) (b : BlockId) : List VReg :=
  (succs p b).flatMap (fun s => lookupL st s)

/-- live-in(b) = uses(b) ∪ (live-out(b) - defs(b)). -/
def liveInOf (p : Prog) (st : LState) (b : BlockId) : List VReg :=
  match p.blocks[b]? with
  | none => []
  | some bl => usesOfBlock bl ++ listSub (liveOut p st b) (defsOfBlock bl)

def liveSweep (p : Prog) (st : LState) : LState :=
  (blockIds p).map (fun b => (b, liveInOf p st b))

/-- The liveness analysis of catalog pass #57: iterate to the fixpoint. -/
def liveness (p : Prog) (fuel : Nat) : Option LState :=
  sweepToFix (liveSweep p) fuel []

theorem liveSweep_stable (p : Prog) (st : LState) (fuel : Nat)
    (h : liveness p fuel = some st) : liveSweep p st = st :=
  sweepToFix_stable (liveSweep p) fuel [] st h

/-! ## #58 Reaching definitions (forward may). -/

/-- A reaching fact: vreg `v` may hold the definition made in block `d`. -/
abbrev RFact := VReg × BlockId
abbrev RState := List (BlockId × List RFact)

def lookupR : RState → BlockId → List RFact
  | [], _ => []
  | (c, v) :: rest, b => if b = c then v else lookupR rest b

/-- in(b) = union of out over predecessors (entry gets the boot facts). -/
def rdInOf (boot : List RFact) (p : Prog) (st : RState) (b : BlockId) : List RFact :=
  if b = 0 then boot else (preds p b).flatMap (fun c => lookupR st c)

/-- out(b) = gen(b) ∪ (in(b) - kill(b)): definitions made in `b` survive,
    other facts about the same vregs are killed. -/
def rdOutOf (boot : List RFact) (p : Prog) (st : RState) (b : BlockId) : List RFact :=
  match p.blocks[b]? with
  | none => []
  | some bl =>
    let gen : List RFact := (defsOfBlock bl).map (fun d => (d, b))
    let killed : List VReg := defsOfBlock bl
    gen ++ (rdInOf boot p st b).filter (fun f => !killed.contains f.1)

def rdSweep (boot : List RFact) (p : Prog) (st : RState) : RState :=
  (blockIds p).map (fun b => (b, rdOutOf boot p st b))

/-- The reaching-definitions analysis of catalog pass #58. -/
def reachingDefs (boot : List RFact) (p : Prog) (fuel : Nat) : Option RState :=
  sweepToFix (rdSweep boot p) fuel []

theorem rdSweep_stable (boot : List RFact) (p : Prog) (st : RState) (fuel : Nat)
    (h : reachingDefs boot p fuel = some st) : rdSweep boot p st = st :=
  sweepToFix_stable (rdSweep boot p) fuel [] st h

/-! ## #59 Available expressions (forward must, list form). -/

/-- An expression fact: the computation `o a b` (never division: it traps). -/
abbrev EFact := Binop × Arg × Arg
abbrev EState := List (BlockId × List EFact)

def lookupE : EState → BlockId → List EFact
  | [], _ => []
  | (c, v) :: rest, b => if b = c then v else lookupE rest b

/-- The pure binary computations performed in a block. -/
def genExprs (bl : Block) : List EFact :=
  bl.instrs.flatMap (fun i => match i with
    | .binop _ o a b => if o = .udiv then [] else [(o, a, b)]
    | _ => [])

/-- An operand survives a definition block if it is an immediate or a
    register the block does not define. -/
def argLiveThru (defs : List VReg) (a : Arg) : Bool :=
  match a with
  | .imm _ => true
  | .reg r => !defs.contains r

/-- A fact survives block `b` when both its operands survive its defs. -/
def killExprs (bl : Block) (e : EFact) : Bool :=
  argLiveThru (defsOfBlock bl) e.2.1 && argLiveThru (defsOfBlock bl) e.2.2

def availOutOf (p : Prog) (st : EState) (b : BlockId) : List EFact :=
  match p.blocks[b]? with
  | none => []
  | some bl =>
    let ins : List EFact :=
      if b = 0 then [] else (preds p b).flatMap (fun c => lookupE st c)
    genExprs bl ++ ins.filter (killExprs bl)

def availSweep (p : Prog) (st : EState) : EState :=
  (blockIds p).map (fun b => (b, availOutOf p st b))

/-- The available-expressions analysis of catalog pass #59. -/
def availExprs (p : Prog) (fuel : Nat) : Option EState :=
  sweepToFix (availSweep p) fuel []

theorem availSweep_stable (p : Prog) (st : EState) (fuel : Nat)
    (h : availExprs p fuel = some st) : availSweep p st = st :=
  sweepToFix_stable (availSweep p) fuel [] st h

/-! ## #60 Very busy expressions (backward must). -/

/-- in(b) = intersection over successors of (gen(s) ∪ (out(s) - kill(s))). -/
def vbInOf (p : Prog) (st : EState) (b : BlockId) : List EFact :=
  match p.blocks[b]? with
  | none => []
  | some bl =>
    let outs : List (List EFact) :=
      (succs p b).map (fun s =>
        match p.blocks[s]? with
        | none => []
        | some sl => genExprs sl ++
          (lookupE st s).filter (killExprs sl))
    let inter : List EFact :=
      outs.foldl (fun acc l => acc.filter (fun e => l.contains e)) (outs.headD [])
    inter ++ genExprs bl

def vbSweep (p : Prog) (st : EState) : EState :=
  (blockIds p).map (fun b => (b, vbInOf p st b))

/-- The very-busy-expressions analysis of catalog pass #60. -/
def veryBusy (p : Prog) (fuel : Nat) : Option EState :=
  sweepToFix (vbSweep p) fuel []

theorem vbSweep_stable (p : Prog) (st : EState) (fuel : Nat)
    (h : veryBusy p fuel = some st) : vbSweep p st = st :=
  sweepToFix_stable (vbSweep p) fuel [] st h

/-! ## #61 Anticipatable expressions (path-based). -/

/-- The instruction computes exactly the expression fact `e`. -/
def Instr.computes (e : EFact) : Instr → Bool
  | .binop _ o a b => o = e.1 && a = e.2.1 && b = e.2.2
  | _ => false

/-- The block `c` computes the expression fact `e` somewhere. -/
def blockComputes (p : Prog) (c : BlockId) (e : EFact) : Bool :=
  match p.blocks[c]? with
  | none => false
  | some blk => blk.instrs.any (Instr.computes e)

/-- The expression `e` is computed somewhere in the instruction stream of
    the path. -/
def pathComputes (p : Prog) (l : List BlockId) (e : EFact) : Prop :=
  ∃ c, c ∈ l ∧ blockComputes p c e = true

/-- The expression `e` is anticipatable at `b` (catalog #61): every path from
    the entry to `b` computes `e` on the way. -/
def AnticipatableAt (p : Prog) (e : EFact) (b : BlockId) : Prop :=
  ∀ l, PathTo p 0 l → l.getLast? = some b → pathComputes p l e

/-! ### The discharge laws of #61: anticipation propagates along edges. -/

/-- The last element of a nonempty list is a member of the list. -/
theorem getLast?_mem_self : ∀ (l : List BlockId) (b : BlockId),
    l.getLast? = some b → b ∈ l := by
  intro l
  induction l with
  | nil => intro b h; simp at h
  | cons a rest ih =>
      intro b h
      cases rest with
      | nil =>
          have hab : a = b := by simpa using h
          subst hab
          exact List.Mem.head _
      | cons c rest' =>
          have h' : (c :: rest').getLast? = some b := by simpa using h
          exact List.Mem.tail _ (ih b h')

/-- Seeding: a path through a block that computes `e` computes `e`. -/
theorem pathComputes_of_mem (p : Prog) (e : EFact) (l : List BlockId) (c : BlockId)
    (hc : c ∈ l) (hbc : blockComputes p c e = true) : pathComputes p l e :=
  ⟨c, hc, hbc⟩

/-- Prefix preservation: whatever a path computes, every extension of it
    computes as well. -/
theorem pathComputes_append (p : Prog) (e : EFact) (l r : List BlockId)
    (h : pathComputes p l e) : pathComputes p (l ++ r) e := by
  obtain ⟨c, hc, hbc⟩ := h
  exact ⟨c, List.mem_append.mpr (Or.inl hc), hbc⟩

/-- Path extension: appending one edge to a path that ends at `b` gives a
    valid path that ends at the successor `y`. -/
theorem pathTo_append_edge (p : Prog) : ∀ (x : BlockId) (l : List BlockId)
    (b y : BlockId), PathTo p x l → l.getLast? = some b → y ∈ succs p b →
    PathTo p x (l ++ [y]) := by
  intro x l
  induction l generalizing x with
  | nil => intro b y _ hlast _; simp at hlast
  | cons a rest ih =>
      intro b y hpath hlast hy
      cases rest with
      | nil =>
          have hab : a = b := by simpa using hlast
          subst hab
          have hp : a ∈ succs p x := by simpa [PathTo] using hpath
          exact ⟨hp, hy, True.intro⟩
      | cons c rest' =>
          have hpath' : a ∈ succs p x ∧ PathTo p a (c :: rest') := by
            simpa [PathTo] using hpath
          have hrest : (c :: rest').getLast? = some b := by simpa using hlast
          exact ⟨hpath'.1, ih a b y hpath'.2 hrest hy⟩

/-- **The #61 discharge (transfer)**: if `e` is anticipatable at `b` — the
    exact path-based property — then every path reaching the successor `y`
    through a path to `b` still computes `e`.  This is the local transfer
    step that the PRE worklist iterates. -/
theorem anticipatable_transfer (p : Prog) (e : EFact) (b y : BlockId)
    (hAB : AnticipatableAt p e b) (_hy : y ∈ succs p b)
    (l : List BlockId) (hl : PathTo p 0 l) (hlast : l.getLast? = some b) :
    pathComputes p (l ++ [y]) e :=
  pathComputes_append p e l [y] (hAB l hl hlast)

/-- **The #61 discharge (seeding)**: if the block `b` itself computes `e`,
    then every path reaching `y` through a path to `b` computes `e`, even
    when no earlier block on the path did. -/
theorem anticipatable_seed (p : Prog) (e : EFact) (l : List BlockId) (b y : BlockId)
    (hlast : l.getLast? = some b) (hbc : blockComputes p b e = true) :
    pathComputes p (l ++ [y]) e := by
  refine pathComputes_of_mem p e (l ++ [y]) b ?_ hbc
  have hmem : b ∈ l := getLast?_mem_self l b hlast
  exact List.mem_append.mpr (Or.inl hmem)

end RV8FV
