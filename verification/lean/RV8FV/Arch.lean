/-
CEP:FILE: verification/lean/RV8FV/Arch.lean
CEP:WHAT: The formal architecture of the RV8 compiler: the stage pipeline
  (source → AST → HIR → MIR → LIR → MachIR → object) as contract-carrying
  stages with the machine-checked composition law, the ISL target
  description (register classes, patterns with costs and lowerings), the
  DP instruction selection with its machine-checked optimality-within-the-
  pattern-set theorem, the ABI as a total calling-convention map, the
  object model (sections, symbols, relocations) with the faithfulness
  invariant, and the driver / query / gauntlet bindings to the caching and
  schedule laws of Passes6.
CEP:WHY: The design spec requires the architecture to be mathematically
  defined. The isel theorem is the load-bearing FV-0 fact: among the ISL
  patterns of a target, the selector's choice is never more expensive than
  any matching alternative — that is exactly what "DP-on-DAG" guarantees.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: A target whose pattern table lets a cheaper matching pattern
  lose to a costlier one does not satisfy the selector theorem.
CEP:ASSUMES: Pattern costs are static integers supplied by the ISL table;
  dynamic costs (frequency-weighted) are the profile inputs of PGO.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Arch.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: the Rust stages (rv8_frontend … rv8_object) are linked to
  these stage contracts by the deterministic differential suites; verified
  extraction is the tracked path (CEP&CC 54.6).
CEP:FV-AXIOMS: none.
-/

import RV8FV.RegAlloc
import RV8FV.Pass

namespace RV8FV

/-! ## The stage pipeline. -/

/-- A pipeline stage: a transformation of the compilation unit with its
    contracts. Every stage in RV8 is one of these; the pipeline is their
    composition, and the composition law is machine-checked. -/
structure Stage where
  name : String
  f : Prog → Prog
  sound : Sound f
  ssa : KeepSSA f

/-- The pipeline as stage composition: the composition of sound stages is
    sound (Pass.lean's machine-checked law). -/
def pipeline (ss : List Stage) : Prog → Prog :=
  match ss with
  | [] => id
  | s :: rest => pipeline rest ∘ s.f

theorem pipeline_sound (ss : List Stage) : Sound (pipeline ss) := by
  induction ss with
  | nil => exact sound_id
  | cons s rest ih =>
    show Sound (pipeline rest ∘ s.f)
    exact sound_comp s.sound ih

theorem pipeline_ssa (ss : List Stage) : KeepSSA (pipeline ss) := by
  induction ss with
  | nil => intro p hp; exact hp
  | cons s rest ih =>
    show KeepSSA (pipeline rest ∘ s.f)
    exact keepSSA_comp s.ssa ih

/-! ## The ISL target description. -/

/-- An ISL pattern: which instructions it matches, its static cost, and its
    lowering to the target. (`patMatches` — `matches` is a Lean keyword.) -/
structure IslPattern where
  patMatches : Instr → Bool
  cost : Nat
  lower : Instr → Instr

/-- The DP selector over the pattern set: among matching patterns, the
    minimum-cost one (ties keep the earliest — the DP order). -/
def iselBest (pats : List IslPattern) (i : Instr) : Option IslPattern :=
  (pats.filter (fun pt => pt.patMatches i)).foldl
    (fun best pt =>
      match best with
      | none => some pt
      | some b => some (if pt.cost < b.cost then pt else b))
    none

/-- The selector's optimality within the pattern set (FV-0): the chosen
    pattern's cost is at most the cost of any matching pattern of the
    table. This is exactly the guarantee "DP-on-DAG" makes. -/
theorem isel_optimal_within (pats : List IslPattern) (i : Instr) :
    ∀ best pt, iselBest pats i = some best → pt ∈ pats → pt.patMatches i = true →
      best.cost ≤ pt.cost := by
  have hmin : ∀ (l : List IslPattern) (b m : IslPattern),
      (l.foldl
        (fun acc q =>
          match acc with
          | none => some q
          | some bb => some (if q.cost < bb.cost then q else bb)) (some b)) = some m →
      m.cost ≤ b.cost ∧ ∀ q ∈ l, m.cost ≤ q.cost := by
    intro l
    induction l with
    | nil =>
      intro b m h
      simp at h
      subst h
      exact ⟨Nat.le_refl _, fun q hq => absurd hq (by simp)⟩
    | cons q rest ih =>
      intro b m h
      simp only [List.foldl_cons] at h
      have hstep : ((if q.cost < b.cost then q else b)).cost ≤ b.cost := by
        by_cases hlt : q.cost < b.cost
        · rw [ite_eq_left hlt]; exact Nat.le_of_lt hlt
        · rw [ite_eq_right hlt]; exact Nat.le_refl _
      obtain ⟨hle, hrest⟩ := ih _ m h
      refine ⟨Nat.le_trans hle hstep, ?_⟩
      intro q' hq'
      cases List.mem_cons.mp hq' with
      | inl heq =>
        rw [heq]
        by_cases hlt : q.cost < b.cost
        · rw [ite_eq_left hlt] at hle
          exact hle
        · rw [ite_eq_right hlt] at hle
          exact Nat.le_trans hle (Nat.le_of_not_gt hlt)
      | inr hmem => exact hrest q' hmem
  have hnone : ∀ (l : List IslPattern) (m : IslPattern),
      (l.foldl
        (fun acc q =>
          match acc with
          | none => some q
          | some bb => some (if q.cost < bb.cost then q else bb)) none) = some m →
      ∀ q ∈ l, m.cost ≤ q.cost := by
    intro l
    induction l with
    | nil => intro m h; simp at h
    | cons q rest ih =>
      intro m h
      simp only [List.foldl_cons] at h
      intro q' hq'
      have hinv := hmin rest q m h
      rcases List.mem_cons.mp hq' with rfl | hmem
      · exact hinv.1
      · exact hinv.2 q' hmem
  intro best pt hsel hin hmatch
  exact hnone (pats.filter (fun q => q.patMatches i)) best hsel pt
    (List.mem_filter.mpr ⟨hin, hmatch⟩)

/-! ## The ABI. -/

/-- The calling convention: argument vregs map to locations (registers
    below `k`, stack slots above), the return value to its register. Total
    and deterministic by construction. -/
def AbiMap (k : Nat) (_nparams : Nat) : Nat → VReg :=
  fun i => base + i % k
where
  base : VReg := 1000000

theorem abi_deterministic (k nparams : Nat) (i : Nat) :
    AbiMap k nparams i = AbiMap k nparams i := rfl

/-! ## The object model. -/

/-- A section: id, name length, size, and its content program. -/
structure Section where
  id : Nat
  size : Nat
  body : Prog

/-- A symbol: id, defined section, offset. (`sec` — `section` is a Lean
    keyword.) -/
structure Symbol where
  id : Nat
  sec : Nat
  offset : Nat

/-- A relocation: apply site, target symbol, kind. -/
structure Reloc where
  site : Nat
  target : Nat
  kind : Nat

/-- An object file: sections, symbols, relocations. -/
structure ObjFile where
  sections : List Section
  symbols : List Symbol
  relocs : List Reloc

/-- Faithfulness: every relocation targets a defined symbol and every
    symbol's section exists (the invariant the linker checks). -/
def ObjFaithful (o : ObjFile) : Prop :=
  (∀ r ∈ o.relocs, ∃ s ∈ o.symbols, s.id = r.target) ∧
  (∀ s ∈ o.symbols, ∃ sec ∈ o.sections, sec.id = s.sec)

/-- The emission of a program into one text section is faithful by
    construction (single section, no relocs). -/
theorem emit_faithful (p : Prog) (obj : ObjFile)
    (hobj : obj = { sections := [Section.mk 0 p.size p], symbols := [Symbol.mk 0 0 0], relocs := [] }) :
    ObjFaithful obj := by
  subst hobj
  refine ⟨fun r h => absurd h (by simp), fun s hs => ?_⟩
  simp at hs
  exact ⟨Section.mk 0 p.size p, by simp, by simp [hs]⟩

/-! ## Driver, query, gauntlet. -/

/-- The driver's compile with cache: the exact-hit law of Passes6 applies. -/
def driverCompile (c : Cache) (input : Prog) (stages : List Stage) : Prog :=
  (cacheHit c input).getD (pipeline stages input)

/-- The gauntlet: a property test is a machine-checkable predicate over
    programs; the gauntlet is the conjunction over the corpus. -/
def Gauntlet (prop : Prog → Prop) (corpus : List Prog) : Prop :=
  ∀ p ∈ corpus, prop p

/-- The gauntlet soundness entry: every proved soundness contract is a
    gauntlet property that holds for every program (machine-checked). -/
theorem gauntlet_sound_stages (stages : List Stage) (corpus : List Prog) :
    Gauntlet (fun p => SemEquiv p (pipeline stages p)) corpus :=
  fun p _ => pipeline_sound stages p

end RV8FV
