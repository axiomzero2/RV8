/-
CEP:FILE: verification/lean/RV8FV/CFG.lean
CEP:WHAT: The control-flow mathematics of the RV8 LIR: successors, paths as
  lists, reachability, dominators and post-dominators defined over complete
  paths (every path from the entry must contain the dominator), back edges,
  natural loop bodies, the reducibility law (every cycle-closing edge
  reachable from the entry has its target dominating its source), and
  Ferrante-style control dependence, plus terminator replacement with its
  pointwise block lemmas.
CEP:WHY: Category-1 passes (catalog #22-#30) and the loop passes (#2-#17) are
  specified over exactly these objects; defining them over complete paths
  makes "dominates" a total mathematical predicate with no hidden assumptions
  about reducibility or DFS orders.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A pass that moves code across a dominance boundary that the
  predicates below deny is outside the model and must be rejected.
CEP:ASSUMES: Block identity is the index into the block list; the entry block
  is 0 (as in Spec.lean). Empty paths have no last element, so no block
  dominates through them; self-reachability uses the reflexive clause.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/CFG.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: the Rust CFG analyses (rv8_middle::analysis) are bridged by
  the dominance fixtures of the differential suites.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Wrd

namespace RV8FV

/-! ## Successors and exits. -/

/-- The successor blocks of block `b`: the terminator's targets. -/
def succs (p : Prog) (b : BlockId) : List BlockId :=
  match p.blocks[b]? with
  | none => []
  | some bl =>
    match bl.term with
    | .jmp l => [l]
    | .br _ l r => [l, r]
    | .ret _ => []

/-- Whether block `b` is an exit (its terminator is a return). -/
def isExit (p : Prog) (b : BlockId) : Bool :=
  match p.blocks[b]? with
  | some bl => match bl.term with | .ret _ => true | _ => false
  | none => false

/-! ## Paths. -/

/-- `PathTo p x l`: the list `l` is a valid control-flow path that starts at
    block `x` by following successors. The empty path stays at `x`. -/
def PathTo (p : Prog) (x : BlockId) : List BlockId → Prop
  | [] => True
  | y :: rest => y ∈ succs p x ∧ PathTo p y rest

/-- The path without its final element (used for "avoids" reasoning). -/
def pathInit : List BlockId → List BlockId
  | [] => []
  | [_] => []
  | x :: rest => x :: pathInit rest

/-- Reachability: `x` reaches `z`, and the edge `z -> w` exists, so `x`
    reaches `w`. Reflexivity gives the zero-length path. -/
inductive Reach (p : Prog) : BlockId → BlockId → Prop
  | refl (b : BlockId) : Reach p b b
  | step (x z w : BlockId) : Reach p x z → w ∈ succs p z → Reach p x w

theorem reach_edge (p : Prog) (x y : BlockId) (h : y ∈ succs p x) : Reach p x y :=
  .step _ _ _ (.refl x) h

/-- Transitivity of reachability. -/
theorem Reach_trans (p : Prog) (a b : BlockId) :
    ∀ (c : BlockId), Reach p a b → Reach p b c → Reach p a c
  | _, h1, .refl _ => h1
  | _, h1, .step _ _ _ hz hw => .step _ _ _ (Reach_trans p a b _ h1 hz) hw

/-- `a` reaches `b` either trivially (a = b) or along a nonempty path whose
    last element is `b`. -/
def ReachesBy (p : Prog) (a b : BlockId) : Prop :=
  a = b ∨ ∃ l, PathTo p a l ∧ l.getLast? = some b

theorem reachesBy_edge (p : Prog) (x y : BlockId) (h : y ∈ succs p x) :
    ReachesBy p x y :=
  Or.inr ⟨[y], ⟨h, True.intro⟩, rfl⟩

theorem PathTo_append (p : Prog) : ∀ (x : BlockId) (l₁ : List BlockId),
    PathTo p x l₁ → ∀ (y : BlockId), l₁.getLast? = some y → ∀ (l₂ : List BlockId),
    PathTo p y l₂ → PathTo p x (l₁ ++ l₂)
  | _, [], _, _, hlast, _, _ => by simp at hlast
  | _, u :: rest, hpath, y, hlast, l₂, hl₂ => by
      simp only [List.cons_append]
      cases rest with
      | nil =>
        have huy : u = y := by simpa [List.getLast?] using hlast
        exact ⟨hpath.1, by rw [huy]; exact hl₂⟩
      | cons v tl =>
        have htl : (v :: tl).getLast? = some y := by simpa [List.getLast?] using hlast
        exact ⟨hpath.1, PathTo_append p u (v :: tl) hpath.2 y htl l₂ hl₂⟩

theorem getLast?_append_single (p : Prog) (x : BlockId) :
    ∀ (l : List BlockId) (w : BlockId), PathTo p x l → (l ++ [w]).getLast? = some w
  | [], _, _ => rfl
  | u :: rest, w, ⟨_, hrest⟩ => by
      cases rest with
      | nil => rfl
      | cons v tl =>
        show ((v :: tl) ++ [w]).getLast? = some w
        exact getLast?_append_single p u (v :: tl) w hrest

theorem reachesBy_last_mem (p : Prog) (b : BlockId) :
    ∀ (a : BlockId) (l : List BlockId), PathTo p a l → l.getLast? = some b → b ∈ l := by
  intro a l
  induction l generalizing a with
  | nil => intro _ h; simp at h
  | cons y rest ih =>
    intro hpath hlast
    obtain ⟨_, hrest⟩ := hpath
    cases rest with
    | nil => simp at hlast; simp [hlast]
    | cons z rest' => exact List.mem_cons_of_mem _ (ih y hrest hlast)

/-- Reachability agrees with path existence. -/
theorem reach_path (p : Prog) (a : BlockId) :
    ∀ (b : BlockId), Reach p a b → ReachesBy p a b
  | _, .refl _ => Or.inl rfl
  | _, .step _ z w hz hw => by
      rcases reach_path p a _ hz with heq | ⟨l, hl1, hl2⟩
      · subst heq
        exact Or.inr ⟨[w], ⟨hw, True.intro⟩, rfl⟩
      · have hp : PathTo p a (l ++ [w]) :=
          PathTo_append p a l hl1 z hl2 [w] ⟨hw, True.intro⟩
        exact Or.inr ⟨l ++ [w], hp, getLast?_append_single p a l w hl1⟩

/-! ## Dominance. -/

/-- `d` dominates `b` iff every control-flow path from the entry (block 0) to
    `b` contains `d`. This is the complete-path definition: no DFS order and
    no reducibility assumption is hidden inside. -/
def Dominates (p : Prog) (d b : BlockId) : Prop :=
  ∀ l, PathTo p 0 l → l.getLast? = some b → d ∈ l

/-- Dominance is reflexive. -/
theorem dominates_refl (p : Prog) (b : BlockId) : Dominates p b b :=
  fun l hl hlast => reachesBy_last_mem p b 0 l hl hlast

/-! ## Post-dominance. -/

/-- `y` post-dominates `x` iff every path from `x` to any exit contains `y`. -/
def PostDominates (p : Prog) (y x : BlockId) : Prop :=
  ∀ l e, PathTo p x l → l.getLast? = some e → isExit p e = true → y ∈ l

/-! ## Back edges and natural loops. -/

/-- A back edge: `h -> l` where `l` dominates `h` (l is the loop header). -/
def BackEdge (p : Prog) (h l : BlockId) : Prop :=
  l ∈ succs p h ∧ Dominates p l h

/-- `m` can reach `to` on some path that does not pass through `avoid` except
    possibly as the final element. -/
def ReachesAvoiding (p : Prog) (m to avoid : BlockId) : Prop :=
  ∃ l, PathTo p m l ∧ l.getLast? = some to ∧ avoid ∉ pathInit l

/-- Membership in the natural loop body of the back edge `h -> l`: the header
    itself, or any block that reaches the latch without passing through the
    header. -/
def InLoopBody (p : Prog) (h l m : BlockId) : Prop :=
  m = h ∨ (ReachesAvoiding p m l h ∧ m ≠ h)

theorem loopBody_header (p : Prog) (h l : BlockId) : InLoopBody p h l h := Or.inl rfl

/-! ## Reducibility. -/

/-- Reducibility law: every edge `h -> l` that closes a cycle reachable from
    the entry (i.e. `l` can reach `h`) must have `l` dominate `h`. This is
    the DFS-free form of "every retreating edge is a back edge". -/
def Reducible (p : Prog) : Prop :=
  ∀ h l, Reach p 0 h → l ∈ succs p h → Reach p l h → Dominates p l h

/-! ## Control dependence (Ferrante-Ottenstein-Warren form). -/

/-- `x` is control-dependent on `y` iff `y` branches, one successor of `y`
    post-dominates `x`, and `y` itself does not post-dominate `x`. -/
def ControlDependent (p : Prog) (x y : BlockId) : Prop :=
  ∃ y', y' ∈ succs p y ∧ PostDominates p y' x ∧ ¬ PostDominates p y x

/-! ## Terminator replacement with pointwise lemmas. -/

/-- Replace the terminator of block `b` in a block list. -/
def setTermBlocks : List Block → BlockId → Term → List Block
  | [], _, _ => []
  | bl :: rest, 0, t => { bl with term := t } :: rest
  | bl :: rest, b + 1, t => bl :: setTermBlocks rest b t

def setTerm (p : Prog) (b : BlockId) (t : Term) : Prog :=
  { p with blocks := setTermBlocks p.blocks b t }

theorem setTermBlocks_length : ∀ (bls : List Block) (b : BlockId) (t : Term),
    (setTermBlocks bls b t).length = bls.length := by
  intro bls b t
  induction bls generalizing b with
  | nil => cases b <;> rfl
  | cons bl rest ih =>
    cases b with
    | zero => rfl
    | succ n => simp only [setTermBlocks, List.length_cons]; rw [ih n]

theorem setTerm_length (p : Prog) (b : BlockId) (t : Term) :
    (setTerm p b t).blocks.length = p.blocks.length :=
  setTermBlocks_length p.blocks b t

theorem setTermBlocks_at : ∀ (bls : List Block) (b c : BlockId) (t : Term),
    (setTermBlocks bls b t)[c]?
      = if c = b then (bls[c]?).map (fun bl => { bl with term := t }) else bls[c]? := by
  intro bls b c t
  induction bls generalizing b c with
  | nil => cases b <;> cases c <;> simp [setTermBlocks]
  | cons bl rest ih =>
    cases b with
    | zero =>
      cases c with
      | zero => simp [setTermBlocks]
      | succ n => simp [setTermBlocks]
    | succ m =>
      cases c with
      | zero => simp [setTermBlocks]
      | succ n =>
        simp only [setTermBlocks, List.getElem?_cons_succ, List.getElem?_cons_succ,
          Nat.succ_inj]
        simpa using ih m n

theorem setTerm_other (p : Prog) (b c : BlockId) (t : Term) (h : c ≠ b) :
    (setTerm p b t).blocks[c]? = p.blocks[c]? := by
  show (setTermBlocks p.blocks b t)[c]? = p.blocks[c]?
  rw [setTermBlocks_at]
  rw [if_neg h]

theorem setTerm_self (p : Prog) (b : BlockId) (t : Term) :
    (setTerm p b t).blocks[b]?
      = (p.blocks[b]?).map (fun bl => { bl with term := t }) := by
  show (setTermBlocks p.blocks b t)[b]? = _
  rw [setTermBlocks_at]
  rw [if_pos rfl]

end RV8FV
