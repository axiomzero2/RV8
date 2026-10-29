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
