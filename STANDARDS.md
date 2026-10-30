# Compiler 2.0 Standards Manifest

Status: normative review baseline for all components and all agent-built work
Owner: axiomzero2
Supersedes: none
Companion specs: docs/specs/multi_level_ir.md, standard.md (CEP&CC 0.1), doc_standard.md

---

## 1. Purpose

Every component of RV8, whether built directly or by parallel agents, is reviewed against this manifest before acceptance. The manifest states the per-component invariants, the acceptance checks, and the non-negotiable properties that span the whole compiler. A component that violates any item here is rejected regardless of test status.

## 2. Global Non-Negotiables

1. **Zero external dependencies.** All crates build with no third-party crates. `Cargo.lock` contains first-party path dependencies only.
2. **Deterministic translation (38.10, 38.19).** Same input plus same flags plus same toolchain produces byte-identical output. No HashMap iteration may reach output ordering; order by index, sort by stable keys, or use ordered structures.
3. **Fail-closed, never panic (38.6).** Library code returns Result for every fallible operation. Panics are permitted only in tests. Untrusted input cannot drive recursion depth; all traversals over user-provided structure are iterative.
4. **IR contract (38.17).** Every IR level is printable, hashable, and index-identified. Identity is dense u32 indices into flat vectors, never pointers.
5. **Verification after every pass (38.18).** Each IR level exposes a verifier; the pipeline runs it after construction and after every transformation.
6. **Named budgets (38.12).** Every per-function or per-graph limit is a named constant (for example MAX_HIR_ITEMS), never a magic number scattered at call sites.
7. **CEP comment discipline.** Every file opens with CEP:FILE metadata; every public item carries CEP:WHAT/WHY/STATUS/FAILURE/ASSUMES/COST/EVIDENCE. STATUS is honest: stub, partial, or complete.
8. **Documentation standard.** docs/ follows doc_standard.md: objective, precise, complete, no marketing language.

## 3. Optimization Baseline

The curated catalog contains 176 optimizations in six categories:

| Range | Category |
|-------|----------|
| 1-30 | Control flow and structural |
| 31-70 | Data flow and value-based |
| 71-95 | Memory and cache |
| 96-120 | Parallelism and vectorization |
| 121-140 | Procedure and link-time |
| 141-176 | Advanced and systems |

Removed by design decision (do not implement): e-graphs/rewriting saturation, polyhedral compilation, quantum abstractions, JIT-specific infrastructure. The catalog count is 176 after these removals.

## 4. Five-Level IR Contract (docs/specs/multi_level_ir.md)

| Level | Name | Container | Key invariants |
|-------|------|-----------|----------------|
| 0 | HIR | rv8_ir::hir | region tree acyclic and nested; loops have exactly one header and one latch; lexical scoping; valid break/continue/return targets |
| 1 | Sea of Nodes | rv8_ir::son | SSA, single entry, phi at merges, operands dominate uses (loops excepted via back-edges) |
| 2 | Heterogeneous IR | rv8_ir::hetir | six node classes, six edge kinds; cross-space memory needs ordering edges; communication carries source and destination spaces; sync carries scope; no dead pure nodes; data edges form a DAG |
| 3 | LIR | rv8_ir::lir | blocks end in explicit transfers; every value has a register class; reducible CFG |
| 4 | Machine IR | rv8_ir::machir | ISA-legal opcodes; register-class signatures respected; ABI-conformant frame (alignment power of two >= 16, size multiple); valid addressing modes (x86 scale in 1/2/4/8, no index without scale coherence; AArch64 base + imm9 only in this contract) |

Transitions: 6.1 flatten (HIR to Sea of Nodes, phi placement, back-edges), 6.2 linearize (deterministic Kahn order keyed by node id), 6.3 lower (virtual registers, block formation, terminator synthesis). Transition 6.4 (LIR to Machine IR: isel, MCF allocation, scheduling) belongs to the backend.

Conventions pinned by implementation:
- Region execution order: statements before child regions; post-construct code lives in sibling tail blocks.
- Joins own synthesized blocks beyond region ids; a region block never hosts two control points.
- Sea-of-Nodes input convention: Region/Merge inputs all control; Branch [cond, control]; Return [value?, control]; Phi all data.

## 5. Component Invariants and Acceptance Checks

### 5.1 MCF Register Allocation (rv8_backend::regalloc)
Min-cost multi-commodity flow via successive shortest paths with Johnson potentials; spill edges have infinite capacity so feasibility holds by construction; heterogeneous register classes with sub-register aliasing; deterministic tie-breaking; linear-scan warm start; incremental and region-based re-solve. Accept: allocation is optimal for small instances against a brute-force checker, deterministic across runs, and respects ABI pre-coloring.

### 5.2 Instruction Selection (rv8_backend::isel)
Bottom-up DP over the DAG with memoized ValueCell {form, cost, operands, instrs, rank}; chain folding requires single-user checks with bounded depth (MAX_CHAIN_DEPTH = 96); per-target rule set and cost model; tie-break cost, then instruction count, then rank, then pattern index. Accept: selection is cost-optimal on tree-shaped expressions and deterministic on DAGs.

### 5.3 Autotuner (rv8_autotuner)
Plackett-Luce pass-ordering logits plus discrete iterations plus continuous factors; one model: conditional graph transformer with delta and value heads; MF-PES acquisition divided by fidelity cost; four-tier async funnel (surrogate filter, partial-IR execution, cycle-accurate simulation, silicon); MinHash/Jaccard warm start; autotuner RNG seeded from LIR hash plus hardware fingerprint so self-hosting stays reproducible. Accept: cache hit reproduces a prior configuration bit-for-bit; no allocation in the hot path.

### 5.4 Borrow Checker (rv8_middle::borrowck)
Graph CSP over the Sea of Nodes; region inference via Tarjan SCC condensation with reverse-topological bitset propagation; validation is bitset intersection (no alias overlap between conflicting borrows, no use after move); drop-graph cycle detection; liveness bitsets feed the allocator directly. Accept: accepts and rejects exactly the reference corpus; deterministic.

### 5.5 Object Emission and Linker (rv8_object)
Format-agnostic universal object model with deterministic ELF/Mach-O/PE serializers; section layout by permission and alignment; string dedup with suffix sharing; section GC by reachability; weak/strong conflict rules. Accept: two runs over the same input emit identical bytes (cmp passes).

### 5.6 Partial Escape Analysis (rv8_middle::pea)
Path-sensitive escape sets; scalar replacement when escapes do not reach the hot path; AllocationSiteId provenance from HIR through Level 2 meta nodes; must be deterministic because self-hosting hashes compiler outputs.

### 5.7 Frontend (rv8_frontend)
TokenTree with nested delimiters and hygiene via SyntaxContext; recursive descent with speculative parse and backtrack; Union-Find inference; recursive trait solving with memoized cache; decision-tree exhaustiveness. Frontend output is HIR satisfying section 4 level-0 invariants.

### 5.8 Runtime (rv8_runtime)
no_std; primitives lowered to intrinsics; compiler-rt software intrinsics for 128-bit division, soft-float, and SIMD memory ops; bump allocator for bare metal; unwinding via .eh_frame tables or abort.

### 5.9 ABI (rv8_backend::abi)
External C ABI per OS (SysV AMD64, Windows x64, ARM64) plus an internal Rust ABI; stack-clash probing; zero-cost unwinding CFI; pre-coloring constrains the allocator.

### 5.10 Incremental Compilation (rv8_driver::cache)
Content-addressed storage keyed by BLAKE3; module dependency invalidation; pass dependency matrix; autotuner cache keyed on (LIR hash, hardware fingerprint, compiler version).

### 5.11 Driver (rv8_driver)
Immutable session; demand-driven memoized queries; crate-DAG parallelism; rustc-compatible CLI; deterministic .d files.

### 5.12 Self-Hosting
Stage 2 output must equal Stage 3 output bit-for-bit. Fixed-seed hashing everywhere; scheduling-independent ordering; metadata sanitization (no wall-clock timestamps, no absolute paths).

## 6. Agent Review Checklist

For every component delivered by a build agent, the reviewer records:

1. CEP annotations present and truthful on every public item; STATUS reflects reality.
2. Verifier exists, runs after every pass, and rejects each violated invariant from the spec (one negative test per invariant minimum).
3. Determinism test exists (same input twice, identical hash or bytes).
4. No panic paths reachable from library code; no recursion over untrusted structure.
5. No external crates added; no new dependencies in Cargo.lock.
6. Public API documented per doc_standard.md; cargo doc passes with -D warnings.
7. clippy clean; build clean under RUSTFLAGS="-D warnings"; cargo test passes for the crate.
8. Component-level acceptance check from section 5 demonstrated with evidence in the CEP:EVIDENCE lines.

## 7. CI Gates

Enforced by .github/workflows/ci.yml: warnings-as-errors build, workspace tests, rustdoc with -D warnings, CEP lint over crates/tools/tests, end-to-end compile-and-run via the driver, and a reproducible-build cmp check. A merge is accepted only when all gates pass or the failing gate has an open ticket with STATUS stub recorded here.

---

## 8. Convention Pins from Integration (session of 2026-10-30)

Pinned by implementation, enforced by tests:

1. **Parameter threading (spec 1.2/2.2/6.1).** `HirFunction::params: Vec<VarId>` (order = ABI argument position); `hir_to_son` binds each to `Opcode::Parameter(i)`; `son_to_hetir` maps to `ComputeOp::Param`; the MCF allocator pins never-defined-but-used values to `abi::ARG_REGISTERS` in order and return values to RAX.
2. **Phi lowering (spec 6.2/6.3).** LIR 4.3.1 relaxation is normative: header/join phis lower to `mov` copies placed at the END of each predecessor block (edge copies), one definition per incoming path; the allocator treats [first definition, last use] as one live range with interior rewrites. Copy sets that interleave (swap shapes) sequence through one scratch value, ready-order, deterministic.
3. **Conservative wrap-sound liveness (RV8-137, 2026-10-31).** Linear layout order linearizes liveness only for forward flows. A value live-out of a block with a layout-earlier successor (a back-edge), and every multi-def (phi) value, takes the interval [first definition, last program point]: no single-def value may take its lane after its last layout use. The block-level backward dataflow that detects this runs under `MAX_LIVENESS_ROUNDS`. The rule is conservative (reserves the whole tail); interval splitting is the precise replacement and remains future work.
4. **Lane-private MCF job pairs (RV8-137, 2026-10-31).** Each (range, register) job pair is private to one register timeline: entry and exit arcs touch the same chain, so a unit of flow can never migrate between registers. Because idle timelines profitably re-take priced chords, the solve is followed by a deterministic assignment repair (keep the cheapest flowing chord per range, cancel surplus chords, then fill untaken ranges through idle lanes in definition order, cheapest eligible lane first). The brute-force corpora (single-def and multi-def) are the standing arbiters of the assignment's exactness.
5. **Expression flattener operand order (RV8-137, 2026-10-31).** The LIFO work stack of `hir_to_son` requires right-to-left pushes: Binary pushes right before left, Call pushes arguments in reverse. `Combine` consumes the value stack in push order, so operands arrive in source order. Pinned by the operand-order regression batch in rv8_frontend::tests.
6. **Return-value threading (RV8-137, 2026-10-31).** A Return with a value-bearing data input lowers to `Ret [value]`; isel emits the ABI move into RAX and the allocator pins the value for its whole range unless an ABI argument pin already holds (a returned parameter keeps RDI and isel emits the register-to-register move). A value-less Return (bare or tail call) stays bare: the tail-call result reaches RAX by the call convention and call results have no virtual registers (consuming one in a live compute is rejected, never miscompiled).
7. **Data-DAG exception (spec 3.3).** Data edges form a DAG except through `Compute(Phi)` nodes, which are cycle-closure points for loop-carried values (mirrors the SoN dominance exception 2.3.1). A data cycle containing no phi still rejects.
8. **Branch successors (spec 5.3).** LIR/MIR block successors for a conditional branch are `[taken, fallthrough]` and the fallthrough must be the layout-next block; when the true arm is layout-next the branch condition is inverted (Ne swapped for Eq), preserving semantics.
9. **Structural blocks.** The flattener assigns one block id per HIR region; `hetir_to_lir` compacts to blocks reachable from entry (deterministic renumber) so every emitted block is reachable and terminated.

Open tickets recorded under section 7 rules:
- **RV8-133** cache.rs: BLAKE3 core to replace the SHA-256 interim (STATUS partial).
- **RV8-134** query.rs: full demand-driven memoized query system (STATUS partial).
- **RV8-135** rv8_ir: Display for IR error types (driver formats via Debug meanwhile).
- **RV8-140** backend: MCF linear-scan warm start and incremental/region re-solve (STANDARDS 5.1 acceptance sentence already met).

Resolved tickets:
- **RV8-137** (resolved 2026-10-31): the loop end-to-end test `pipeline_runs_loop_program` is un-ignored and green. Root-cause chain, fixed in one pass: (1) shared MCF job pairs let a flow unit enter a chord on one register and exit on another, double-booking lanes while entry-arc attribution read only the entry side; (2) linear intervals missed back-edge wrap liveness for BOTH multi-def (phi) and single-def (loop constant) values; (3) `hetir_to_lir` dropped the Return value input, so results reached RAX only through allocation luck; (4) the ret pin overwrote parameter pins; (5) the expression flattener delivered Binary operands and Call arguments in reverse (every non-commutative operator and call was miscompiled; the fixture passed only because addition commutes); (6) the priced chords let idle timelines multi-book popular ranges and displace fresh ones (the assignment repair now restores assignment semantics on the solved flow). Evidence: the RV8-137 regression batch (twenty tests, crates/rv8_backend/tests/rv8_137_regression.rs), the operand-order batch (twenty tests, crates/rv8_frontend/tests/operand_order_regression.rs), the return-threading batch (twenty tests, crates/rv8_ir/tests/ret_value_regression.rs), and the two brute-force corpora (single-def, 24 seeds; multi-def, 12 seeds).
