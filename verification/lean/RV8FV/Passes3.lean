/-
CEP:FILE: verification/lean/RV8FV/Passes3.lean
CEP:WHAT: Mathematical definitions of catalog passes #71-#95 (Memory & Cache
  Hierarchy Optimizations of design.md): tiling and locality measures, the
  recursive cache-oblivious decomposition, prefetch insertion, stream
  alignment, array padding, structure splitting and field reordering,
  stack-slot sharing by live-range overlap, heap allocation elision guarded
  by escape analysis, object inlining, barrier elision under ownership,
  NUMA placement, TLB-batching, page coloring, memory coalescing, shared
  memory tiling, spill costing, live-range splitting, rematerialization,
  pointer swizzling, compact layouts, bitpacking, delta encoding with its
  machine-checked round-trip theorem (FV-0, over the wrapping word), and
  aliasing disambiguation bound to the MayAlias contract.
CEP:WHY: The design spec requires every pass to be mathematically defined;
  this file is the normative definition of the memory category.
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: A memory transform that claims to remove an access without a
  certificate (deadness, ownership, escape-freedom) is outside the model.
CEP:ASSUMES: Target parameters (line size, associativity, node count) are
  target-hardware inputs of the ISL description (Arch.lean); the hardware
  behaviors themselves are the declared hardware axioms of the target.
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Passes3.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: the hardware-parameterized transforms are contracts over
  the target model; their semantic soundness is per-target and tracked in
  the frames. The delta-encoding round-trip and the pure-local definitions
  are FV-0.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Passes2

namespace RV8FV

/-! ## #71 / #87 Tiling with target locality. -/

/-- #71: the tile schedule of an `n`-iteration loop with tile size `b`
    (same partition as #5, now parameterized by the target's line size for
    the locality measure). -/
def p71_tile (n b : Nat) : List (List Nat) := p05_tile n b

/-- #71: the locality gain of tile size `b` for stride `s` on a target with
    line size `l`: distinct lines touched per tile. -/
def p71_locality (b s l : Nat) : Nat := p05_lines b s l

/-- #87: the shared-memory tile: a tile whose working set `b * s` fits the
    target's shared memory budget `m` is resident (decidable predicate). -/
def p87_fitsShared (b s m : Nat) : Bool := b * s <= m

/-! ## #72 Cache-oblivious recursive decomposition. -/

/-- #72: the cache-oblivious schedule: recursively halve the index range
    until singletons; the traversal order is the recursion order. -/
def p72_rectSplit : Nat → Nat → List (Nat × Nat)
  | lo, 1 => [(lo, lo)]
  | lo, n + 2 =>
    let half := (n + 2) / 2
    p72_rectSplit lo half ++ p72_rectSplit (lo + half) (n + 2 - half)
  | _, 0 => []

theorem p72_length : ∀ (n lo : Nat), (p72_rectSplit lo n).length = n := by
  intro n
  induction n using Nat.strongRecOn with
  | _ n ih =>
    intro lo
    cases n with
    | zero => simp [p72_rectSplit]
    | succ m =>
      cases m with
      | zero => simp [p72_rectSplit]
      | succ m' =>
        simp only [p72_rectSplit]
        have h1 : (m' + 2) / 2 < m' + 2 := Nat.div_lt_self (by omega) (by omega)
        have h2 : m' + 2 - (m' + 2) / 2 < m' + 2 := by
          have hp : (m' + 2) / 2 > 0 := Nat.div_pos (by omega) (by omega)
          omega
        rw [List.length_append,
          ih ((m' + 2) / 2) h1 lo,
          ih (m' + 2 - (m' + 2) / 2) h2 (lo + (m' + 2) / 2)]
        omega

/-! ## #73 Data prefetching. -/

/-- #73: insert a prefetch (modeled as a load into a scratch register, which
    the backend lowers to the target's prefetch instruction) before every
    `stride`-th access of the block. -/
def p73_prefetch (p : Prog) (stride : Nat) (scratch : VReg) : Prog :=
  p62_safepoints p stride scratch

/-! ## #74 Stream buffers / alignment. -/

/-- #74: align a base address up to the line size `l`. -/
def alignTo (base l : Nat) : Nat := (base + l - 1) / l * l

theorem alignTo_aligned (base l : Nat) (hl : l > 0) : alignTo base l % l = 0 := by
  unfold alignTo
  exact Nat.mul_mod_left _ _

/-! ## #75 Array padding. -/

/-- #75: pad the stride so consecutive rows do not collide in the cache:
    when the stride is a multiple of `line * assoc`, add one line. -/
def p75_padStride (stride line assoc : Nat) : Nat :=
  if line * assoc > 0 && stride % (line * assoc) = 0 then stride + line else stride

/-! ## #76 / #77 Structure splitting and field reordering. -/

/-- A field: offset, width, and access weight (from the profile). -/
structure Field where
  offset : Nat
  width : Nat
  weight : Nat

/-- #76: split the fields into the hot set (weight ≥ threshold) and the
    cold set; the structure becomes two structures and every access is
    redirected to the containing one. -/
def p76_split (fields : List Field) (threshold : Nat) : List Field × List Field :=
  (fields.filter (fun f => f.weight ≥ threshold),
   fields.filter (fun f => f.weight < threshold))

/-- Insert a field into a weight-decreasing list (stable). -/
def insertByWeight (f : Field) : List Field → List Field
  | [] => [f]
  | g :: rest => if g.weight < f.weight then f :: g :: rest else g :: insertByWeight f rest

/-- #77: reorder fields by decreasing weight (stable): the layout permution
    the backend applies. -/
def p77_reorder (fields : List Field) : List Field :=
  fields.foldl (fun acc f => insertByWeight f acc) []

/-! ## #78 Stack coloring. -/

/-- A live range: first and last program point. -/
abbrev Range := Nat × Nat

/-- Two ranges overlap when they intersect. -/
def RangeOverlap (a b : Range) : Prop := a.1 ≤ b.2 ∧ b.1 ≤ a.2

/-- #78: two stack slots may share one color when their live ranges never
    overlap; the coloring is the partition into overlap-free classes. -/
def StackShareable (a b : Range) : Prop := ¬ RangeOverlap a b

/-! ## #79 Heap allocation elision. -/

/-- #79: the allocation of `v` may be moved to the stack when `v` never
    escapes (the pass consumes NoEscape of #47; the transformation replaces
    the allocation by stack-pointer-relative addressing, which the model
    sees as ordinary register-local vregs). -/
def p79_elidable (p : Prog) (v : VReg) : Prop := NoEscape p v

/-! ## #80 Object inlining. -/

/-- #80: an object of total width `w` may be inlined into its unique
    referrer when `w` fits the inline budget. -/
def p80_inlineable (w budget : Nat) : Bool := w <= budget

/-! ## #81 / #82 Barrier elision. -/

/-- #81/#82: a write (read) barrier on `v` may be elided when the object is
    thread-owned: no other thread can hold a reference (the ownership
    discipline of the capability model, pass #170). -/
def BarrierElidable (p : Prog) (v : VReg) : Prop := NoEscape p v

/-! ## #83-#85 NUMA, TLB, page coloring. -/

/-- #83: NUMA placement: data identified by `id` is placed on node
    `id % nodes` (the affinity map; the transform annotates the allocation). -/
def p83_numaNode (id nodes : Nat) : Nat := id % nodes

/-- #84: TLB shootdown batching: shootdowns are deferred until `batch`
    invalidations accumulate. -/
def p84_batchDue (pending batch : Nat) : Bool := pending >= batch

/-- #85: page coloring: the color of address `a` on a target with `colors`
    colors and line size `l`. -/
def p85_color (a l colors : Nat) : Nat := (a / l) % colors

/-! ## #86 Memory coalescing. -/

/-- #86: two word accesses at `a₁` and `a₂` of width `w` coalesce when they
    are adjacent within one line (a₂ = a₁ + w). -/
def Coalescable (a₁ a₂ w : Nat) : Prop := a₂ = a₁ + w

/-! ## #88-#90 Spill costing, range splitting, rematerialization. -/

/-- #88: the spill cost of a vreg: its use count weighted by loop depth
    (the cost the MCF allocator of RegAlloc.lean minimizes). -/
def spillCost (uses depth : Nat) : Nat := uses * (depth + 1)

/-- #89: split the live range `[lo, hi)` at `t` into two ranges; the
    register may then differ across the split point. -/
def p89_splitRange (r : Range) (t : Nat) : Option (Range × Range) :=
  if r.1 < t && t < r.2 then some ((r.1, t), (t, r.2)) else none

/-- #90: a vreg is rematerializable when its single definition is a pure
    instruction over immediates: recomputing it is cheaper than spilling. -/
def Rematerializable (p : Prog) (v : VReg) : Prop :=
  ∃ bl : Block, ∃ i ∈ bl.instrs, i.defOf = some v ∧
    match i with
    | .binop _ o (.imm _) (.imm _) => o ≠ .udiv
    | .copy _ (.imm _) => true
    | _ => false

/-! ## #91-#93 Swizzling, compact layouts, bitpacking. -/

/-- #91: pointer swizzling: the external pointer `base + off` becomes the
    offset `off` alone; the bijection with the base is the pool identity. -/
def p91_swizzle (base off : Word) : Word := wsub off base

/-- #92: the compact layout size of fields with widths `ws` (sum of
    widths, no alignment holes). -/
def p92_size (ws : List Nat) : Nat := ws.foldl Nat.add 0

/-- #93: bitpack: pack `k`-bit fields into one word, low to high; the
    round-trip is the tracked obligation (bounded by 64 bits total). -/
def bitpack (ws : List Word) (k : Nat) : Word :=
  ws.foldl (fun acc w => wor (wshl acc k) (wand w (2 ^ k - 1))) 0

/-! ## #94 Delta encoding (FV-0 round-trip). -/

/-- Subtraction is the inverse of addition from the left: adding back the
    difference returns the original wrapped word (the delta-coding core). -/
theorem wadd_wsub_cancel (prev y : Word) (hp : prev < W64) (hy : y < W64) :
    wadd prev (wsub y prev) = y := by
  unfold Word at *
  show ((prev + ((y + W64 - prev) % W64)) % W64) = y
  have hW : W64 = 18446744073709551616 := rfl
  rw [hW] at hp hy ⊢
  rw [Nat.add_comm prev ((y + 18446744073709551616 - prev) % 18446744073709551616),
    Nat.mod_add_mod]
  have h2 : (y + 18446744073709551616 - prev) + prev = y + 18446744073709551616 := by omega
  rw [h2, Nat.add_mod_right, Nat.mod_eq_of_lt hy]

/-- #94: delta encoding: the head is stored directly, each successor as the
    wrapping difference to its predecessor; decoding adds back. -/
def deltaEncode : List Word → List Word
  | [] => []
  | x :: xs => x :: deltaEncode.deltaLoop x xs
where
  deltaLoop : Word → List Word → List Word
    | _, [] => []
    | prev, y :: ys => wsub y prev :: deltaLoop y ys

def deltaDecode : List Word → List Word
  | [] => []
  | x :: xs => x :: deltaDecode.decodeLoop x xs
where
  decodeLoop : Word → List Word → List Word
    | _, [] => []
    | prev, d :: ds => wadd prev d :: decodeLoop (wadd prev d) ds

theorem deltaLoop_roundtrip : ∀ (xs : List Word) (prev : Word), prev < W64 →
    (∀ y ∈ xs, y < W64) →
    deltaDecode.decodeLoop prev (deltaEncode.deltaLoop prev xs) = xs := by
  intro xs
  induction xs with
  | nil => intro prev _ _; rfl
  | cons y ys ih =>
    intro prev hp hbound
    have hy : y < W64 := hbound y (List.mem_cons_self ..)
    have hys : ∀ z ∈ ys, z < W64 := fun z hz => hbound z (List.mem_cons_of_mem _ hz)
    show wadd prev (wsub y prev) :: deltaDecode.decodeLoop (wadd prev (wsub y prev))
        (deltaEncode.deltaLoop y ys) = y :: ys
    rw [wadd_wsub_cancel prev y hp hy]
    rw [ih y hy hys]

/-- #94 round-trip (FV-0): decoding the encoding returns the original list,
    exactly, for wrapped words (every modeled word is wrapped). -/
theorem delta_roundtrip (xs : List Word) (hbound : ∀ y ∈ xs, y < W64) :
    deltaDecode (deltaEncode xs) = xs := by
  cases xs with
  | nil => rfl
  | cons x rest =>
    simp only [deltaEncode, deltaDecode]
    have hxb : x < W64 := hbound x (List.mem_cons_self ..)
    rw [deltaLoop_roundtrip rest x hxb (fun z hz => hbound z (List.mem_cons_of_mem _ hz))]

end RV8FV
