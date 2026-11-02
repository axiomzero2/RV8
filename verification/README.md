# Verification Notes

## Z3
The SMT proofs for this repository live in tools/z3_verify/ and verification/.
The 2026-11-03 session adds:
- verification/z3_warm_incremental.py: models the timeline network's job-pair
  structure and proves, for small instances, (a) any integral flow of value R is a
  feasible lane-disjoint assignment, (b) the warm-start seeding (scan chords plus
  idle chains) satisfies flow conservation, and (c) the never-regress promotion
  makes the published assignment's cost equal the brute-force minimum.
- verification/z3_query_soundness.py: models the memo validity rule (deps hash
  equality) and proves memoized reads equal recomputation while dependencies are
  unchanged, and that a dependency change forces re-execution.

## Isabel
The Isabel (Isabelle/HOL) formal verification tool is not installable in the build
sandbox (no apt candidate, and the full Isabelle distribution is out of scope for
the image). The Z3 proofs above are the machine-checked evidence of record; an
Isabelle session formalizing the timeline-network reduction is future work and
should reuse the statements in the two scripts above.
