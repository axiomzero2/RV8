# MCF Register Allocation

## Abstract

The register allocator assigns each live range of a function either one physical register lane of its register class or a spill slot. It formulates the assignment per register class as a min-cost multi-commodity flow over a machine-timeline network and solves it with successive shortest paths under Johnson potentials, followed by a deterministic assignment repair. Inputs are a LirFunction and a target Plan derived from an ISL target description; the output is a RegisterAllocation consumed by the spill rewriter.

## Formal Definitions

A live range is `LiveRange { value: ValueId, def: u32, last_use: u32, class: RegClassId, pin: Option<PhysReg> }` over linearized program points: point 0 is function entry and the instruction at global ordinal g is point g+1. The conservative wrap-sound interval rule (convention pin 3) assigns multi-def values and values live-out of a block with a layout-earlier successor the interval `[def, last_point]`; all other values take `[def, last_use]`.

A `Plan { costs: CostTable, classes: [ClassPlan; 2], abi_return: PhysReg, abi_args: Vec<PhysReg> }` is the allocator-facing target. `ClassPlan` holds the lanes in RegId order with per-lane callee-saved and divide-clobber flags. `CostTable` holds the integer-grid move, spill, save/restore, and pinned-spill costs (one published cost unit equals `COST_SCALE` network points).

The flow network per class has one capacity-1 timeline chain per register lane (`SRC -> C(r,0) -> ... -> C(r,last) -> SNK`, idle arcs cost 0) and one lane-private job pair per (range, register): entry arc `C(r, def_i) -> J_in(i,r)`, priced through-edge `J_in -> J_out` with cost `-SPILL_PRICE(i)`, exit arc `J_out -> C(r, use_i)` with cost `c(i,r)`.

## Algorithm

1. build_ranges derives points, intervals, and ABI pins; a block-level backward liveness fixpoint (bounded by `MAX_LIVENESS_ROUNDS`) marks wrap-live values.
2. build_barriers marks per-point clobber masks: calls block the class's caller-saved lanes and the argument-move window; cqo/idiv block the divide-clobber lanes.
3. solve_class builds the network, omits job pairs that cross a barrier or violate a pin, initializes potentials with one Bellman-Ford pass (the initial network is acyclic), and performs R augmentations, updating `pot[v] += min(dist[v], dist[snk])` after each. A Bellman-Ford negative-cycle cleanse guards the SSP invariant.
4. repair_assignment restores assignment semantics on the solved flow: each range keeps its cheapest flowing chord (ties to the lowest lane), surplus chords are canceled onto their own idle chains, and untaken ranges are filled in (def, value) order through the cheapest lane whose unit idles across the whole window.
5. Attribution reads the flowing entry arc of each (range, register) pair.

Time: one reduced-cost Dijkstra per register per class solve, `O(R * E log N)`; Bellman-Ford passes are `O(N * E)` under `MAX_MCF_BF_STEPS`. Space is `O(N + E)` under `MAX_MCF_NODES` and `MAX_MCF_EDGES`.

## Invariants

1. Any integral flow of value R decomposes into R lane-private register timelines whose chords are pairwise time-disjoint per lane; the attributed assignment is therefore feasible by construction.
2. Feasibility is absolute: every timeline can idle, so an R-unit flow always exists and spilling is always available.
3. Every flow mutation reroutes exactly one lane-private unit, preserving per-lane disjointness through the repair.
4. Determinism: Dijkstra ties pop the lowest (distance, node) pair; the repair scans lanes by (cost, index) and ranges by (def, value); allocation is byte-identical across runs.
5. ABI pre-coloring holds: parameters pin to the plan's argument lanes in value-id order; a ret operand pins to the return lane unless an argument pin already holds.

## Integration Points

Depends on: `rv8_isl` (TargetDescription), `rv8_ir::lir` (LirFunction, RegisterAllocation), `rv8_backend::abi` (physical encodings for the x86-64 rewriter), `rv8_core::rng` (test corpora).

Depended on by: `rv8_backend::lib::compile_lir` (select -> allocate -> rewrite -> schedule -> verify -> encode), `rv8_driver::pipeline`.

Signatures: `pub fn allocate(lir: &LirFunction) -> Result<RegisterAllocation, BackendError>`; `pub fn allocate_with(plan: &Plan, lir: &LirFunction) -> Result<RegisterAllocation, BackendError>`; `pub fn rewrite(mir: &mut MachineIrFunction, lir: &LirFunction, alloc: &RegisterAllocation) -> Result<FramePlan, BackendError>`.

## Known Limitations

The wrap-sound rule reserves a wrapped value's lane to the end of the layout; interval splitting is the precise replacement. Call results carry no virtual registers, so consuming one in a live compute is rejected at the LIR boundary. The rewriter is the x86-64 emission stage; non-x86 targets consume `allocate_with` directly. The encoded flow objective is degenerate under surplus takes (the repair restores assignment semantics; the brute-force corpora are the exactness arbiters).
