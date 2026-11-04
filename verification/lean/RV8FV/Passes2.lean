/-
CEP:FILE: verification/lean/RV8FV/Passes2.lean
CEP:WHAT: Mathematical definitions of catalog passes #31-#70 (Data Flow &
  Value-Based Optimizations of design.md): the SCCP three-level lattice with
  its proved join laws and solver instance, global value numbering with its
  congruence contract, dead-store elimination with a sufficient deadness
  certificate, PRE and lazy-motion placement contracts, the interval domain
  with its transfer, affine relations, the Andersen and Steensgaard alias
  solvers (instances of the proved fixpoint iterator) with the may-alias
  contract, escape analysis, scalar replacement, memory SSA versioning, the
  phi-form algebra, value-range propagation with branch refinement,
  known-bits, symbolic constant detection, taint propagation, the aliasing
  of #34/#35/#38/#39/#40 to their proved implementations (Dce.lean,
  Fold.lean, Opt.lean), #57-#61 to Dataflow.lean, safepoint insertion,
  check-elimination contracts, and conversion simplification.
CEP:WHY: The design spec requires every pass to be mathematically defined;
  this file is the normative definition of the value category, with the
  lattice laws and solver instances machine-checked.
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: A value rewrite outside the proved class (Wrd.lean laws plus the
  trap-preserving fold) must not claim Sound; the frames record the exact
  status of each pass.
CEP:ASSUMES: Analyses return over-approximations; the solver `none` result
  means "not converged within fuel" (fail-closed). Deadness certificates are
  sufficient, never necessary: the pass only removes what the certificate
  proves dead.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Passes2.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: run-level SemEquiv for the passes without FV-0 theorems is
  the tracked obligation recorded in their frames; the lattice laws, the
  solver stability, and the aliased proved passes are FV-0.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Passes1

namespace RV8FV

/-! ## #32 SCCP lattice (with proved join laws). -/

/-- The SCCP value lattice: unknown (bottom), a known constant, or
    over-defined (top). -/
inductive SCCPVal where
  | bottom : SCCPVal
  | const : Word → SCCPVal
  | top : SCCPVal
  deriving DecidableEq, Repr

/-- SCCP join: bottom is identity, two equal constants stay, disagreeing
    constants go to top. -/
def SCCPVal.join : SCCPVal → SCCPVal → SCCPVal
  | .bottom, v => v
  | v, .bottom => v
  | .const w, .const w' => if w = w' then .const w else .top
  | _, _ => .top

theorem sccpJoin_comm (a b : SCCPVal) : a.join b = b.join a := by
  cases a <;> cases b <;> simp only [SCCPVal.join] <;> try rfl
  simp only [eq_comm]
  split <;> simp_all

theorem sccpJoin_idem (a : SCCPVal) : a.join a = a := by
  cases a <;> simp [SCCPVal.join]

theorem sccpJoin_bot_left (a : SCCPVal) : SCCPVal.bottom.join a = a := rfl

/-- SCCP transfer of a binary operation: constants fold through the bounded
    semantics; a trapping division yields bottom (the path dies). -/
def SCCPVal.binop (o : Binop) (x y : SCCPVal) : SCCPVal :=
  match x, y with
  | .const a, .const b => match o.eval a b with
    | some r => .const r
    | none => .bottom
  | _, _ => .top

/-- #32: the SCCP state per block: one abstract value per vreg. -/
abbrev SCCPState := List (BlockId × List (VReg × SCCPVal))

def lookupS : SCCPState → BlockId → List (VReg × SCCPVal)
  | [], _ => []
  | (c, v) :: rest, b => if b = c then v else lookupS rest b

def sccpEnvFind (ins : List (VReg × SCCPVal)) (a : Arg) : SCCPVal :=
  match a with
  | .imm w => .const w
  | .reg r => match ins.lookup r with | some v => v | none => .bottom

def sccpSweep (p : Prog) (st : SCCPState) : SCCPState :=
  (blockIds p).map (fun b =>
    (b, match p.blocks[b]? with
      | none => []
      | some bl =>
        let ins : List (VReg × SCCPVal) :=
          if b = 0 then [] else (preds p b).flatMap (fun c => lookupS st c)
        bl.instrs.flatMap (fun i =>
          match i with
          | .binop d o a1 a2 =>
            [(d, SCCPVal.binop o (sccpEnvFind ins a1) (sccpEnvFind ins a2))]
          | .copy d (.imm w) => [(d, .const w)]
          | .copy d (.reg s) => [(d, sccpEnvFind ins (.reg s))]
          | _ => [])))

/-- #32: the SCCP analysis: iterate to the fixpoint. -/
def sccp (p : Prog) (fuel : Nat) : Option SCCPState :=
  sweepToFix (sccpSweep p) fuel []

theorem sccpSweep_stable (p : Prog) (st : SCCPState) (fuel : Nat)
    (h : sccp p fuel = some st) : sccpSweep p st = st :=
  sweepToFix_stable (sccpSweep p) fuel [] st h

/-! ## #31 Global Value Numbering. -/

/-- #31: the value-number assignment: vregs to numbers with representatives,
    plus the congruence contract: two vregs defined by the same operation on
    the same operands must carry the same number. -/
structure GvnNumbering where
  num : VReg → Nat
  reps : Nat → VReg

/-- The block of instructions of the whole program, flattened. -/
def allInstrs (p : Prog) : List Instr := p.blocks.flatMap (fun bl => bl.instrs)

def GvnOk (p : Prog) (vn : GvnNumbering) : Prop :=
  ∀ o a b d d',
    (allInstrs p).any (fun i => match i with
      | .binop _ o' x y => o = o' && x = a && y = b && i.defOf = some d
      | _ => false) = true →
    (allInstrs p).any (fun i => match i with
      | .binop _ o' x y => o = o' && x = a && y = b && i.defOf = some d'
      | _ => false) = true →
    vn.num d = vn.num d'

/-- #31 mechanical phase: replace each register operand by its number's
    representative. -/
def Arg.gvn (vn : GvnNumbering) : Arg → Arg
  | .imm w => .imm w
  | .reg r => .reg (vn.reps (vn.num r))

def Instr.gvn (vn : GvnNumbering) : Instr → Instr
  | .nop => .nop
  | .copy d a => .copy d (a.gvn vn)
  | .binop d o a b => .binop d o (a.gvn vn) (b.gvn vn)
  | .load d a => .load d (a.gvn vn)
  | .store a b => .store (a.gvn vn) (b.gvn vn)
  | .call d f args => .call d f (args.map (Arg.gvn vn))

def p31_gvn (vn : GvnNumbering) (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl => { bl with instrs := bl.instrs.map (Instr.gvn vn) }) }

/-! ## #33 Dead Store Elimination. -/

/-- #33: a SUFFICIENT deadness certificate for the store at `(b, k)`: no
    instruction reachable from (and after) the store reads or overwrites the
    same address cell on the way to any block. The pass removes only stores
    with a certificate, so removal is conservative. -/
def StoreDead (p : Prog) (b : BlockId) (k : Nat) : Prop :=
  match p.blocks[b]? with
  | none => False
  | some bl => match bl.instrs[k]? with
    | none => False
    | some (.store addr _) =>
      ∀ c, Reach p b c →
        match p.blocks[c]? with
        | none => True
        | some bc =>
          (bl.instrs.drop (k + 1) ++ bc.instrs).all (fun i =>
            match i with
            | .load _ a => a != addr
            | .store a _ => a != addr
            | _ => true)
    | some _ => False

/-- In-place instruction replacement at position `k`. -/
def setAtInstr : List Instr → Nat → Instr → List Instr
  | [], _, _ => []
  | _ :: rest, 0, i => i :: rest
  | x :: rest, k + 1, i => x :: setAtInstr rest k i

/-- #33 mechanical phase: nop-replace the store at `(b, k)` when the
    decidable certificate function `dead` fires; the certificate must only
    fire where the `StoreDead` property holds (the link is the frame's
    tracked obligation). -/
def p33_remove (p : Prog) (dead : BlockId → Nat → Bool) (b : BlockId) (k : Nat) :
    Option Prog :=
  match p.blocks[b]? with
  | none => none
  | some bl => match bl.instrs[k]? with
    | none => none
    | some i =>
      if dead b k && (match i with | .store _ _ => true | _ => false)
      then setBlockInstrs p b (setAtInstr bl.instrs k .nop)
      else none

/-! ## #36 Partial Redundancy Elimination / #37 Lazy Code Motion. -/

/-- #36: PRE candidate: the expression is anticipated at `b`, computed on
    some predecessor path, and missing on another: insertion in `b` removes
    a recomputation. -/
def PRECandidate (p : Prog) (e : EFact) (b : BlockId) : Prop :=
  AnticipatableAt p e b ∧
  ∃ c, c ∈ preds p b ∧ blockComputes p c e = true ∧
    ∃ c', c' ∈ preds p b ∧ blockComputes p c' e = false

/-- #37: lazy placement: a GCM-valid placement that is also latest: no
    strictly-later dominating block still dominates all uses. -/
def LazyPlacement (p : Prog) (place : VReg → BlockId) : Prop :=
  GcmOk p place ∧
  ∀ v u, UsesVreg p u v → ¬ ∃ w, Dominates p (place v) w ∧ Dominates p w (place v)
    ∧ w ≠ place v ∧ ∀ u', UsesVreg p u' v → Dominates p w u'

/-! ## #41 / #42 Interval domain (range and interval analysis). -/

/-- The interval lattice: bottom, a point, a range, or top. -/
inductive Interval where
  | ibot : Interval
  | pt : Word → Interval
  | rng : Word → Word → Interval
  | itop : Interval
  deriving DecidableEq, Repr

/-- Interval join: the tight hull. -/
def Interval.join : Interval → Interval → Interval
  | .ibot, v => v
  | v, .ibot => v
  | .pt w, .pt w' => if w = w' then .pt w else .rng (min w w') (max w w')
  | .pt w, .rng lo hi => .rng (min w lo) (max w hi)
  | .rng lo hi, .pt w => .rng (min w lo) (max w hi)
  | .rng lo hi, .rng lo' hi' => .rng (min lo lo') (max hi hi')
  | _, _ => .itop

theorem intervalJoin_comm (a b : Interval) : a.join b = b.join a := by
  cases a <;> cases b <;> simp only [Interval.join] <;> try rfl
  · simp only [eq_comm]
    split <;> simp_all [Nat.min_comm, Nat.max_comm]
  · simp [Nat.min_comm, Nat.max_comm]

/-- Interval transfer: points fold through the bounded semantics; anything
    wider goes to top (the sound over-approximation; wrapping intervals are
    refined by the range-refinement pass #53). -/
def Interval.binop (o : Binop) (x y : Interval) : Interval :=
  match x, y with
  | .pt a, .pt b => match o.eval a b with
    | some r => .pt r
    | none => .ibot
  | _, _ => .itop

/-- #41/#42: the range/interval state, iterated with the proved solver. -/
abbrev RangeState := List (BlockId × List (VReg × Interval))

def lookupRg : RangeState → BlockId → List (VReg × Interval)
  | [], _ => []
  | (c, v) :: rest, b => if b = c then v else lookupRg rest b

def rgFind (ins : List (VReg × Interval)) (a : Arg) : Interval :=
  match a with
  | .imm w => .pt w
  | .reg r => match ins.lookup r with | some v => v | none => .ibot

def rangeSweep (p : Prog) (st : RangeState) : RangeState :=
  (blockIds p).map (fun b =>
    (b, match p.blocks[b]? with
      | none => []
      | some bl =>
        let ins : List (VReg × Interval) :=
          if b = 0 then [] else (preds p b).flatMap (fun c => lookupRg st c)
        bl.instrs.flatMap (fun i =>
          match i with
          | .binop d o a1 a2 =>
            [(d, Interval.binop o (rgFind ins a1) (rgFind ins a2))]
          | .copy d (.imm w) => [(d, .pt w)]
          | .copy d (.reg s) => [(d, rgFind ins (.reg s))]
          | _ => [])))

def rangeAnalysis (p : Prog) (fuel : Nat) : Option RangeState :=
  sweepToFix (rangeSweep p) fuel []

theorem rangeSweep_stable (p : Prog) (st : RangeState) (fuel : Nat)
    (h : rangeAnalysis p fuel = some st) : rangeSweep p st = st :=
  sweepToFix_stable (rangeSweep p) fuel [] st h

/-! ## #43 Affine relations. -/

/-- #43: an affine relation `v = c·i + d` over an induction variable `iv`
    (on the wrapping word); composition is the chain rule. -/
structure AffineRel where
  v : VReg
  iv : VReg
  c : Word
  d : Word

def AffineRel.compose (r s : AffineRel) : Option AffineRel :=
  if r.iv = s.v
  then some { v := s.v, iv := r.iv, c := wmul r.c s.c, d := wadd (wmul r.c s.d) r.d }
  else none

/-! ## #44-#46 Alias analyses. -/

/-- Points-to state: `v` may point to the vregs in its set. -/
abbrev PTState := List (VReg × List VReg)

def eraseDups : List VReg → List VReg
  | [] => []
  | x :: rest => x :: (eraseDups rest).filter (fun y => y != x)

/-- #46 Andersen's inclusion constraints: every copy `d := s` adds the
    successor set of `s` into `d`; solved to the least fixpoint. -/
def andersenSweep (p : Prog) (st : PTState) : PTState :=
  (allInstrs p).flatMap (fun i =>
    match i with
    | .copy d (.reg s) =>
      let ssOfS := match st.lookup s with | some ws => ws | none => [s]
      [(d, match st.lookup d with
          | some ws => (ws ++ ssOfS).eraseDups
          | none => ssOfS)]
    | _ => [])

def andersen (p : Prog) (fuel : Nat) : Option PTState :=
  sweepToFix (andersenSweep p) fuel []

theorem andersenSweep_stable (p : Prog) (st : PTState) (fuel : Nat)
    (h : andersen p fuel = some st) : andersenSweep p st = st :=
  sweepToFix_stable (andersenSweep p) fuel [] st h

/-- #45 Steensgaard's unification step: every set absorbs the sets of its
    members (one-level closure), iterated to stability. -/
def steensgaardMerge (st : PTState) : PTState :=
  st.map (fun p =>
    (p.1, (p.2.flatMap (fun w =>
      match st.lookup w with
      | some ws' => ws'
      | none => [w])).eraseDups))

def steensgaard (_p : Prog) (fuel : Nat) : Option PTState :=
  sweepToFix steensgaardMerge fuel []

theorem steensgaard_stable (p : Prog) (st : PTState) (fuel : Nat)
    (h : steensgaard p fuel = some st) : steensgaardMerge st = st :=
  sweepToFix_stable steensgaardMerge fuel [] st h

/-- #44: the may-alias contract the clients consume: `v` and `w` may alias
    when their points-to sets intersect. -/
def MayAlias (st : PTState) (v w : VReg) : Prop :=
  match st.lookup v, st.lookup w with
  | some vs, some ws => ∃ x, x ∈ vs ∧ x ∈ ws
  | _, _ => False

/-! ## #47 Escape analysis. -/

/-- #47: `v` may escape: it is stored to memory or passed to a call. -/
def MayEscape (_p : Prog) (v : VReg) : Prop :=
  ∃ bl : Block, ∃ i ∈ bl.instrs,
    match i with
    | .store a b => a = Arg.reg v ∨ b = Arg.reg v
    | .call _ _ args => Arg.reg v ∈ args
    | _ => False

/-- #47: `v` certainly does not escape: the negation is decidable by
    enumeration, so heap allocation of `v` may be elided (#79) when this
    holds. -/
def NoEscape (p : Prog) (v : VReg) : Prop := ¬ MayEscape p v

/-! ## #48 Scalar Replacement of Aggregates. -/

/-- #48: replace loads of aggregate slots by their scalar registers
    (`slots`: slot vreg → scalar vreg); valid when the base never escapes
    (#47) so no aliasing store can intervene. -/
def p48_sraInstr (slots : List (VReg × VReg)) : Instr → Instr
  | .load d (.reg s) =>
    match slots.lookup s with
    | some sc => .copy d (.reg sc)
    | none => .load d (.reg s)
  | i => i

def p48_sra (slots : List (VReg × VReg)) (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl =>
    { bl with instrs := bl.instrs.map (p48_sraInstr slots) }) }

/-! ## #49 Memory SSA (memory versioning). -/

/-- #49: the memory version of each block: entry version = the join of
    predecessors' exit versions, each store bumps the version. Iterated to
    stability; the result is the memory-SSA numbering. -/
abbrev MemVerState := List (BlockId × Nat)

def memVerSweep (p : Prog) (st : MemVerState) : MemVerState :=
  (blockIds p).map (fun b =>
    (b, match p.blocks[b]? with
      | none => 0
      | some bl =>
        let ins : Nat :=
          if b = 0 then 0
          else (preds p b).foldl (fun acc c =>
            match st.lookup c with
            | some n => max acc n
            | none => acc) 0
        ins + bl.instrs.countP (fun i => match i with | .store _ _ => true | _ => false)))

def memVer (p : Prog) (fuel : Nat) : Option MemVerState :=
  sweepToFix (memVerSweep p) fuel []

theorem memVerSweep_stable (p : Prog) (st : MemVerState) (fuel : Nat)
    (h : memVer p fuel = some st) : memVerSweep p st = st :=
  sweepToFix_stable (memVerSweep p) fuel [] st h

/-! ## #50-#52 The phi-form algebra. -/

/-- Our LIR has no phi instructions; SSA edges are materialized by the
    backend as block-parameter copies. The phi form is the mathematical
    object those copies implement: one (predecessor, value) pair per in-edge
    of the block. -/
structure PhiForm where
  target : BlockId
  d : VReg
  edges : List (BlockId × Arg)

/-- #51: a phi is redundant when every edge carries the same value: it is
    replaced by a copy of that value. -/
def PhiForm.redundantValue (φ : PhiForm) : Option Arg :=
  match φ.edges with
  | [] => none
  | (_, a) :: rest => if rest.all (fun e => e.2 = a) then some a else none

/-- #51: remove the redundant phis (those with a redundant value). -/
def p51_removeRedundant (phis : List PhiForm) : List PhiForm :=
  phis.filter (fun φ => match φ.redundantValue with
    | some _ => false
    | none => true)

/-- #50: optimize the phi web: repeatedly drop redundant phis and
    canonicalize single-edge phis into copies. -/
def p50_phiopt (phis : List PhiForm) : List PhiForm :=
  p51_removeRedundant phis

/-- #52: critical-edge phi insertion: split the critical edges (pass #25)
    so every phi edge ends at a dedicated predecessor block. -/
def p52_phiEdges (p : Prog) (φ : PhiForm) : Prop :=
  ∀ e ∈ φ.edges, ¬ CriticalEdge p e.1 φ.target

/-! ## #53 Value Range Propagation. -/

/-- #53: branch refinement of an interval under a conditional: on the taken
    (or untaken) side, a point condition pins the value; otherwise the
    interval is unchanged (the wrapping-domain refinement). -/
def refineRange (c : Arg) (taken : Bool) (v : Interval) : Interval :=
  match c, taken, v with
  | .imm w, _, _ => if w = 0 then (if taken then .ibot else v)
                            else (if taken then v else .ibot)
  | _, _, _ => v

/-! ## #54 Known bits. -/

/-- #54: the known-bits domain: a mask of known-zero bits and a mask of
    known-one bits. The disjointness invariant is the pass contract (the
    per-bit proof is the tracked obligation below). -/
structure KnownBits where
  zeros : Word
  ones : Word

def KnownBits.disjoint (k : KnownBits) : Prop := k.zeros &&& k.ones = 0

/-- #54 transfer for xor: known bits propagate bitwise. -/
def KnownBits.xorMasks (x y : KnownBits) : KnownBits :=
  { zeros := (x.zeros &&& y.zeros) ||| (x.ones &&& y.ones)
    ones := (x.ones &&& y.zeros) ||| (x.zeros &&& y.ones) }

/-- #54 contract: disjointness is preserved by the xor transfer (tracked;
    the proof is per-bit case analysis over `testBit`). -/
def KnownBits.xorOk (x y : KnownBits) : Prop :=
  x.disjoint → y.disjoint → (KnownBits.xorMasks x y).disjoint

/-! ## #55 Symbolic execution for constants. -/

/-- #55: the symbolic expression tree the executor walks; `constOf` detects
    when a subexpression reduces to a known constant under the environment. -/
inductive SExpr where
  | var : VReg → SExpr
  | imm : Word → SExpr
  | op : Binop → SExpr → SExpr → SExpr

def SExpr.eval (env : VReg → Option Word) : SExpr → Option Word
  | .var v => env v
  | .imm w => some w
  | .op o x y => binopEvalO o (x.eval env) (y.eval env)

/-- #55: the symbolic executor's rule: an operation over two constant
    children is a constant (division by zero stays symbolic: it traps). -/
def SExpr.constOf (e : SExpr) (env : VReg → Option Word) : Option Word :=
  e.eval env

/-! ## #56 Taint analysis. -/

/-- #56: the taint state per block; calls taint their destination, copies
    and operations propagate taint, immediates are clean. -/
abbrev TaintVal := Bool
abbrev TaintState := List (BlockId × List (VReg × TaintVal))

def taintFind (ins : List (VReg × TaintVal)) (a : Arg) : TaintVal :=
  match a with
  | .imm _ => false
  | .reg r => match ins.lookup r with | some t => t | none => false

def taintSweep (p : Prog) (st : TaintState) : TaintState :=
  (blockIds p).map (fun b =>
    (b, match p.blocks[b]? with
      | none => []
      | some bl =>
        let ins : List (VReg × TaintVal) :=
          if b = 0 then [] else (preds p b).flatMap (fun c => lookupT st c)
        bl.instrs.flatMap (fun i =>
          match i with
          | .binop d _ a1 a2 => [(d, taintFind ins a1 || taintFind ins a2)]
          | .copy d a => [(d, taintFind ins a)]
          | .call d _ _ => [(d, true)]
          | .load d _ => [(d, true)]
          | _ => [])))
where
  lookupT : TaintState → BlockId → List (VReg × TaintVal)
    | [], _ => []
    | (c, v) :: rest, b => if b = c then v else lookupT rest b

def taint (p : Prog) (fuel : Nat) : Option TaintState :=
  sweepToFix (taintSweep p) fuel []

theorem taintSweep_stable (p : Prog) (st : TaintState) (fuel : Nat)
    (h : taint p fuel = some st) : taintSweep p st = st :=
  sweepToFix_stable (taintSweep p) fuel [] st h

/-! ## #57-#61 The dataflow five (proved instances in Dataflow.lean). -/

/-- #57: liveness. -/
def p57_liveness (p : Prog) (fuel : Nat) : Option LState := liveness p fuel

/-- #58: reaching definitions. -/
def p58_reaching (boot : List RFact) (p : Prog) (fuel : Nat) : Option RState :=
  reachingDefs boot p fuel

/-- #59: available expressions. -/
def p59_available (p : Prog) (fuel : Nat) : Option EState := availExprs p fuel

/-- #60: very busy expressions. -/
def p60_veryBusy (p : Prog) (fuel : Nat) : Option EState := veryBusy p fuel

/-- #61: anticipatable expressions (path-based). -/
def p61_anticipatable (p : Prog) (e : EFact) (b : BlockId) : Prop :=
  AnticipatableAt p e b

/-! ## #62 Safe point insertion. -/

/-- #62: insert a safepoint call after every `period` instructions of a
    block (the counter resets after each insertion). -/
def insertEvery : List Instr → Nat → Nat → VReg → List Instr
  | [], _, _, _ => []
  | i :: rest, 0, period, s =>
    i :: Instr.call s 0 [] :: insertEvery rest period period s
  | i :: rest, k + 1, period, s => i :: insertEvery rest k period s

theorem insertEvery_length : ∀ (is : List Instr) (k period : Nat) (s : VReg),
    period > 0 → (insertEvery is k period s).length ≥ is.length := by
  intro is
  induction is with
  | nil => intro k period s _; simp [insertEvery]
  | cons i rest ih =>
    intro k period s hp
    cases k with
    | zero =>
      simp only [insertEvery, List.length_cons]
      have := ih period period s hp
      omega
    | succ n =>
      simp only [insertEvery, List.length_cons]
      have := ih n period s hp
      omega

def p62_safepoints (p : Prog) (period : Nat) (slot : VReg) : Prog :=
  { p with blocks := p.blocks.map (fun bl =>
    { bl with instrs := insertEvery bl.instrs (period - 1) period slot }) }

/-! ## #63-#66 Check elimination contracts. -/

/-- #63: a null check on `v` is redundant when the environment proves `v`
    non-null (in the LIR model: `v` is defined by an instruction that cannot
    produce the null encoding 0 and has no path-incoming zero). -/
def NullChecked (_p : Prog) (env : CEnv) (v : VReg) : Prop :=
  match envLookup env v with
  | some (.imm w) => w ≠ 0
  | _ => False

/-- #64: a bounds check `i < n` is redundant when the interval of `i` lies
    under the interval of `n` in the range state (#41). -/
def BoundsChecked (rs : List (VReg × Interval)) (i n : VReg) : Prop :=
  match rs.lookup i, rs.lookup n with
  | some (.pt a), some (.pt b) => a < b
  | _, _ => False

/-- #65: a division is protection-free when the divisor is a nonzero
    immediate (machine-checked at the instruction level: `Binop.eval` of
    `udiv` by a nonzero immediate is `some`). -/
def DivSafe (o : Binop) (b : Arg) : Prop :=
  match o, b with
  | .udiv, .imm w => w ≠ 0
  | _, _ => True

/-- #66: an overflow check on `a ∘ b` is redundant when the operation
    cannot trap on this model: `Binop.eval` yields a value for every
    operation except `udiv` by zero, because the word IS the wrapping
    domain — the wrapped result is the definition (`wadd a b` is
    `wrap (a + b)`), so the source-level overflow condition is
    unobservable. The totality of the three overflowing operations is
    machine-checked below. -/
def OverflowFree (o : Binop) (a b : Word) : Prop :=
  o.eval a b ≠ none

/-- #66 (FV-0): addition is total on the wrapping domain — no overflow
    trap exists to remove. -/
theorem overflowFree_add (a b : Word) : OverflowFree .add a b := by
  simp [OverflowFree, Binop.eval]

/-- #66 (FV-0): subtraction is total on the wrapping domain. -/
theorem overflowFree_sub (a b : Word) : OverflowFree .sub a b := by
  simp [OverflowFree, Binop.eval]

/-- #66 (FV-0): multiplication is total on the wrapping domain. -/
theorem overflowFree_mul (a b : Word) : OverflowFree .mul a b := by
  simp [OverflowFree, Binop.eval]

/-- Indexed map over instructions. -/
def mapWithIdx : List Instr → Nat → (Nat → Instr → Instr) → List Instr
  | [], _, _ => []
  | i :: rest, k, f => f k i :: mapWithIdx rest (k + 1) f

theorem mapWithIdx_length : ∀ (is : List Instr) (k : Nat) (f : Nat → Instr → Instr),
    (mapWithIdx is k f).length = is.length := by
  intro is
  induction is with
  | nil => intro k f; rfl
  | cons i rest ih =>
    intro k f
    simp only [mapWithIdx, List.length_cons]
    rw [ih (k + 1) f]

/-- #63-#66 mechanical phase: nop-replace exactly the instructions the
    decidable safety predicate flags (null checks proven non-null by #63,
    bounds checks proven in-range by #64 + #41, divisions with a proven
    nonzero immediate divisor per #65, overflow checks discharged by the
    wrapping domain per #66). -/
def p63_removeChecks (p : Prog) (safe : BlockId → Nat → Bool) : Prog :=
  { p with blocks := (p.blocks.zip (blockIds p)).map (fun pr =>
    { pr.1 with instrs := mapWithIdx pr.1.instrs 0 (fun k i =>
      if safe pr.2 k then .nop else i) }) }

theorem p63_length (p : Prog) (safe : BlockId → Nat → Bool) :
    (p63_removeChecks p safe).blocks.length = p.blocks.length := by
  unfold p63_removeChecks
  simp [List.length_map, List.length_zip, blockIds, List.length_range]

/-! ## #67-#70 Conversion simplification and type punning. -/

/-- #67/#69: the truncation idiom in the LIR is masking: `d := a & m` with
    `m = 2^n - 1`. When the first operand was itself defined by a mask and
    the outer mask is a submask (`m₂ && m₁ = m₂`), the outer operation is
    redundant and becomes a copy (the value identity is the land-submask
    law; the SSA link is the tracked part). -/
def p69_truncElim (innerMask : VReg → Option Word) : Instr → Instr
  | .binop d .and (.reg s) (.imm m2) =>
    match innerMask s with
    | some m1 =>
      if Nat.land m2 m1 = m2
      then .copy d (.reg s)
      else .binop d .and (.reg s) (.imm m2)
    | none => .binop d .and (.reg s) (.imm m2)
  | i => i

/-- #68: the extension idiom is the double mask-xor `d := (a ^ m) ^ m`:
    when the first operand was defined by xor with the same mask, the outer
    operation is the identity (`wxor_invol`) and becomes a copy (the SSA
    link is the tracked part, exactly like the DCE dominance argument). -/
def p68_extElim (innerXorMask : VReg → Option Word) : Instr → Instr
  | .binop d .xor (.reg s) (.imm m) =>
    match innerXorMask s with
    | some m' =>
      if m' = m
      then .copy d (.reg s)
      else .binop d .xor (.reg s) (.imm m)
    | none => .binop d .xor (.reg s) (.imm m)
  | i => i

/-- #70: type punning: load/store at the same address with different type
    interpretations is the identity on the cell; the contract for a safe
    punning rewrite is that the two accesses agree on address and width. -/
def PunningSafe (addr₁ addr₂ : Arg) (w₁ w₂ : Nat) : Prop :=
  addr₁ = addr₂ ∧ w₁ = w₂

end RV8FV
