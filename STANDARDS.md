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

Removed by design decision (do not implement): e-graphs/rewriting saturation, polyhedral compilation, quantum abstractions. The catalog count is 176 after these removals.

**JIT hybrid (RV8-141, 2026-11-04): in scope.** The earlier "remove JIT-specific infrastructure" decision is superseded. The compiler is an AOT-first JIT hybrid: design.md's "JIT Runtime & Adaptive Compilation" chapter (region-based partial deoptimization, on-stack replacement, the runtime code emitter, the AOT-to-JIT transition, and the runtime-optimization catalog) is binding, executed at hybrid tiering fidelity - the AOT pipeline remains the correctness oracle and the fallback tier, and every JIT path must be disableable so pure-AOT behavior stays reproducible for self-hosting.

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

### 5.13 JIT Hybrid (rv8_runtime::jitmem, rv8_backend, rv8_middle)
The runtime code emitter maps W^X executable memory (write-never-execute and execute-never-write, mmap-based on POSIX targets), encodes through the same two-pass deterministic layout as the object writer, and patches through recorded relocation records only. Region-based partial deoptimization carries per-region interpreter state summaries (DeoptState) produced alongside MachineIR; OSR entry points are generated from the same allocation as the entry code; tiering decisions are content-seeded and deterministic per input. Accept: emitted code executes and returns the expected result in-process; a deopt reconstructs the interpreter state exactly; disabling the JIT yields byte-identical AOT output.

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
10. **ISL target descriptions (2026-10-31).** Target facts are data: `rv8_isl` defines the description grammar, validation, and model; `crates/rv8_isl/targets/x86-64.isl` is the description of record for the shipped target, cross-checked lane-for-lane against the `rv8_backend::abi` tables by test. The allocator core is parameterized by `regalloc::desc::Plan` and entered through `allocate_with(plan, lir)`; `allocate(lir)` is the x86-64 default. The toy four-lane description (`targets/toy4.isl`) proves one core serves two targets (the same LIR spills there and not on x86-64).
11. **Structural blocks.** The flattener assigns one block id per HIR region; `hetir_to_lir` compacts to blocks reachable from entry and lays them out in deterministic DFS reverse post-order so every emitted block is reachable, terminated, and every use follows a definition in layout order (back edges are the only backward transfers; id-order renumbering placed synthesized joins before the arms defining their phi inputs).

Convention pins from the RV8-137 follow-up session (2026-10-31, all enforced by tests):

12. **Value plumbing law (spec 6.3).** A non-Unit call DEFINES its result as an LIR value and isel emits the RAX result move immediately after the call; a function's returned value rides the Ret instruction as its single operand. Straight-line fixtures are not evidence for either: they pass through RAX residue.
13. **Branch arm provenance.** Branch TRUE/FALSE arms derive from the Branch node's control out-edges ordered by target node id (the flattener creates the TRUE-arm node first), never from block numbering or successor-list order; layout renumbering must not change semantics.
14. **Phi-copy sequencing.** A copy is emittable when its DESTINATION is not a pending source; every pure cycle breaks through its own fresh scratch value with its own deferred delivery. Multi-def values may be read before their same-block redefinition (the read consumes the incoming value); validation uses the global linearized first-definition table.
15. **Spill-scratch soundness.** A spill scratch write is permanent: source and memory scratches avoid the instruction's reads plus the exact live-out set at that MIR position (backward liveness initialized from block-crossing lanes); a destination scratch avoids only the live-out set (x86 reads operands before writing the destination). Immediate definitions spill through `StoreStackImm` with no register.
16. **Test wiring.** Every `tests.rs`/`tests/` file must be declared as a module or integration target; an orphaned test file is a standards violation (the regalloc corpus shipped unwired with a syntax error and never executed).

Open tickets recorded under section 7 rules:
- **RV8-141** (open, 2026-11-04): JIT hybrid. Supersedes the "JIT-specific infrastructure removed by design decision" clause in section 3 (see the RV8-141 note there). The runtime code emitter, region-based partial deoptimization, OSR, the AOT-to-JIT tiering transition, and the runtime-optimization catalog are binding work per design.md 3021-3528 at hybrid tiering fidelity; the AOT pipeline stays the oracle and pure-AOT output must remain reproducible with the JIT disabled. Prior JIT-referencing acceptance text elsewhere in this file predates this decision and is amended by it. Progress: the runtime code emitter's memory half is landed (rv8_runtime::jitmem: W^X pages, bump segment list, patch protocol, refcounted reclamation; in-process execution of compile_lir bytes is acceptance-tested); the emitter's instruction path is landed (rv8_backend::jit: the design 3.4 relocation resolver - PcRel32 as S+A-P and Abs64 as S+A with the exact rv8_object semantics, validated all-or-nothing before any byte is written - and JitModule::build, which compiles a LIR module through the AOT oracle into one deterministic W^X CodePool, resolves module-local call targets through the slot table and external ordinals through a caller-provided function table before finalization, and hands out per-ordinal executable entries plus patch-through-slots for the later tiers; jitmem gains the write-phase page_addr/write_slot the pre-finalize resolution needs; 25 new tests: resolver formulas and fail-closed paths as unit tests, and in-process execution of forward/backward/chained/shared/external call targets, SysV argument passing, patch-time call-site redirect, layout determinism, and executed-bytes-equal-linked-AOT-bytes in tests/jit_module.rs); deopt/OSR, tiering, and the runtime-optimization catalog remain.
- **RV8-143** (open, 2026-11-04): Magic Division (design.md 802, 1181). The PatternForm::MagicDiv rule and the "replace division by constant with multiply-by-magic-number" mandate require a 128-bit high-multiply instruction (x86-64 `imul r/m64`, REX.W F7 /5, RDX:RAX). The Machine ISA has no high-multiply opcode; landing MagicDiv therefore requires the ImulHi extension across lir.rs (opcode), machir.rs (slot class with implicit RAX input and RDX:RAX outputs), encode.rs (encoding), schedule.rs (RAX/RDX dependency edges), and isel.rs (the magic-number pattern with the round-up magic construction m = ceil(2^(64+s)/d), correctness condition e = m*d - 2^(64+s) <= 2^s, sign handling by negation for negative divisors, and DivSeq retention for d in {-1, 0, 1}).
- **RV8-150** (open, 2026-11-13): the chapter-65 hunt-receipt checks are configured-and-dormant, not implemented. dylan's CEP&CC revision (standard.md chapter 65, the Bloodhound/HUNTER protocol) documents two severity-0 lint rules — `CEP-LINT-HUNT-RECEIPT` (check `commit_hunt_manifest`) and `CEP-LINT-HUNTER-RECEIPT` (check `hunter_manifest`) — active only under a `--verify-hunt` invocation and requiring git-diff parsing plus cryptographic receipt-signature verification. DOC-SYNC's bidirectional rule-coverage contract failed at that revision's HEAD (the two documented rules had no configuration entries; the same revision also clobbered the `### Rule CEP-LINT-ATOMIC-FILE` catalog heading, so the configured 0.4 rule was undocumented), so this change aligns the contract honestly: the configuration carries both entries with their documented severities and check ids, `run_file_rules` dispatches both check ids to documented dormant arms (normal runs emit no findings, exactly the no-`--verify-hunt` behavior the chapter specifies), and the ATOMIC-FILE catalog entry is restored verbatim. The full enforcement (the `--verify-hunt` flag, receipt parsing, first-party signature verification, the Yell Protocol output) remains this ticket's work; until it lands nothing claims the hunts run.

Resolved tickets (2026-11-05 session, all enforced by tests):
- **RV8-144** (resolved): the panic-console parallel-test flake. `panic::tests::console_callback_receives_text` failed ~1 run in 10 (`["hello"]` instead of `["hello", "world"]`): `set_console` registers a process-global writer, the module's two lib tests raced their `install_sink` calls on it, and cargo runs lib tests on parallel threads — the module's "single-threaded test execution" CEP:ASSUMES was false. The fix serializes every console-using test behind a module-local `CONSOLE_MUX` (poisoning deliberately ignored so one failed assert cannot cascade); no assertion was weakened. Evidence: the twenty-test console-contract battery `crates/rv8_runtime/tests/console_contract_rv8_144.rs` (registration/replacement/removal, chunk order/boundaries/content/volume, the report transcript, payload mapping, and per-cycle muxed install/write/observe/remove stress that deterministically reproduces the original collision), plus 50 consecutive green full-crate runs after the fix versus the pre-fix flake rate.
- **RV8-145** (resolved): the FV-0 source gate false-positived on its own documentation. The CI zero-sorry step was a naive `grep -rn "sorry"` over the Lean sources, so the word "sorry" inside `RV8FV/Audit.lean`'s doc comments (lines describing the gate) failed every run since the machine audit landed — while the machine audit itself (the total `collectAxioms` sweep, 1726 constants + 87 flagship canaries against the {propext, Classical.choice, Quot.sound} whitelist) stayed green. The gate is now `tools/fv_zero_sorry.py`: it strips Lean comments (nested `/- -/`, `--`) and string literals while preserving line numbers, then flags `sorry`/`sorryAx`/`mkSorry` tokens in actual code — strictly stronger than the grep, which missed `mkSorry`-minted holes entirely. The scanner carries an 11-case self-test (`--self-test`) covering real holes, nested comments, string prose, and the Audit.lean header shape.
- **RV8-146** (resolved): CEP&CC lint 0.3 (user directive: "update the CEP&CC lint to 0.3"). The lint had silently drifted behind its own law: the config pinned `CEP&CC 0.1` while standard.md was 0.2 — the weak any-line `version_pattern` was satisfied by a stale cross-reference at standard.md 4361 — and `CEP-LINT-MACRO-PREFIX` was documented in chapter 50 but shipped unconfigured, invisible because DOC-SYNC checked only the configured-to-documented direction. standard.md is revised to **0.3** (revision-history block; no rule's detection semantics changed): DOC-SYNC is now mechanical in both coverage directions plus `Version:`-line equality against `standard.version`; MACRO-PREFIX is configured (Rust mapping: `macro_rules!` definitions obey the `CEP_` name pattern or the include-guard pattern; the one violating repo macro, `range_for_int`, is renamed `CEP_range_for_int`); `--self-test` (50.6.3) runs the shipped 38-scenario manifest with exact per-rule finding counts; `--version` prints tool and standard versions (50.1). Evidence: `cargo test -p cep_lint` (70 unit + 13 integration tests, including the shipped-agreement, reverse-direction, stale-cross-reference regression, and manifest-coverage tests), the 38-scenario self-test run, and the live gate `cargo run -p cep_lint -- crates tools tests` at S0=0, S1=0.
- **RV8-147** (resolved): CEP&CC lint 0.4 (follow-on to RV8-146's directive; the spec moved to 0.4 in dylan's chapter-62 revision while the lint stayed at 0.3, so DOC-SYNC's Version:-equality contract failed the gate at HEAD and the new chapter's rule shipped unenforced). standard.md gains the `### Rule CEP-LINT-ATOMIC-FILE` catalog entry (chapter 50.3; no detection semantics of existing rules changed). The lint implements `file_topology_policy` (62.5.3's four duties, all config-driven): per-class line limits for exactly the classes 62.5.1 names (CEP-0/1/2 at 400/800/1200, wc-style count, the generated-marker exemption scoped to a file's opening lines), the junk-drawer filename stem ban (62.4), the one-pass-registration cap under the configured `**/passes/**` topology, and the one-ISEL-category cap under the configured `**/targets/*/isel/**` topology (category = the capture of the configured tag pattern over whole lines; distinct captures count, repeats do not). HPC-*/RCS-*/CC/FFI classes carry no mechanical line limit (62.5.1 does not name them) and stay review-governed until a revision does. The line-text rule family moved from rules.rs to checks_text.rs so both modules sit inside the new CEP-2 line budget (rules.rs 1128, checks_text.rs 635; rules.rs was the one tree file over a configured limit at 1357 lines). The self-test manifest grows 38 to 47 scenarios (violating + compliant per duty, generated exemption included). Evidence: `cargo test -p cep_lint` (76 unit + 13 integration, including the 47-scenario manifest run), clippy clean, and the live gate `cargo run -p cep_lint -- crates tools tests` at S0=0, S1=0, exit 0.

All four previously open tickets were resolved in the 2026-11-03 session:

Resolved tickets (2026-11-04 session, all enforced by tests):
- **RV8-142** (resolved): the ETXTBSY end-to-end spawn flake. CI run 41703539038 observed one "spawn exec: Text file busy (os error 26)" in `le_loop_is_inclusive` even after the per-invocation unique temp path: exec refuses a file any process still holds open for writing, which a unique name alone cannot rule out under aggressive test parallelism. `pipeline::run` now retries ETXTBSY on a fresh unique path (bounded at four attempts, never silent) and removes the temporary executable after every run. Evidence: the twenty-program ETXTBSY regression battery in `crates/rv8_driver/tests/end_to_end_semantics_rv8_137.rs` (single-threaded exit-code pinning plus an eight-thread concurrent battery, 160 spawns per run, three consecutive green local runs).

Resolved tickets (2026-11-03 session, all enforced by tests):
- **RV8-133** (resolved): `rv8_core::blake3` lands the BLAKE3 core (zero external dependencies, incremental Hasher + XOF), pinned against vectors generated with the official implementation; `rv8_driver::cache::content_id` now hashes through it (convention 10's one-file swap held).
- **RV8-134** (resolved): the demand-driven memoized query system lives in `rv8_driver::query` (engine: dependency-recording at read time, transitive input-dependency invalidation, cycle detection with structured paths, error caching, named budgets MAX_QUERY_DEPTH/SLOTS/INPUTS) and `rv8_driver::queries` (FunctionCountQuery, FunctionObjectQuery, ProgramQuery). `pipeline::compile` demands ProgramQuery; re-demands on unchanged source are memo hits (stats observable).
- **RV8-135** (resolved): `rv8_ir::diag_text` implements Display for HirError, NodeError, LowerError, HetError, MirError; every variant's text is pinned by tests and the driver formats through Display.
- **RV8-140** (resolved): `rv8_backend::regalloc::warm` adds the linear-scan warm start (design 7.4) and the incremental/region re-solve (design 7.2/7.3) over the ONE shared timeline network (the network constructor was factored out of solve.rs). Documented physical limit: a residual cycle cannot reassign a chord between two register timelines, so both accelerator paths run the optimality cleanse, the assignment repair, and a never-regress promotion check against the exact cold solve; the corpora pin cost equality (warm vs brute force on all 24 seeds, incremental identity on unchanged input).
- **Stub completions** (same session): rv8_object Mach-O/PE-COFF serializers and DWARF emission + ELF hot/cold split (deterministic, strictly validating, 36+ new tests); rv8_frontend decision-tree exhaustiveness/usefulness engine with budgeted iterative traversal (25 regression tests, 300k-deep input rejected without stack overflow); rv8_runtime no_std foundation (traits, Option/Result, Vec/RawVec/Box, iterators, u128 division, soft-float bit-exact vs native, memcpy/memset/memcmp, bump allocator, panic reporting; 60 tests); rv8_backend ABI stack-clash probing remains a documented stub (flag exists, no probe code emitted) - it is the only component still carrying STATUS stub.

Earlier resolved tickets:
- **RV8-137 follow-up** (2026-10-31, second session): five defect classes remained after the first fix - call results produced no values (call-in-loop programs failed to compile); block layout was id-order, not an execution order (if-inside-while failed or hung); branch arms were read from block numbering (loops exited immediately under renumbering); the phi-copy sequencer emitted the destroying copy first and owned one scratch for all cycles (fibonacci and double-swap miscompiled); spill scratches ignored liveness (call-in-loop hung). Also repaired: the regalloc test module was orphaned (never compiled, syntax error hidden), and parallel end-to-end tests raced on one temp executable. Fixes ported on top of commits 81c5224/918a500 with the new `StoreStackImm` opcode for register-free immediate spills. Evidence: `docs/postmortems/rv8_137_followup.md`, the behavioral suites (`end_to_end_semantics_rv8_137.rs`, 26 tests; `value_plumbing_rv8_137.rs`, 22 tests), and the Z3 proofs in `tools/z3_verify/`. Workspace: 484 tests at the time of writing; 740+ after the 2026-11-03 session (BLAKE3 conformance, query engine battery, warm/incremental corpora, object serialization, pattern exhaustiveness, runtime foundation), zero failures, all gates green.
- **RV8-137** (resolved 2026-10-31): the loop end-to-end test `pipeline_runs_loop_program` is un-ignored and green. Root-cause chain, fixed in one pass: (1) shared MCF job pairs let a flow unit enter a chord on one register and exit on another, double-booking lanes while entry-arc attribution read only the entry side; (2) linear intervals missed back-edge wrap liveness for BOTH multi-def (phi) and single-def (loop constant) values; (3) `hetir_to_lir` dropped the Return value input, so results reached RAX only through allocation luck; (4) the ret pin overwrote parameter pins; (5) the expression flattener delivered Binary operands and Call arguments in reverse (every non-commutative operator and call was miscompiled; the fixture passed only because addition commutes); (6) the priced chords let idle timelines multi-book popular ranges and displace fresh ones (the assignment repair now restores assignment semantics on the solved flow). Evidence: the RV8-137 regression batch (twenty tests, crates/rv8_backend/tests/rv8_137_regression.rs), the operand-order batch (twenty tests, crates/rv8_frontend/tests/operand_order_regression.rs), the return-threading batch (twenty tests, crates/rv8_ir/tests/ret_value_regression.rs), and the two brute-force corpora (single-def, 24 seeds; multi-def, 12 seeds).
