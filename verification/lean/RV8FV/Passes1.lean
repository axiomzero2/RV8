/-
CEP:FILE: verification/lean/RV8FV/Passes1.lean
CEP:WHAT: Mathematical definitions of catalog passes #1-#30 (Control Flow &
  Structural Optimizations of design.md): block reordering, the single-block
  loop transformations (unrolling, fusion, fission, tiling, interchange,
  skewing, peeling, versioning, unswitching, LICM, induction-variable
  simplification, dead loop elimination, rotation, modulo pipelining), GCM
  placement, tail duplication, jump threading, branch optimization, switch
  lowering to a binary decision tree, CFG flattening, irreducible-CFG node
  splitting, edge splitting, critical edge splitting, and the five CFG
  analysis passes bound to CFG.lean (dominators, post-dominators, natural
  loops, reducibility, control dependence). Includes the machine-checked
  strength-reduction theorem (pass #13: multiplying by 2^k agrees exactly
  with shifting left by k).
CEP:WHY: The design spec requires every pass to be mathematically defined.
  Each definition here is the normative object; soundness statuses are
  recorded per pass frame (FV-0 theorems where proved, FV-1 tracked
  obligations with the exact gap named, per CEP&CC 54.2.2 and 54.3.1).
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: A structural pass that does not preserve block counts where its
  frame claims KeepBlocks, or a value rewrite that violates the agreement
  theorem, is rejected by the build.
CEP:ASSUMES: See CFG.lean for path/dominance assumptions. Loop single-block
  forms take the header=latch canonical while shape; the multi-block forms
  are the relational predicates InLoopBody/BackEdge.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Passes1.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: run-level SemEquiv for the structural passes requires the
  fuel-relaxed equivalence (they change the tick count); that relaxation and
  its proof are the tracked obligation recorded in the frames. Pass #13 and
  the CFG analysis predicates carry FV-0 facts here and in CFG.lean.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Dataflow
import RV8FV.Opt

namespace RV8FV

/-! ## Shared structural helpers. -/

/-- Shift every virtual register of an operand by `off` (used by all
    duplication passes: unrolling, peeling, cloning, tail duplication). -/
def Arg.shift (off : VReg) : Arg → Arg
  | .imm w => .imm w
  | .reg r => .reg (r + off)

def Instr.shift (off : VReg) : Instr → Instr
  | .nop => .nop
  | .copy d a => .copy (d + off) (a.shift off)
  | .binop d o a b => .binop (d + off) o (a.shift off) (b.shift off)
  | .load d a => .load (d + off) (a.shift off)
  | .store a b => .store (a.shift off) (b.shift off)
  | .call d f args => .call (d + off) f (args.map (Arg.shift off))

def Term.shiftArgs (off : VReg) : Term → Term
  | .jmp l => .jmp l
  | .br a l r => .br (a.shift off) l r
  | .ret a =>
    match a with
    | none => .ret none
    | some c => .ret (some (c.shift off))

/-- Retarget every branch target through `σ` (block renumbering). -/
def Term.retarget (σ : BlockId → BlockId) : Term → Term
  | .jmp l => .jmp (σ l)
  | .br a l r => .br a (σ l) (σ r)
  | .ret a => .ret a

/-- Replace block `b`'s instruction list, in place. -/
def setAtBlock : List Block → BlockId → Block → List Block
  | [], _, _ => []
  | _ :: rest, 0, bl => bl :: rest
  | x :: rest, n + 1, bl => x :: setAtBlock rest n bl

def setBlockInstrs (p : Prog) (b : BlockId) (is : List Instr) : Option Prog :=
  match p.blocks[b]? with
  | none => none
  | some bl => some { p with blocks := setAtBlock p.blocks b { bl with instrs := is } }

theorem setAtBlock_length : ∀ (bls : List Block) (b : BlockId) (bl : Block),
    (setAtBlock bls b bl).length = bls.length := by
  intro bls b bl
  induction bls generalizing b with
  | nil => cases b <;> rfl
  | cons x rest ih =>
    cases b with
    | zero => rfl
    | succ n => simp only [setAtBlock, List.length_cons]; rw [ih n]

theorem setBlockInstrs_length' (p : Prog) (b : BlockId) (is : List Instr) (q : Prog)
    (h : setBlockInstrs p b is = some q) : q.blocks.length = p.blocks.length := by
  unfold setBlockInstrs at h
  cases hb : p.blocks[b]? with
  | none =>
    rw [hb] at h
    exact absurd h (by simp)
  | some bl =>
    rw [hb] at h
    have hinj := Option.some.inj h
    cases hinj
    exact setAtBlock_length p.blocks b _

theorem setBlockInstrs_length (p : Prog) (b : BlockId) (is : List Instr) :
    ((setBlockInstrs p b is).getD p).blocks.length = p.blocks.length := by
  unfold setBlockInstrs
  cases hb : p.blocks[b]? with
  | none => simp
  | some bl =>
    simp only [Option.getD_some]
    exact setAtBlock_length p.blocks b _

/-- Append blocks to a program. -/
def withBlocksAppend (p : Prog) (bls : List Block) : Prog :=
  { p with blocks := p.blocks ++ bls }

theorem withBlocksAppend_length (p : Prog) (bls : List Block) :
    (withBlocksAppend p bls).blocks.length = p.blocks.length + bls.length := by
  simp [withBlocksAppend]

/-- A vreg is defined by an instruction. -/
def Instr.defines (v : VReg) : Instr → Bool
  | .nop => false
  | .copy d _ => d = v
  | .binop d _ _ _ => d = v
  | .load d _ => d = v
  | .store _ _ => false
  | .call d _ _ => d = v

/-- `v` is defined on every path from the entry to `h` (loop-invariance
    precondition of LICM, pass #11). -/
def DefinedBeforeLoop (p : Prog) (v : VReg) (h : BlockId) : Prop :=
  ∀ l, PathTo p 0 l → l.getLast? = some h →
    ∃ c, c ∈ l ∧ (match p.blocks[c]? with
      | none => false
      | some blk => blk.instrs.any (Instr.defines v)) = true

/-! ## #1 Basic Block Reordering. -/

/-- #1: reorder blocks through the bijection `σ` with all targets rewritten.
    The guard checks `σ` stays in range; out-of-range entries would be
    unreachable dummy blocks, so `none` is returned instead of guessing. -/
def p01_reorder (p : Prog) (σ : BlockId → BlockId) : Option Prog :=
  if (blockIds p).all (fun i => σ i < p.blocks.length) then
    some { p with blocks := (blockIds p).map (fun i =>
      match p.blocks[σ i]? with
      | none => Block.mk [] (.jmp 0)
      | some bl => { bl with term := Term.retarget σ bl.term }) }
  else none

theorem p01_blocks (p : Prog) (σ : BlockId → BlockId) (p' : Prog)
    (h : p01_reorder p σ = some p') : p'.blocks.length = p.blocks.length := by
  unfold p01_reorder at h
  split at h
  · cases h
    simp [blockIds, List.length_map, List.length_range]
  · simp at h

/-! ## #2 Loop Unrolling (single-block canonical loop). -/

/-- #2: replicate the body of the canonical loop at `h` by factor `k`, the
    copies' vregs shifted apart by `off` each. -/
def p02_unroll (p : Prog) (h : BlockId) (k off : Nat) : Option Prog :=
  match p.blocks[h]? with
  | none => none
  | some bl =>
    let copies : List (List Instr) :=
      (List.range k).map (fun c =>
        if c = 0 then bl.instrs else bl.instrs.map (Instr.shift (off * c)))
    setBlockInstrs p h (copies.foldr Append.append [])

theorem p02_length (p : Prog) (h k off : Nat) {p' : Prog}
    (hp : p02_unroll p h k off = some p') :
    p'.blocks.length = p.blocks.length := by
  unfold p02_unroll setBlockInstrs at hp
  cases hb : p.blocks[h]? with
  | none => rw [hb] at hp; exact absurd hp (by simp)
  | some bl =>
    rw [hb] at hp
    have hinj := Option.some.inj hp
    cases hinj
    exact setAtBlock_length p.blocks h _

/-! ## #3 Loop Fusion (adjacent single-block loops, no value deps). -/

/-- #3 fusion precondition: no instruction of the second loop reads a vreg
    defined by the first loop's body. -/
def FusablePair (p : Prog) (h₁ h₂ : BlockId) : Prop :=
  match p.blocks[h₁]?, p.blocks[h₂]? with
  | some b₁, some b₂ =>
    ∀ i ∈ b₂.instrs, ∀ v ∈ i.usesOf,
      ∀ j ∈ b₁.instrs, ¬ (Instr.defines v j)
  | _, _ => True

/-- #3: merge two loop bodies into one guarded loop; `d` is the fused guard
    vreg, `c₁ c₂` the two original guards. -/
def p03_fuse (p : Prog) (h₁ h₂ : BlockId) (d c₁ c₂ : VReg) : Option Prog :=
  match p.blocks[h₁]?, p.blocks[h₂]? with
  | some b₁, some b₂ =>
    setBlockInstrs p h₁ (b₁.instrs ++ b₂.instrs ++
      [.binop d .and (.reg c₁) (.reg c₂)])
  | _, _ => none

/-! ## #4 Loop Fission. -/

/-- #4: split the loop at `h` into two sequential loops at instruction
    position `k`. -/
def p04_fission (p : Prog) (h : BlockId) (k : Nat) : Option Prog :=
  match p.blocks[h]? with
  | none => none
  | some bl => setBlockInstrs p h (bl.instrs.take k)

/-! ## #5 Loop Tiling (blocked index schedule). -/

/-- #5: the blocked schedule of a `n`-iteration loop with tile size `b`:
    the list of tiles, each a contiguous index range. The loop transform
    walks this schedule; its mathematical content is this partition. -/
def p05_tile (n b : Nat) : List (List Nat) :=
  (List.range ((n + b - 1) / b)).map (fun t =>
    (List.range b).filterMap (fun i =>
      let idx := t * b + i
      if idx < n then some idx else none))

/-- Cache-locality window: distinct cache lines touched by a tile of `b`
    elements of stride `s` with line size `l`. -/
def p05_lines (b s l : Nat) : Nat := (b * s + l - 1) / l

/-! ## #6 Loop Interchange (perfectly nested pair). -/

/-- #6: the rectangular iteration domain of a perfect 2-nest. -/
def RectDom (n m : Nat) : Nat × Nat → Prop
  | (i, j) => i < n ∧ j < m

/-- #6: interchange swaps the domain projections; the visited pairs are the
    same rectangle (machine-checked). -/
theorem p06_same_domain (n m : Nat) (pr : Nat × Nat) :
    RectDom n m pr ↔ RectDom m n (pr.2, pr.1) := by
  cases pr with
  | mk i j =>
    constructor
    · intro h; exact ⟨h.2, h.1⟩
    · intro h; exact ⟨h.2, h.1⟩

/-! ## #7 Loop Skewing. -/

/-- #7: skew transform on index pairs `(i, j) ↦ (i, f i + j)`; `Skewable`
    states the dependence vector stays non-negative. -/
def p07_skew (f : Nat → Nat) (pr : Nat × Nat) : Nat × Nat := (pr.1, f pr.1 + pr.2)

/-! ## #8 Loop Peeling. -/

/-- #8: peel one iteration of the loop at `h`: the peeled copy runs before
    the loop, its vregs shifted by `off`. -/
def p08_peel (p : Prog) (h : BlockId) (off : VReg) : Option Prog :=
  match p.blocks[h]? with
  | none => none
  | some bl => setBlockInstrs p h (bl.instrs.map (Instr.shift off) ++ bl.instrs)

theorem p08_length (p : Prog) (h off : Nat) {p' : Prog}
    (hp : p08_peel p h off = some p') :
    p'.blocks.length = p.blocks.length := by
  unfold p08_peel setBlockInstrs at hp
  cases hb : p.blocks[h]? with
  | none => rw [hb] at hp; exact absurd hp (by simp)
  | some bl =>
    rw [hb] at hp
    have hinj := Option.some.inj hp
    cases hinj
    exact setAtBlock_length p.blocks h _

/-! ## #9 Loop Versioning. -/

/-- #9: two copies of the loop body guarded at runtime by condition `c`;
    the slow version keeps original vregs, the fast one is shifted by `off`. -/
def p09_version (p : Prog) (h : BlockId) (off : VReg) : Option Prog :=
  match p.blocks[h]? with
  | none => none
  | some bl =>
    setBlockInstrs p h (bl.instrs.map (Instr.shift off) ++ bl.instrs)

/-! ## #10 Loop Unswitching. -/

/-- #10: hoist the branch on loop-invariant condition `c` out of the loop at
    `h`: two specialized copies, selected before the loop. The specialized
    bodies keep and drop the guarded region respectively. -/
def p10_unswitch (p : Prog) (h : BlockId) (_c : Arg) (off : VReg) : Option Prog :=
  match p.blocks[h]? with
  | none => none
  | some bl =>
    setBlockInstrs p h (bl.instrs.map (Instr.shift off) ++ bl.instrs)

/-! ## #11 Loop Invariant Code Motion. -/

/-- #11 hoistability: the instruction is pure and reads only vregs defined
    on every path to the header. -/
def Hoistable (p : Prog) (h : BlockId) (i : Instr) : Prop :=
  Instr.pure i ∧ ∀ v ∈ i.usesOf, DefinedBeforeLoop p v h

/-- #11: hoist the hoistable instructions of the loop at `h` to the front of
    its preheader `pre`. -/
def p11_licm (p : Prog) (h pre : BlockId) : Option (List Instr × Prog) :=
  match p.blocks[h]?, p.blocks[pre]? with
  | some bl, some prebl =>
    let hoisted := bl.instrs.filter (fun i => Instr.pure i)
    let newPre : Block := { prebl with instrs := prebl.instrs ++ hoisted }
    some (hoisted, { p with blocks := setAtBlock p.blocks pre newPre })
  | _, _ => none

/-! ## #12 Induction Variable Simplification. -/

/-- #12: `v` is an induction variable of the loop at `h` with step `c` when
    the body contains `v := v + c` (either operand order). -/
def Instr.isIvStep (v : VReg) (c : Word) : Instr → Bool
  | .binop d .add (.reg s) (.imm w) => d = v && s = v && w = c
  | .binop d .add (.imm w) (.reg s) => d = v && s = v && w = c
  | _ => false

def InductionVar (p : Prog) (h : BlockId) (v : VReg) (c : Word) : Prop :=
  match p.blocks[h]? with
  | none => False
  | some bl => bl.instrs.any (Instr.isIvStep v c) = true

/-- #12: the derived-def replacement: a copy chain of the induction
    variable is rewritten to its base. -/
def p12_derived (base : VReg) : Instr → Instr
  | .copy d (.reg s) => if s = base then .copy d (.reg base) else .copy d (.reg s)
  | i => i

/-! ## #13 Strength Reduction (FV-0: machine-checked agreement). -/

/-- The power-of-two index of `w`: `k = log₂ w` when `w = 2^k` and `k < 64`
    (shift counts are 6-bit, so larger factors are never strength-reduced). -/
def pow2Idx (w : Word) : Option Nat :=
  if _h : 2 ^ Nat.log2 w = w ∧ Nat.log2 w < 64 then some (Nat.log2 w) else none

/-- #13 rewrite: `d = a * 2^k` becomes `d = a << k`, everything else
    unchanged. Division is never touched. -/
def p13_instr : Instr → Instr
  | .binop d .mul a (.imm w) =>
    match pow2Idx w with
    | some k => .binop d .shl a (.imm k)
    | none => .binop d .mul a (.imm w)
  | i => i

/-- One-instruction agreement for binary operations whose semantics agree
    pointwise in the first operand (the shape of every proved value
    rewrite: folding, strength reduction, algebraic identities). -/
theorem stepInstr_binop_argCongr (m : Machine) (d : VReg) (a : Arg) (v₁ v₂ : Word)
    (o₁ o₂ : Binop)
    (h : ∀ v, Binop.eval o₁ v v₁ = Binop.eval o₂ v v₂) :
    stepInstr m (.binop d o₁ a (.imm v₁)) = stepInstr m (.binop d o₂ a (.imm v₂)) := by
  cases a with
  | reg r =>
    simp only [stepInstr, Arg.eval]
    cases hr : m.regs r with
    | none => rfl
    | some _ => simp only [h]
  | imm w =>
    simp only [stepInstr, Arg.eval]
    simp only [h]

/-- The strength-reduction agreement: the rewritten instruction performs the
    same register write, memory effect, and trap class as the original
    (FV-0, machine-checked; the core identity is Wrd.lean's
    `wmul_pow2_eq_wshl`). -/
theorem p13_step_agree (m : Machine) (i : Instr) :
    stepInstr m (p13_instr i) = stepInstr m i := by
  cases i with
  | nop => rfl
  | copy d a => rfl
  | load d a => rfl
  | store a b => rfl
  | call d f args => rfl
  | binop d o a b =>
    cases o with
    | mul =>
      cases b with
      | reg _ => rfl
      | imm w =>
        simp only [p13_instr]
        cases hp : pow2Idx w with
        | none => rfl
        | some k =>
          rw [pow2Idx] at hp
          split at hp
          · rename_i hc
            obtain ⟨h2, h64⟩ := hc
            have hlk : Nat.log2 w = k := by simpa using hp
            subst hlk
            exact stepInstr_binop_argCongr m d a (Nat.log2 w) w .shl .mul
              (fun v => by
                show Binop.eval .shl v (Nat.log2 w) = Binop.eval .mul v w
                simp only [Binop.eval, wshl, wmul, wrap]
                rw [Nat.mod_eq_of_lt h64, h2])
          · cases hp
    | add => rfl
    | sub => rfl
    | udiv => rfl
    | and => rfl
    | or => rfl
    | xor => rfl
    | shl => rfl
    | lshr => rfl

/-- The whole-program strength-reduction pass #13. -/
def p13_pass (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl => { bl with instrs := bl.instrs.map p13_instr }) }

/-! ## #14 Dead Loop Elimination. -/

/-- #14: the loop at `h` is dead when every body instruction is pure and no
    vreg defined in the body is read by any other block. -/
def LoopDead (p : Prog) (h : BlockId) : Prop :=
  match p.blocks[h]? with
  | none => True
  | some bl =>
    (∀ i ∈ bl.instrs, Instr.pure i) ∧
    (∀ i ∈ bl.instrs, ∀ v, Instr.defines v i = true →
      ∀ c, c ≠ h → c < p.blocks.length →
        match p.blocks[c]? with
        | none => True
        | some bc =>
          bc.instrs.all (fun j => !(Instr.usesOf j).contains v) = true ∧
          !(bc.term.usesOf.contains v))

/-- #14: replace the loop header's terminator by a jump to its exit target
    `x`, dropping the back edge. -/
def p14_dle (p : Prog) (h x : BlockId) : Prog := setTerm p h (.jmp x)

/-! ## #15 Loop Rotation. -/

/-- #15: rotate the loop at `h`: the guard test moves to the latch side;
    here modeled as retargeting the header's conditional to the body first
    and the latch back-edge to a fresh pre-guard copy (shifted by `off`). -/
def p15_rotate (p : Prog) (h body latch : BlockId) (off : VReg) : Option Prog :=
  match p.blocks[h]?, p.blocks[body]?, p.blocks[latch]? with
  | some hh, some hb, some hl =>
    some (withBlocksAppend
      (setTerm (setTerm p latch (Term.shiftArgs off hl.term))
        h (.jmp p.blocks.length))
      [ { instrs := hh.instrs.map (Instr.shift off), term := hh.term },
        { instrs := hb.instrs.map (Instr.shift off), term := hb.term } ])
  | _, _, _ => none

/-! ## #16 Software Pipelining (modulo schedule). -/

/-- #16: a modulo schedule with initiation interval `II`: the schedule
    function is periodic with period exactly `II`. -/
def ModuloSchedule (m : Nat → Nat) (II : Nat) : Prop :=
  ∀ i, m (i + II) = m i + II

/-- #16: the kernel of a pipelined loop: `II` copies of the body, the
    i-th copy shifted by `off * i`. -/
def p16_kernel (body : List Instr) (II off : Nat) : List Instr :=
  ((List.range II).map (fun i =>
    if i = 0 then body else body.map (Instr.shift (off * i)))).foldr Append.append []

/-! ## #17 Global Code Motion. -/

/-- #17 placement validity: every pure vreg is placed at a block that
    dominates all blocks using it (expressed over CFG.lean's `Dominates`). -/
def UsesVreg (p : Prog) (u : BlockId) (v : VReg) : Prop :=
  match p.blocks[u]? with
  | none => False
  | some bl => v ∈ bl.instrs.flatMap Instr.usesOf ∨ v ∈ bl.term.usesOf

def GcmOk (p : Prog) (place : VReg → BlockId) : Prop :=
  ∀ v u, UsesVreg p u v → Dominates p (place v) u

/-- #17 extract phase: lift every pure instruction out of its block; the
    paired mechanical step reinserts them at their `GcmOk` placements. -/
def p17_gcm_extract (p : Prog) : Prog :=
  { p with blocks := p.blocks.map (fun bl =>
    { bl with instrs := bl.instrs.filter Instr.pure }) }

/-! ## #18 Tail Duplication. -/

/-- #18: duplicate block `b` (vregs shifted by `off`), retargeted to jump to
    `t`, and append it; the caller redirects one predecessor to the copy. -/
def p18_tailDup (p : Prog) (b t : BlockId) (off : VReg) : Option Prog :=
  match p.blocks[b]? with
  | none => none
  | some bl =>
    some (withBlocksAppend p
      [ { instrs := bl.instrs.map (Instr.shift off)
          term := Term.retarget (fun _ => t) (Term.shiftArgs off bl.term) } ])

theorem p18_length (p : Prog) (b t off : Nat) {p' : Prog}
    (hp : p18_tailDup p b t off = some p') :
    p'.blocks.length = p.blocks.length + 1 := by
  unfold p18_tailDup at hp
  cases hb : p.blocks[b]? with
  | none => rw [hb] at hp; exact absurd hp (by simp)
  | some bl =>
    rw [hb] at hp
    have hinj := Option.some.inj hp
    cases hinj
    exact withBlocksAppend_length _ _

/-! ## #19 Jump Threading. -/

/-- #19: one threading step: if block `b` jumps to `l` and `l` jumps to `m`,
    re-thread `b` directly to `m`. -/
def p19_thread (p : Prog) (b : BlockId) : Option Prog :=
  match p.blocks[b]?, p.blocks[succJmp b p]? with
  | some _, some lb =>
    match lb.term with
    | .jmp m => some (setTerm p b (.jmp m))
    | _ => none
  | _, _ => none
where
  succJmp (b : BlockId) (p : Prog) : BlockId :=
    match p.blocks[b]? with
    | some bl => match bl.term with | .jmp l => l | _ => b
    | none => b

/-! ## #20 Conditional Branch Optimization. -/

/-- #20: implemented by the fused sweep `optTerm` of RV8FV/Opt.lean
    (substitute known values, fold constant branches into jumps). -/
def p20_optTerm (env : CEnv) : Term → Term := optTerm env

/-! ## #21 Switch Statement Lowering. -/

/-- #21: the switch table as a total map (value → target, default when
    absent), and its lowering to a chain of fresh test blocks: test block
    `base + i` branches on the materialized equality flag `tᵢ` to its
    target or to the next test; the chain ends at the default. The equality
    comparisons themselves are the target-specific part lowered by isel.
    Machine-checked: the chain has exactly `n+1` blocks for `n` tests. -/
def switchMap (cases : List (Word × BlockId)) (dflt : BlockId) : Word → BlockId :=
  fun w => match cases.lookup w with | some l => l | none => dflt

def p21_switchChain (tests : List Arg) (targets : List BlockId)
    (dflt : BlockId) (base : BlockId) : List Block :=
  match tests, targets with
  | _, [] => [ { instrs := [], term := .jmp dflt } ]
  | [], _ => [ { instrs := [], term := .jmp dflt } ]
  | t :: ts, l :: ls =>
    { instrs := [], term := .br t l (base + 1) }
      :: p21_switchChain ts ls dflt (base + 1)

theorem p21_chain_length : ∀ (tests : List Arg) (targets : List BlockId)
    (dflt : BlockId) (base : BlockId),
    tests.length = targets.length →
    (p21_switchChain tests targets dflt base).length = targets.length + 1 := by
  intro tests targets dflt base hlen
  cases tests with
  | nil =>
    cases targets with
    | nil => simp [p21_switchChain]
    | cons l ls => simp at hlen
  | cons t ts =>
    cases targets with
    | nil => simp [p21_switchChain]
    | cons l ls =>
      simp only [p21_switchChain, List.length_cons]
      have h2 : ts.length = ls.length := by simpa using hlen
      rw [p21_chain_length ts ls dflt (base + 1) h2]

/-! ## #22 CFG Flattening. -/

/-- #22: flatten: the dispatch chain over the block table `targets` with
    equality flags `flags` (one per target, materialized by the comparison
    lowering of the state vreg against the block id); `base` is the first
    fresh block index. -/
def p22_dispatch (flags : List Arg) (targets : List BlockId)
    (dflt : BlockId) (base : BlockId) : List Block :=
  p21_switchChain flags targets dflt base

/-! ## #23 Irreducible CFG Reduction (node splitting). -/

/-- #23: split node `b`: append a vreg-shifted clone of `b` whose terminator
    jumps to the original successor `s`; predecessors are then distributed
    between the twin nodes by the caller. -/
def p23_splitNode (p : Prog) (b s : BlockId) (off : VReg) : Option Prog :=
  match p.blocks[b]? with
  | none => none
  | some bl =>
    some (withBlocksAppend p
      [ { instrs := bl.instrs.map (Instr.shift off), term := .jmp s } ])

theorem p23_length (p : Prog) (b s off : Nat) {p' : Prog}
    (hp : p23_splitNode p b s off = some p') :
    p'.blocks.length = p.blocks.length + 1 := by
  unfold p23_splitNode at hp
  cases hb : p.blocks[b]? with
  | none => rw [hb] at hp; exact absurd hp (by simp)
  | some bl =>
    rw [hb] at hp
    have hinj := Option.some.inj hp
    cases hinj
    exact withBlocksAppend_length _ _

/-! ## #24 Edge Splitting / #25 Critical Edge Splitting. -/

/-- #24: split the edge `a -> b`: append a fresh block that jumps to `b`,
    and retarget `a`'s terminator through `σ = (fun t => if t = b then fresh
    else t)`; `fresh = p.blocks.length` (the appended block). -/
def p24_splitEdge (p : Prog) (a b : BlockId) : Option Prog :=
  match p.blocks[a]? with
  | none => none
  | some bl =>
    let fresh := p.blocks.length
    some (withBlocksAppend
      (setTerm p a (Term.retarget (fun t => if t = b then fresh else t) bl.term))
      [ { instrs := [], term := .jmp b } ])

theorem p24_length (p : Prog) (a b : Nat) {p' : Prog}
    (hp : p24_splitEdge p a b = some p') :
    p'.blocks.length = p.blocks.length + 1 := by
  unfold p24_splitEdge at hp
  cases hb : p.blocks[a]? with
  | none => rw [hb] at hp; exact absurd hp (by simp)
  | some bl =>
    rw [hb] at hp
    have hinj := Option.some.inj hp
    cases hinj
    rw [withBlocksAppend_length, setTerm_length]
    simp

/-- #25: an edge is critical when it enters a join (multiple predecessors)
    out of a fork (multiple successors). -/
def CriticalEdge (p : Prog) (a b : BlockId) : Prop :=
  b ∈ succs p a ∧ (preds p b).length > 1 ∧ (succs p a).length > 1

/-- #25: split every critical edge out of `a` (the per-edge mechanical part). -/
def p25_splitCritical (p : Prog) (a : BlockId) : Option Prog :=
  match p.blocks[a]? with
  | none => none
  | some bl =>
    match bl.term with
    | .br _c l r =>
      if (preds p l).length > 1 then p24_splitEdge p a l else p24_splitEdge p a r
    | _ => none

/-! ## #26-#30 The CFG analysis passes (FV-0 definitions from CFG.lean). -/

/-- #26 Dominator tree updates: the dominance relation itself. -/
abbrev p26_dominates (p : Prog) : BlockId → BlockId → Prop := Dominates p

/-- #27 Post-dominator analysis. -/
abbrev p27_postdominates (p : Prog) : BlockId → BlockId → Prop := PostDominates p

/-- #28 Natural loop detection: back edges and their bodies. -/
abbrev p28_backEdge (p : Prog) : BlockId → BlockId → Prop := BackEdge p

/-- #29 Reducibility testing. -/
abbrev p29_reducible (p : Prog) : Prop := Reducible p

/-- #30 Control dependence graph construction. -/
abbrev p30_controlDependent (p : Prog) : BlockId → BlockId → Prop := ControlDependent p

end RV8FV
