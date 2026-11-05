/-
CEP:FILE: verification/lean/RV8FV/Wrd.lean
CEP:WHAT: The algebraic laws of the 64-bit wrapping word that the value passes
  rely on: commutativity/associativity of wrapping add and multiply, left
  distributivity over wrapping add, the add/sub round-trip, wrapping identity
  elements, the bitwise xor laws (commutativity, associativity, self-inverse,
  zero), and the strength-reduction law (multiplication by 2^k is a left
  shift by k, for k < 64).
CEP:WHY: The value-rewriting passes of design.md categories 2 and 4 rewrite
  computations using exactly these identities. Proving them once here, over
  the bounded 2^64 word (overflow modeled, never simplified away, CEP&CC
  54.4.1), makes every downstream use a machine-checked fact instead of a
  paper claim.
CEP:CLASS: FV-0
CEP:STATUS: complete
CEP:FAILURE: Any pass rewrite rule not derivable from these laws plus the
  trap-preserving folding rule of Fold.lean is outside the proved
  value-rewrite class.
CEP:ASSUMES: Shift counts are masked to 6 bits as in Spec.lean (x86 semantics).
  `wrap` is idempotent, so wrapped operands need no extra hypotheses.
CEP:FV-CLASS: FV-0
CEP:FV-PROVER: Lean 4 (leanprover/lean4:v4.35.0, pinned in lean-toolchain)
CEP:FV-SCRIPT: verification/lean/RV8FV/Wrd.lean
CEP:FV-SORRY-COUNT: 0
CEP:FV-REFINEMENT: see Spec.lean; the Rust value-rewriting passes are bridged by
  the deterministic differential suites.
CEP:FV-AXIOMS: none.
-/

import RV8FV.Spec

namespace RV8FV

/-! ## Wrapping basics. -/

/-- Wrapping is idempotent: the wrapped value is already in the 64-bit domain. -/
theorem wrap_wrap (a : Word) : wrap (wrap a) = wrap a :=
  Nat.mod_mod_of_dvd a (Nat.dvd_refl W64)

/-- Multiplication is unaffected by wrapping the right operand first. -/
theorem mul_mod_right_raw (a b : Word) : (a * (b % W64)) % W64 = (a * b) % W64 := by
  rw [Nat.mul_comm a (b % W64), Nat.mod_mul_mod, Nat.mul_comm b a]

theorem wrap_mul_right (a b : Word) : wrap (a * wrap b) = wrap (a * b) :=
  mul_mod_right_raw a b

/-! ## Wrapping add. -/

theorem wadd_comm (a b : Word) : wadd a b = wadd b a := by
  show (a + b) % W64 = (b + a) % W64
  rw [Nat.add_comm]

theorem wadd_assoc (a b c : Word) : wadd (wadd a b) c = wadd a (wadd b c) := by
  show ((a + b) % W64 + c) % W64 = (a + (b + c) % W64) % W64
  rw [Nat.mod_add_mod, Nat.add_comm a ((b + c) % W64), Nat.mod_add_mod]
  have h : a + b + c = b + c + a := by unfold Word; omega
  rw [h]

theorem wadd_zero (a : Word) : wadd a 0 = wrap a := by
  show (a + 0) % W64 = a % W64
  rw [Nat.add_zero]

/-- Subtraction is the inverse of addition on the 64-bit domain: the round
    trip returns the wrapped original. -/
theorem wsub_wadd_cancel (a b : Word) (ha : a < W64) (hb : b < W64) :
    wsub (wadd a b) b = a := by
  unfold Word at *
  have hW : W64 = 18446744073709551616 := rfl
  simp only [wadd, wsub, wrap] at *
  rw [hW] at ha hb ⊢
  have hb' : b ≤ 18446744073709551616 := by omega
  rw [Nat.add_sub_assoc hb', Nat.mod_add_mod]
  have h2 : a + b + (18446744073709551616 - b) = a + 18446744073709551616 := by omega
  rw [h2, Nat.add_mod_right]
  exact Nat.mod_eq_of_lt ha

/-! ## Wrapping multiply. -/

theorem wmul_comm (a b : Word) : wmul a b = wmul b a := by
  show (a * b) % W64 = (b * a) % W64
  rw [Nat.mul_comm]

theorem wmul_assoc (a b c : Word) : wmul (wmul a b) c = wmul a (wmul b c) := by
  show (((a * b) % W64) * c) % W64 = (a * ((b * c) % W64)) % W64
  rw [Nat.mod_mul_mod, mul_mod_right_raw]
  congr 1
  exact Nat.mul_assoc a b c

theorem wmul_one (a : Word) : wmul a 1 = wrap a := by
  show (a * 1) % W64 = a % W64
  rw [Nat.mul_one]

theorem wmul_zero (a : Word) : wmul a 0 = 0 := by
  show (a * 0) % W64 = 0
  rw [Nat.mul_zero, Nat.zero_mod]

/-- Left distributivity: multiplication distributes over wrapping addition
    exactly, overflow included. -/
theorem wmul_wadd_distr (a b c : Word) :
    wmul a (wadd b c) = wadd (wmul a b) (wmul a c) := by
  show (a * ((b + c) % W64)) % W64
      = (((a * b) % W64) + ((a * c) % W64)) % W64
  rw [mul_mod_right_raw, Nat.left_distrib, ← Nat.add_mod]

/-! ## Bitwise xor laws (used by check-elimination and bit-level passes). -/

theorem wxor_comm (a b : Word) : wxor a b = wxor b a := Nat.xor_comm a b

theorem wxor_assoc (a b c : Word) : wxor (wxor a b) c = wxor a (wxor b c) :=
  Nat.xor_assoc a b c

theorem wxor_self (a : Word) : wxor a a = 0 := Nat.xor_self a

theorem wxor_zero (a : Word) : wxor a 0 = a := Nat.xor_zero a

theorem wxor_invol (a b : Word) : wxor (wxor a b) b = a := by
  show ((a ^^^ b) ^^^ b) = a
  rw [Nat.xor_assoc, Nat.xor_self]
  exact Nat.xor_zero a

/-! ## Bitwise and/or basics available in core. -/

theorem wand_zero (a : Word) : wand a 0 = 0 := Nat.and_zero a
theorem zero_wand (a : Word) : wand 0 a = 0 := Nat.zero_and a

/-- The mask-subsumption law: if `m1 & m2 = m1` (every bit of the inner
    mask `m1` is also set in the outer mask `m2`), then masking by `m1`
    first makes the outer mask a no-op: `(v & m1) & m2 = v & m1`. This is
    the value identity of truncation elimination (catalog pass #69).
    Machine-checked over the per-bit `testBit` characterization of land. -/
theorem wand_submask (v m1 m2 : Word) (h : Nat.land m1 m2 = m1) :
    wand (wand v m1) m2 = wand v m1 := by
  show Nat.land (Nat.land v m1) m2 = Nat.land v m1
  apply Nat.eq_of_testBit_eq
  intro i
  show ((v &&& m1) &&& m2).testBit i = (v &&& m1).testBit i
  rw [Nat.testBit_and, Nat.testBit_and]
  cases hb1 : Nat.testBit m1 i with
  | false => simp
  | true =>
    have hband : Nat.testBit (m1 &&& m2) i = Nat.testBit m1 i := by
      rw [show (m1 &&& m2) = Nat.land m1 m2 from rfl, h]
    have hbt : Nat.testBit (m1 &&& m2) i = (Nat.testBit m1 i && Nat.testBit m2 i) :=
      Nat.testBit_and m1 m2 i
    rw [hband, hb1] at hbt
    have hper : Nat.testBit m2 i = true := by simpa using hbt
    rw [hper]
    simp

/-! ## Strength reduction: multiply by a power of two is a left shift. -/

/-- For `k < 64`, multiplying by `2^k` and shifting left by `k` agree exactly
    (catalog pass #13 strength reduction, machine-checked). -/
theorem wmul_pow2_eq_wshl (a : Word) (k : Nat) (hk : k < 64) :
    wmul a (2 ^ k) = wshl a k := by
  have hmod : k % 64 = k := Nat.mod_eq_of_lt hk
  show (a * 2 ^ k) % W64 = a * 2 ^ (k % 64) % W64
  rw [hmod]

end RV8FV
