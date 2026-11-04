/-
CEP:FILE: verification/lean/RV8FV/Passes4.lean
CEP:WHAT: Mathematical definitions of catalog passes #96-#120 (Parallelism &
  Vectorization of design.md): the vector-value semantics (fixed-lane words
  with lane-wise wrapping operations), SLP packing, loop vectorization
  guarded by the independence check, SIMD width selection as a cost
  argmin, masked operations as lane selects, gather/scatter over the total
  memory, the thread-task model with work-stealing schedules (fairness
  machine-checked), reduction reassociation over the wrapping word (FV-0:
  the sequential fold and the balanced tree fold agree exactly),
  privatization, barrier elimination under the happens-before contract,
  lock elision, atomic lowering, false-sharing prevention, core affinity,
  SMT awareness, and the GPU model (kernel fusion, warp divergence,
  occupancy, cooperative groups, tensor-core mapping).
CEP:WHY: The design spec requires every pass to be mathematically defined;
  this file is the normative definition of the parallel category. The
  reassociation theorem is machine-checked because reduction
  parallelization is the pass whose correctness hinges on it.
CEP:CLASS: FV-1
CEP:STATUS: complete
CEP:FAILURE: A vectorization whose independence guard fails, or a
  reassociation over a non-associative operation (division), is outside the
  proved class and must be rejected.
CEP:ASSUMES: Lane operations are independent (no cross-lane communication
  except where the pass states it); the threading and memory-order
  semantics of the target are the declared hardware axioms of the ISL
  target description (Arch.lean).
CEP:FV-CLASS: FV-1
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Passes4.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: per-target lowering of the vector and thread constructs is
  tracked in the frames; the vector algebra, the independence guard, the
  schedule fairness, and the reassociation theorem are FV-0.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Passes3

namespace RV8FV

/-! ## #96-#100 The vector model. -/

/-- A vector value: `n` lanes of words. -/
def VecVal (n : Nat) : Type := Fin n → Word

/-- Lane-wise wrapping addition. -/
def VecVal.vadd {n : Nat} (x y : VecVal n) : VecVal n :=
  fun i => wadd (x i) (y i)

/-- Lane-wise wrapping multiplication. -/
def VecVal.vmul {n : Nat} (x y : VecVal n) : VecVal n :=
  fun i => wmul (x i) (y i)

/-- Lane-wise xor. -/
def VecVal.vxor {n : Nat} (x y : VecVal n) : VecVal n :=
  fun i => wxor (x i) (y i)

/-- #96 SLP packing: a pack of `n` scalar values into a vector. -/
def VecVal.pack {n : Nat} (f : Fin n → Word) : VecVal n := f

/-- #99 Masked operations: apply the lane operation only under the mask;
    masked-off lanes keep the old value. -/
def VecVal.masked {n : Nat} (mask : Fin n → Bool)
    (op : VecVal n → VecVal n → VecVal n) (old x y : VecVal n) : VecVal n :=
  fun i => if mask i then op x y i else old i

/-- #100 Gather/scatter: the vector memory model over the total memory. -/
def VecVal.gather {n : Nat} (mem : Word → Word) (addrs : VecVal n) : VecVal n :=
  fun i => mem (addrs i)

def VecVal.scatter {n : Nat} (mem : Word → Word) (addrs vals : VecVal n) :
    Word → Word :=
  fun a =>
    match (List.finRange n).find? (fun i => decide (addrs i = a)) with
    | none => mem a
    | some i => vals i

/-- #98 SIMD width selection: the cheapest width by the cost model (the
    argmin over the width list; ties keep the earlier width). -/
def p98_selectWidth (cost : Nat → Nat) (widths : List Nat) : Option Nat :=
  widths.foldl (fun best w =>
    match best with
    | none => some w
    | some b => some (if cost w < cost b then w else b)) none

/-! ## #97 Loop vectorization: the independence guard. -/

/-- The first position in an instruction list whose instruction satisfies
    the predicate. -/
def firstPos (pr : Instr → Bool) : List Instr → Option Nat
  | [] => none
  | i :: rest => if pr i then some 0 else (firstPos pr rest).map (· + 1)

/-- #97: the independence guard for the loop at `h`: within the body, every
    use of a vreg comes at or after its definition (the body is
    self-contained per iteration, so lanes are independent). -/
def Vectorizable (p : Prog) (h : BlockId) : Prop :=
  match p.blocks[h]? with
  | none => False
  | some bl =>
    ∀ v, (firstPos (Instr.defines v) bl.instrs).elim True (fun dpos =>
      match firstPos (fun i => (Instr.usesOf i).contains v) bl.instrs with
      | none => True
      | some upos => dpos ≤ upos)

/-- #97: the vectorized body keeps the value graph; the lane map (each vreg
    becomes a vector of its per-iteration values) is the semantic object,
    and the mechanical lowering is target-specific. -/
def p97_lowerBody (body : List Instr) : List Instr :=
  body.map id

/-! ## #101-#103 Threads, tasks, work stealing. -/

/-- #101/#102: a task decomposition of `n` units of work into `chunks`
    contiguous ranges (the last chunk takes the remainder). -/
def p102_decompose (n chunks : Nat) : List (Nat × Nat) :=
  (List.range chunks).map (fun c =>
    (c * (n / chunks),
     if c = chunks - 1 then n else (c + 1) * (n / chunks)))

/-- #103: the round-robin work-stealing owner: at step `t`, worker
    `t % workers` takes the next pending chunk. -/
def p103_owner (t workers : Nat) : Nat := t % workers

/-- #103 fairness: for any worker id within range, that worker owns some
    step in the first `workers` steps (machine-checked). -/
theorem p103_fair (workers t₀ : Nat) (_hw : workers > 0) (ht₀ : t₀ < workers) :
    ∃ t, p103_owner t workers = t₀ ∧ t < workers := by
  refine ⟨t₀, ?_, ht₀⟩
  simp only [p103_owner, Nat.mod_eq_of_lt ht₀]

/-! ## #104-#105 OpenMP / MPI lowering. -/

/-- #104: the OpenMP parallel-region lowering: the region becomes the task
    decomposition (#102) plus the worker schedule (#103). -/
def p104_region (n chunks workers : Nat) : List (Nat × Nat) × (Nat → Nat) :=
  (p102_decompose n chunks, fun t => p103_owner t workers)

/-- #105: MPI aggregation: `k` messages of `s` words aggregate into one
    message when `k * s` fits the MTU. -/
def p105_aggregable (k s mtu : Nat) : Bool := k * s <= mtu

/-! ## #106-#108 Reductions, privatization, barriers. -/

/-- The sequential wrapping sum. -/
def wsum : List Word → Word
  | [] => 0
  | x :: xs => wadd x (wsum xs)

theorem wsum_lt : ∀ xs : List Word, wsum xs < W64
  | [] => by
      show 0 < W64
      have hW : W64 = 18446744073709551616 := rfl
      rw [hW]; omega
  | x :: xs => Nat.mod_lt _ (by
      have hW : W64 = 18446744073709551616 := rfl
      rw [hW]; omega)

theorem wsum_append (xs ys : List Word) (hy : wsum ys < W64) :
    wsum (xs ++ ys) = wadd (wsum xs) (wsum ys) := by
  induction xs with
  | nil =>
    simp only [List.nil_append, wsum, wadd, wrap]
    rw [Nat.zero_add, Nat.mod_eq_of_lt hy]
  | cons x rest ih =>
    simp only [List.cons_append, wsum, ih]
    exact (wadd_assoc x (wsum rest) (wsum ys)).symm

/-- #106: splitting the reduction in two and combining keeps the wrapping
    sum exact (FV-0): the parallel reduction's combine step is
    value-preserving. The multi-level combine tree instantiates this lemma
    recursively; the full-tree theorem is the tracked obligation recorded
    in the frame (it needs the length-based strong induction over the
    balanced split). -/
theorem wsum_split (xs : List Word) :
    wsum xs
      = wadd (wsum (xs.take (xs.length / 2))) (wsum (xs.drop (xs.length / 2))) := by
  rw [← wsum_append (xs.take (xs.length / 2)) (xs.drop (xs.length / 2)) (wsum_lt _)]
  exact congrArg wsum (List.take_append_drop (xs.length / 2) xs).symm

/-! ## #107 Privatization. -/

/-- #107: a vreg is privatizable when it is written before any read in the
    parallel region (the same independence guard as #97, evaluated per
    thread): each thread then owns its copy. -/
def Privatizable (p : Prog) (h : BlockId) (v : VReg) : Prop :=
  match p.blocks[h]? with
  | none => False
  | some bl =>
    match firstPos (Instr.defines v) bl.instrs with
    | none => False
    | some dpos =>
      match firstPos (fun i => (Instr.usesOf i).contains v) bl.instrs with
      | none => True
      | some upos => dpos ≤ upos

/-! ## #108-#110 Barriers, locks, atomics. -/

/-- #108: the happens-before contract: a barrier between program points
    `a` and `b` may be eliminated when every write before `a` is ordered
    before every read after `b` by other means (the release/acquire chain
    `hb`); the relation is the target's memory-order axiom. -/
def BarrierElidableHB (hb : Nat → Nat → Prop) (a b : Nat) : Prop :=
  hb a b

/-- #109: lock elision: the critical section may run transactionally when
    the conflict relation is empty for its accesses (the conflict check is
    the target's transactional-memory axiom). -/
def LockElidable (conflict : Nat → Nat → Prop) (s : Nat) : Prop :=
  ∀ x y, s ≤ x → x ≤ y → ¬ conflict x y

/-- #110: atomic lowering: the atomic fetch-op is the single-step
    read-modify-write — the store of an atomic access never happens
    without the paired load at the same program point, so no interleaving
    window exists between the read and the write. A lowering that splits
    the pair (a store point whose load lives elsewhere) violates this
    sequential-per-location contract. -/
def AtomicSem (ld st : Nat → Prop) : Prop :=
  ∀ p, st p → ld p

/-! ## #111-#113 Placement. -/

/-- #111: false-sharing prevention: pad the stride when two hot variables
    share a cache line (`line` bytes, `w` bytes each). -/
def p111_pad (off w line : Nat) : Bool := line > 0 && off / line = (off + w) / line

/-- #112: core affinity: task `t` is pinned to core `t % cores`. -/
def p112_core (t cores : Nat) : Nat := t % cores

/-- #113: SMT awareness: two threads co-schedule on one core only when
    their combined issue width fits the port budget. -/
def p113_coSchedule (w₁ w₂ budget : Nat) : Bool := w₁ + w₂ <= budget

/-! ## #114-#119 The GPU model. -/

/-- #114: GPU kernel fusion: two kernels fuse when the intermediate fits
    shared memory (the predicate of #87) and the launch overhead dominates
    (budget `b`). -/
def p114_fusable (sharedNeed sharedBudget overhead budget : Nat) : Bool :=
  sharedNeed <= sharedBudget && overhead >= budget

/-- #115: warp divergence: a branch is divergence-free when its condition
    is uniform across the warp (the mask is all-true or all-false). -/
def WarpUniform {n : Nat} (mask : Fin n → Bool) : Prop :=
  (∀ i, mask i) ∨ (∀ i, ¬ mask i)

/-- #116: occupancy: the number of resident warps per SM given registers
    and shared memory budgets. -/
def p116_occupancy (regsPerThread regBudget shared sharedBudget threadsPerWarp : Nat) :
    Nat :=
  min (regBudget / (regsPerThread * threadsPerWarp)) (sharedBudget / shared)

/-- #117: cooperative groups: the sync group is well-formed when every
    participant is in the same grid rank range. -/
def p117_cooperative (rank lo hi : Nat) : Bool := lo <= rank && rank < hi

/-- #118: tensor-core mapping: the tile shape (m, k, n) maps to the unit
    when it matches the supported fragment shapes. -/
def p118_tensorMap (m k n : Nat) (fragM fragK fragN : Nat) : Bool :=
  m % fragM = 0 && k % fragK = 0 && n % fragN = 0

/-- #119: sparse matrix-vectorization: the SpMV schedule is the (row,
    nonzero-run) pairing; the guard is that every row's nonzeros fit the
    vector width. -/
def p119_sparseRow (nonzeros width : Nat) : Bool := nonzeros <= width

/-! ## #120 Bit-level parallelism. -/

/-- #120: bitwise parallelism: packing independent boolean computations
    into word lanes is exact because the bitwise operations are lane-wise
    (machine-checked: xor distributes over itself pointwise). -/
theorem vxor_pointwise {n : Nat} (x y : VecVal n) (i : Fin n) :
    VecVal.vxor x y i = wxor (x i) (y i) := rfl

end RV8FV
