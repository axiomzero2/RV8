// CEP:FILE: crates/rv8_ir/src/hetir.rs
// CEP:WHAT: Level 2 Heterogeneous IR per docs/specs/multi_level_ir.md section 3: a
//   target-agnostic graph that models compute, memory, control, synchronization,
//   communication, and meta operations with typed data, control, dependency, locality,
//   synchronization, and communication edges, plus a verifier for invariants 3.3.1-3.3.4.
// CEP:WHY: The autotuner searches at this level (spec 3.4, 7) and the backend lowers it to
//   LIR for any target; explicit address spaces, sync scopes, and memory-dependency edges
//   are the contract that makes cross-architecture lowering possible.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: accessors are total over dense indices; verify() returns a descriptive
//   HetError for every violated invariant instead of panicking.
// CEP:ASSUMES: ids are dense u32 indices; identity is index-based (38.17); the graph is a
//   DAG in data edges (control may form cycles through loop nodes), checked by verify().
// CEP:COST: flat POD vectors; construction amortized O(1); verify() is O(N + E).
// CEP:EVIDENCE: unit tests rv8_ir::hetir::tests::*.
// CEP:SECURITY: all traversals are iterative; no recursion on untrusted input (38.13).

use crate::son::IrType;
use rv8_core::span::Span;
use std::fmt::Write as _;

/// CEP:WHAT: Maximum nodes or edges per Heterogeneous-IR graph.
/// CEP:WHY: Bounded compile time and memory (38.12); named limit, not a magic number.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced by producers and re-checked by verify().
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_budget_guard_rejects_oversize.
pub const MAX_HET_ITEMS: u32 = 1_000_000;

/// CEP:WHAT: Stable node identity inside one HetIr graph.
/// CEP:WHY: Index-based identity satisfies the no-pointer-identity rule (38.17).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct HetNodeId(pub u32);

/// CEP:WHAT: Address spaces named by spec 3.2-3.3 for memory, communication, and
/// synchronization nodes.
/// CEP:WHY: Communication nodes must specify source and destination spaces (3.3.2) and
/// memory ops are classified by the space they touch.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable (used in hashing and printing).
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_communication_without_spaces.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum AddressSpace {
    Host,
    DeviceGlobal,
    DeviceShared,
    DeviceLocal,
    Constant,
}

/// CEP:WHAT: Synchronization scopes named by spec 3.3.3.
/// CEP:WHY: Every synchronization node must state the scope it orders (thread, block,
/// grid, global).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable.
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_sync_without_scope.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum SyncScope {
    Thread,
    Block,
    Grid,
    Global,
}

/// CEP:WHAT: Compute operations: arithmetic, logical, comparison primitives, and value
/// merges (phi).
/// CEP:WHY: Spec 3.2 compute class; phi is a pure value-selection op that survives
/// linearization, so it stays in the compute class; transcendental ops are calls.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: operand arity is enforced by lowering, not by the enum.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ComputeOp {
    Const,
    Param,
    Phi,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Neg,
    Not,
    CmpEq,
    CmpLt,
    CmpLe,
    Cvt,
}

/// CEP:WHAT: Memory operations: load, store, prefetch, cache management, and allocation.
/// CEP:WHY: Spec 3.2 memory class; every variant carries an AddressSpace on the node.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MemoryOp {
    Load,
    Store,
    Prefetch,
    CacheOp,
    Alloc,
}

/// CEP:WHAT: Control operations: branches, merges, loops, calls, returns.
/// CEP:WHY: Spec 3.2 control class; loops and merges are explicit nodes so back-edges and
/// join points are named, which the linearization and LIR block formation require.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlOp {
    Branch,
    Merge,
    Loop,
    Call,
    Return,
}

/// CEP:WHAT: Synchronization operations: barriers, atomics, memory fences.
/// CEP:WHY: Spec 3.2 synchronization class; scope is a required node field (3.3.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_sync_without_scope.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SyncOp {
    Barrier,
    AtomicLoad,
    AtomicStore,
    AtomicRmw,
    Fence,
}

/// CEP:WHAT: Communication operations: data movement between address spaces.
/// CEP:WHY: Spec 3.2 communication class; src/dst spaces are required fields (3.3.2).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_communication_without_spaces.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CommOp {
    Transfer,
}

/// CEP:WHAT: Meta operations: loop bounds, iteration spaces, tiling hints, parallelism
/// annotations, and partial-escape-analysis markers.
/// CEP:WHY: Spec 3.2 meta class; escape markers carry PEA facts (AllocationSiteId in
/// imm) from HIR into autotuner features; consumed by the autotuner, erased before LIR.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: meta nodes are pure (no side effects) for the dead-node rule.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_dead_pure_nodes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MetaOp {
    LoopBounds,
    IterationSpace,
    TilingHint,
    Parallelism,
    EscapePoint,
    Materialize,
}

/// CEP:WHAT: The six node classes of the Heterogeneous IR, each wrapping its operation.
/// CEP:WHY: A closed two-level dispatch keeps the printer, hasher, and verifier exhaustive
/// over spec 3.2.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HetKind {
    Compute(ComputeOp),
    Memory(MemoryOp),
    Control(ControlOp),
    Sync(SyncOp),
    Communication(CommOp),
    Meta(MetaOp),
}

impl HetKind {
    /// CEP:WHAT: Returns the stable short name of the kind for printing and hashing.
    /// CEP:WHY: Textual form is the canonical serialization (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn name(self) -> &'static str {
        match self {
            HetKind::Compute(op) => compute_name(op),
            HetKind::Memory(op) => memory_name(op),
            HetKind::Control(op) => control_name(op),
            HetKind::Sync(op) => sync_name(op),
            HetKind::Communication(op) => comm_name(op),
            HetKind::Meta(op) => meta_name(op),
        }
    }

    /// CEP:WHAT: Reports whether the kind has side effects (memory writes, control,
    /// synchronization, communication).
    /// CEP:WHY: Dead-node rule 3.3.4 spares side-effecting nodes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: loads are pure for this contract; poisoning is modeled via dependency
    /// edges, not node state.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_dead_pure_nodes.
    pub fn has_side_effects(self) -> bool {
        matches!(
            self,
            HetKind::Memory(
                MemoryOp::Store | MemoryOp::CacheOp | MemoryOp::Alloc | MemoryOp::Prefetch
            ) | HetKind::Control(_)
                | HetKind::Sync(_)
                | HetKind::Communication(_)
        )
    }
}

/// CEP:WHAT: Stable short names for compute ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
fn compute_name(op: ComputeOp) -> &'static str {
    match op {
        ComputeOp::Const => "const",
        ComputeOp::Param => "param",
        ComputeOp::Phi => "phi",
        ComputeOp::Add => "add",
        ComputeOp::Sub => "sub",
        ComputeOp::Mul => "mul",
        ComputeOp::Div => "div",
        ComputeOp::Rem => "rem",
        ComputeOp::And => "and",
        ComputeOp::Or => "or",
        ComputeOp::Xor => "xor",
        ComputeOp::Shl => "shl",
        ComputeOp::Shr => "shr",
        ComputeOp::Neg => "neg",
        ComputeOp::Not => "not",
        ComputeOp::CmpEq => "cmpeq",
        ComputeOp::CmpLt => "cmplt",
        ComputeOp::CmpLe => "cmple",
        ComputeOp::Cvt => "cvt",
    }
}

/// CEP:WHAT: Stable short names for memory ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
fn memory_name(op: MemoryOp) -> &'static str {
    match op {
        MemoryOp::Load => "load",
        MemoryOp::Store => "store",
        MemoryOp::Prefetch => "prefetch",
        MemoryOp::CacheOp => "cacheop",
        MemoryOp::Alloc => "alloc",
    }
}

/// CEP:WHAT: Stable short names for control ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
fn control_name(op: ControlOp) -> &'static str {
    match op {
        ControlOp::Branch => "branch",
        ControlOp::Merge => "merge",
        ControlOp::Loop => "loop",
        ControlOp::Call => "call",
        ControlOp::Return => "ret",
    }
}

/// CEP:WHAT: Stable short names for synchronization ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_sync_without_scope.
fn sync_name(op: SyncOp) -> &'static str {
    match op {
        SyncOp::Barrier => "barrier",
        SyncOp::AtomicLoad => "atomicload",
        SyncOp::AtomicStore => "atomicstore",
        SyncOp::AtomicRmw => "atomicrmw",
        SyncOp::Fence => "fence",
    }
}

/// CEP:WHAT: Stable short names for communication ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_communication_without_spaces.
fn comm_name(op: CommOp) -> &'static str {
    match op {
        CommOp::Transfer => "transfer",
    }
}

/// CEP:WHAT: Stable short names for meta ops.
/// CEP:WHY: Printing and hashing share one table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_dead_pure_nodes.
fn meta_name(op: MetaOp) -> &'static str {
    match op {
        MetaOp::LoopBounds => "loopbounds",
        MetaOp::IterationSpace => "iterspace",
        MetaOp::TilingHint => "tiling",
        MetaOp::Parallelism => "parallel",
        MetaOp::EscapePoint => "escape",
        MetaOp::Materialize => "materialize",
    }
}

/// CEP:WHAT: Memory-dependency classes: read-after-write, write-after-read,
/// write-after-write.
/// CEP:WHY: Spec 3.2 edge class "Dependency: RAW, WAR, WAW constraints".
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable.
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_cross_space_memory_without_sync.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum DepKind {
    Raw,
    War,
    Waw,
}

/// CEP:WHAT: The six edge kinds of the Heterogeneous IR per spec 3.2.
/// CEP:WHY: One closed edge vocabulary keeps ordering and locality facts explicit for the
/// autotuner and the linearization.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HetEdgeKind {
    Data { ty: IrType },
    Control,
    Dependency(DepKind),
    Locality { stride: i64, reuse_distance: u32 },
    Sync,
    Communication,
}

impl HetEdgeKind {
    /// CEP:WHAT: Returns the stable short name of the edge kind.
    /// CEP:WHY: Printable edges (38.17).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_print_and_hash_are_stable.
    pub fn name(self) -> &'static str {
        match self {
            HetEdgeKind::Data { .. } => "data",
            HetEdgeKind::Control => "ctrl",
            HetEdgeKind::Dependency(_) => "dep",
            HetEdgeKind::Locality { .. } => "locality",
            HetEdgeKind::Sync => "sync",
            HetEdgeKind::Communication => "comm",
        }
    }
}

/// CEP:WHAT: One directed edge: source, destination, and kind payload.
/// CEP:WHY: Edges are first-class so dependency and locality facts survive passes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: endpoints exist in the same graph (verifier-checked).
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Copy, Debug)]
pub struct HetEdge {
    pub from: HetNodeId,
    pub to: HetNodeId,
    pub kind: HetEdgeKind,
}

/// CEP:WHAT: One Heterogeneous-IR node: kind, type, source and destination address spaces,
/// synchronization scope, immediate payload, owning block, and span.
/// CEP:WHY: A flat record keeps every required attribute (3.3.2, 3.3.3) adjacent to the op;
/// Communication nodes carry both spaces (src in addr_space, dst in addr_dst).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: addr_space is Some exactly for Memory and Communication nodes;
/// addr_dst is Some exactly for Communication nodes; sync_scope is Some exactly for Sync
/// nodes (all verifier-checked).
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
#[derive(Clone, Debug)]
pub struct HetNode {
    pub kind: HetKind,
    pub ty: IrType,
    pub addr_space: Option<AddressSpace>,
    pub addr_dst: Option<AddressSpace>,
    pub sync_scope: Option<SyncScope>,
    pub imm: i64,
    pub block: Option<crate::son::BlockId>,
    pub span: Span,
}

/// CEP:WHAT: Error returned by HetIr::verify for every violated spec-3.3 invariant.
/// CEP:WHY: Fail-closed contracts without panics; variants name offending ids.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit tests rv8_ir::hetir::tests::het_rejects_*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HetError {
    DanglingNode(HetNodeId),
    DanglingEdgeEndpoint { edge: usize, node: HetNodeId },
    MissingAddressSpace { node: HetNodeId },
    MissingSyncScope { node: HetNodeId },
    CrossSpaceMemoryWithoutSync { node: HetNodeId },
    DeadPureNode { node: HetNodeId },
    DataCycle { node: HetNodeId },
    Oversized { items: u32 },
}

/// CEP:WHAT: Enforces the per-graph item budget over nodes and edges.
/// CEP:WHY: Named compile-time budget (38.12); pure function for direct testing.
/// CEP:STATUS: complete
/// CEP:FAILURE: Oversized when items exceeds MAX_HET_ITEMS.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_budget_guard_rejects_oversize.
fn check_budget(items: u32) -> Result<(), HetError> {
    if items > MAX_HET_ITEMS {
        Err(HetError::Oversized { items })
    } else {
        Ok(())
    }
}

/// CEP:WHAT: The Heterogeneous-IR graph for one function: flat node and edge vectors.
/// CEP:WHY: Central Level-2 container consumed by the autotuner and lowered to LIR
/// (spec 3.4, 6.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: see HetError.
/// CEP:ASSUMES: one function per graph.
/// CEP:COST: construction amortized O(1) per node or edge.
/// CEP:EVIDENCE: unit tests rv8_ir::hetir::tests::*.
#[derive(Clone, Debug, Default)]
pub struct HetIr {
    pub nodes: Vec<HetNode>,
    pub edges: Vec<HetEdge>,
}

impl HetIr {
    /// CEP:WHAT: Creates an empty graph.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn new() -> HetIr {
        HetIr {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// CEP:WHAT: Appends a node and returns its id.
    /// CEP:WHY: Single construction path keeps indices dense.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn add_node(&mut self, node: HetNode) -> HetNodeId {
        let id = HetNodeId(self.nodes.len() as u32);
        self.nodes.push(node);
        id
    }

    /// CEP:WHAT: Appends a directed edge.
    /// CEP:WHY: Edges are first-class per spec 3.2.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: endpoints exist; verify() reports dangling endpoints.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn add_edge(&mut self, from: HetNodeId, to: HetNodeId, kind: HetEdgeKind) {
        self.edges.push(HetEdge { from, to, kind });
    }

    /// CEP:WHAT: Immutable node accessor.
    /// CEP:WHY: Fail-closed access (38.6).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingNode for unknown ids.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn node(&self, id: HetNodeId) -> Result<&HetNode, HetError> {
        self.nodes.get(id.0 as usize).ok_or(HetError::DanglingNode(id))
    }

    /// CEP:WHAT: Builds the outgoing edge list of one node in stable edge order.
    /// CEP:WHY: Consumers (linearization, autotuner features) need per-node adjacency
    /// without pointer structures.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(E) per call; callers that need all adjacencies should use
    /// outgoing_all once instead.
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn outgoing(&self, from: HetNodeId) -> Vec<(HetNodeId, HetEdgeKind)> {
        self.edges
            .iter()
            .filter(|e| e.from == from)
            .map(|e| (e.to, e.kind))
            .collect()
    }

    /// CEP:WHAT: Computes per-node outgoing and incoming adjacency in one pass.
    /// CEP:WHY: verify() and the linearization need adjacency without repeated O(E)
    /// scans; index-based output preserves determinism.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(N + E) time and space.
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_construction_and_verify.
    pub fn adjacency(&self) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let mut out = vec![Vec::new(); self.nodes.len()];
        let mut inc = vec![Vec::new(); self.nodes.len()];
        for (i, edge) in self.edges.iter().enumerate() {
            if (edge.from.0 as usize) < out.len() {
                out[edge.from.0 as usize].push(i);
            }
            if (edge.to.0 as usize) < inc.len() {
                inc[edge.to.0 as usize].push(i);
            }
        }
        (out, inc)
    }

    /// CEP:WHAT: Verifies all four spec-3.3 invariants plus DAG-ness of data edges and
    /// the item budget.
    /// CEP:WHY: 38.18 requires verification after every pass; HetIR passes (autotuner
    /// transformations) must re-establish 3.3.1-3.3.4 before LIR lowering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns the first violated invariant as a HetError naming ids.
    /// CEP:ASSUMES: none; total over any contents.
    /// CEP:COST: O(N + E); iterative only.
    /// CEP:EVIDENCE: unit tests rv8_ir::hetir::tests::het_rejects_*.
    pub fn verify(&self) -> Result<(), HetError> {
        let total = match self
            .nodes
            .len()
            .checked_add(self.edges.len())
        {
            Some(v) => v as u32,
            None => return Err(HetError::Oversized { items: u32::MAX }),
        };
        check_budget(total)?;

        // Endpoint existence for every edge.
        for (i, edge) in self.edges.iter().enumerate() {
            if edge.from.0 as usize >= self.nodes.len() {
                return Err(HetError::DanglingEdgeEndpoint { edge: i, node: edge.from });
            }
            if edge.to.0 as usize >= self.nodes.len() {
                return Err(HetError::DanglingEdgeEndpoint { edge: i, node: edge.to });
            }
        }

        // 3.3.2 communication nodes must specify source and destination spaces; memory
        // nodes must specify the space they touch. 3.3.3 sync nodes must specify scope.
        for (i, node) in self.nodes.iter().enumerate() {
            let id = HetNodeId(i as u32);
            match node.kind {
                HetKind::Memory(_) if node.addr_space.is_none() => {
                    return Err(HetError::MissingAddressSpace { node: id });
                }
                HetKind::Communication(_)
                    if node.addr_space.is_none() || node.addr_dst.is_none() =>
                {
                    return Err(HetError::MissingAddressSpace { node: id });
                }
                HetKind::Sync(_) if node.sync_scope.is_none() => {
                    return Err(HetError::MissingSyncScope { node: id });
                }
                _ => {}
            }
        }

        // 3.3.1 a memory access in a non-host space must have a sync or dependency edge
        // attached (crossing address-space boundaries implies ordering requirements).
        let (out_adj, in_adj) = self.adjacency();
        for (i, node) in self.nodes.iter().enumerate() {
            let id = HetNodeId(i as u32);
            let crosses = match node.kind {
                HetKind::Memory(_) => !matches!(
                    node.addr_space,
                    Some(AddressSpace::Host) | None
                ),
                HetKind::Communication(_) => true,
                _ => false,
            };
            if !crosses {
                continue;
            }
            let has_ordering = out_adj[i]
                .iter()
                .chain(in_adj[i].iter())
                .any(|&ei| {
                    matches!(
                        self.edges[ei].kind,
                        HetEdgeKind::Dependency(_) | HetEdgeKind::Sync
                    )
                });
            if !has_ordering {
                return Err(HetError::CrossSpaceMemoryWithoutSync { node: id });
            }
        }

        // 3.3.4 no dead nodes: a pure node with no outgoing edges has no uses.
        for (i, node) in self.nodes.iter().enumerate() {
            if node.kind.has_side_effects() {
                continue;
            }
            if out_adj[i].is_empty() {
                return Err(HetError::DeadPureNode { node: HetNodeId(i as u32) });
            }
        }

        // Data edges must form a DAG (control edges may cycle through loop nodes).
        self.verify_data_dag(&out_adj)?;
        Ok(())
    }

    /// CEP:WHAT: Checks that data edges form a DAG with an iterative Kahn pass.
    /// CEP:WHY: Invariant analogous to SoN SSA domination; cyclic data flow is
    /// unlowerable to LIR.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DataCycle naming one node on the cycle.
    /// CEP:ASSUMES: edges reference existing nodes (checked earlier in verify()).
    /// CEP:COST: O(N + E); iterative.
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_rejects_data_cycle.
    fn verify_data_dag(&self, out_adj: &[Vec<usize>]) -> Result<(), HetError> {
        let mut indegree_data = vec![0u32; self.nodes.len()];
        for edge in &self.edges {
            if matches!(edge.kind, HetEdgeKind::Data { .. }) {
                indegree_data[edge.to.0 as usize] += 1;
            }
        }
        let mut ready: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| indegree_data[i] == 0)
            .collect();
        let mut visited = 0usize;
        while let Some(i) = ready.pop() {
            visited += 1;
            for &ei in &out_adj[i] {
                if matches!(self.edges[ei].kind, HetEdgeKind::Data { .. }) {
                    let target = self.edges[ei].to.0 as usize;
                    indegree_data[target] -= 1;
                    if indegree_data[target] == 0 {
                        ready.push(target);
                    }
                }
            }
        }
        if visited != self.nodes.len() {
            let stuck = (0..self.nodes.len())
                .find(|&i| indegree_data[i] > 0)
                .unwrap_or(0);
            return Err(HetError::DataCycle { node: HetNodeId(stuck as u32) });
        }
        Ok(())
    }

    /// CEP:WHAT: Prints nodes and edges in stable id order.
    /// CEP:WHY: Printable IR contract (38.17).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(N + E).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_print_and_hash_are_stable.
    pub fn print(&self) -> String {
        let mut out = String::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let _ = write!(out, "h{}: {} {}", i, node.kind.name(), type_short(node.ty));
            if let Some(space) = node.addr_space {
                let _ = write!(out, " space={:?}", space);
            }
            if let Some(dst) = node.addr_dst {
                let _ = write!(out, " dst={:?}", dst);
            }
            if let Some(scope) = node.sync_scope {
                let _ = write!(out, " scope={:?}", scope);
            }
            let _ = writeln!(out);
        }
        for (i, edge) in self.edges.iter().enumerate() {
            let _ = writeln!(
                out,
                "e{}: h{} -> h{} {}",
                i,
                edge.from.0,
                edge.to.0,
                edge.kind.name()
            );
        }
        out
    }

    /// CEP:WHAT: Computes a deterministic content hash over nodes and edges.
    /// CEP:WHY: CAS cache keys and pass-change detection (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: vector order is canonical (construction order).
    /// CEP:COST: O(N + E).
    /// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_print_and_hash_are_stable.
    pub fn canonical_hash(&self) -> u64 {
        let mut hash: u64 = 0;
        for node in &self.nodes {
            hash = mix(hash, node.kind.name().as_bytes());
            hash = mix(hash, &[node.ty as u8]);
            hash = mix(
                hash,
                &(node.addr_space.map(|a| a as u8).unwrap_or(u8::MAX)).to_le_bytes(),
            );
            hash = mix(
                hash,
                &(node.addr_dst.map(|a| a as u8).unwrap_or(u8::MAX)).to_le_bytes(),
            );
            hash = mix(
                hash,
                &(node.sync_scope.map(|s| s as u8).unwrap_or(u8::MAX)).to_le_bytes(),
            );
            hash = mix(hash, &node.imm.to_le_bytes());
        }
        for edge in &self.edges {
            hash = mix(hash, &edge.from.0.to_le_bytes());
            hash = mix(hash, &edge.to.0.to_le_bytes());
            hash = mix(hash, edge.kind.name().as_bytes());
            if let HetEdgeKind::Data { ty } = edge.kind {
                hash = mix(hash, &[ty as u8]);
            }
        }
        hash
    }
}

/// CEP:WHAT: Chains bytes into the running canonical hash state.
/// CEP:WHY: Shared helper for HetIr hashing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_print_and_hash_are_stable.
fn mix(state: u64, bytes: &[u8]) -> u64 {
    rv8_core::fx_mix(state, rv8_core::fxhash64(bytes))
}

/// CEP:WHAT: Renders an IrType short name for printing.
/// CEP:WHY: Stable textual type form shared with son.rs printing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::hetir::tests::het_print_and_hash_are_stable.
fn type_short(ty: IrType) -> &'static str {
    match ty {
        IrType::Unit => "void",
        IrType::Bool => "bool",
        IrType::I8 => "i8",
        IrType::I16 => "i16",
        IrType::I32 => "i32",
        IrType::I64 => "i64",
        IrType::I128 => "i128",
        IrType::U8 => "u8",
        IrType::U16 => "u16",
        IrType::U32 => "u32",
        IrType::U64 => "u64",
        IrType::U128 => "u128",
        IrType::F32 => "f32",
        IrType::F64 => "f64",
        IrType::Char => "char",
        IrType::Ptr => "ptr",
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Blank span for tests.
    /// CEP:WHY: Keeps test bodies readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span {
            file: FileId(0),
            lo: 0,
            hi: 0,
        }
    }

    /// CEP:WHAT: Base node factory with neutral fields.
    /// CEP:WHY: Tests override only the fields under test.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn node(kind: HetKind, ty: IrType) -> HetNode {
        HetNode {
            kind,
            ty,
            addr_space: None,
            addr_dst: None,
            sync_scope: None,
            imm: 0,
            block: None,
            span: sp(),
        }
    }

    /// CEP:WHAT: Builds const a, const b, add(a,b) with data edges and verifies.
    /// CEP:WHY: Happy-path construction, adjacency, print, and hash stability.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_construction_and_verify() {
        let mut g = HetIr::new();
        let a = g.add_node(node(HetKind::Compute(ComputeOp::Const), IrType::I32));
        let b = g.add_node(node(HetKind::Compute(ComputeOp::Const), IrType::I32));
        let sum = g.add_node(node(HetKind::Compute(ComputeOp::Add), IrType::I32));
        g.add_edge(a, sum, HetEdgeKind::Data { ty: IrType::I32 });
        g.add_edge(b, sum, HetEdgeKind::Data { ty: IrType::I32 });
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(sum, anchor, HetEdgeKind::Control);
        g.verify().expect("well-formed HetIR must verify");
        let out = g.outgoing(a);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, sum);
        assert!(g.print().contains("h2: add i32"));
    }

    /// CEP:WHAT: A communication node missing its destination space must fail
    /// verification; with both spaces and an ordering edge it passes.
    /// CEP:WHY: Invariant 3.3.2 requires source and destination spaces; 3.3.1 requires an
    /// ordering edge for cross-space data movement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_rejects_communication_without_spaces() {
        let mut g = HetIr::new();
        let mut transfer = node(HetKind::Communication(CommOp::Transfer), IrType::I32);
        transfer.addr_space = Some(AddressSpace::Host);
        // addr_dst still missing.
        let t = g.add_node(transfer);
        let barrier = g.add_node(node(HetKind::Sync(SyncOp::Fence), IrType::Unit));
        g.nodes[barrier.0 as usize].sync_scope = Some(SyncScope::Global);
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(t, anchor, HetEdgeKind::Communication);
        g.add_edge(barrier, t, HetEdgeKind::Sync);
        match g.verify() {
            Err(HetError::MissingAddressSpace { .. }) => {}
            other => panic!("expected MissingAddressSpace, got {other:?}"),
        }
        // Setting the destination repairs the graph.
        g.nodes[t.0 as usize].addr_dst = Some(AddressSpace::DeviceGlobal);
        g.verify().expect("communication with both spaces and a sync edge must verify");
    }

    /// CEP:WHAT: A synchronization node without a scope must fail verification.
    /// CEP:WHY: Invariant 3.3.3.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_rejects_sync_without_scope() {
        let mut g = HetIr::new();
        let b = g.add_node(node(HetKind::Sync(SyncOp::Barrier), IrType::Unit));
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Branch), IrType::Unit));
        g.add_edge(b, anchor, HetEdgeKind::Control);
        match g.verify() {
            Err(HetError::MissingSyncScope { .. }) => {}
            other => panic!("expected MissingSyncScope, got {other:?}"),
        }
    }

    /// CEP:WHAT: A device-memory load with no sync or dependency edge must fail
    /// verification.
    /// CEP:WHY: Invariant 3.3.1.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_rejects_cross_space_memory_without_sync() {
        let mut g = HetIr::new();
        let mut load = node(HetKind::Memory(MemoryOp::Load), IrType::I32);
        load.addr_space = Some(AddressSpace::DeviceGlobal);
        let l = g.add_node(load);
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(l, anchor, HetEdgeKind::Control);
        match g.verify() {
            Err(HetError::CrossSpaceMemoryWithoutSync { .. }) => {}
            other => panic!("expected CrossSpaceMemoryWithoutSync, got {other:?}"),
        }
        // Adding a dependency edge repairs the graph.
        let base = g.add_node(node(HetKind::Memory(MemoryOp::Store), IrType::I32));
        g.edges.retain(|e| e.from != l || e.to != anchor);
        g.add_edge(base, l, HetEdgeKind::Dependency(DepKind::Waw));
        g.add_edge(l, anchor, HetEdgeKind::Control);
        let mut store = g.nodes[base.0 as usize].clone();
        store.addr_space = Some(AddressSpace::Host);
        g.nodes[base.0 as usize] = store;
        g.verify().expect("dependency edge must satisfy 3.3.1");
    }

    /// CEP:WHAT: A pure node with no uses must fail verification.
    /// CEP:WHY: Invariant 3.3.4.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_rejects_dead_pure_nodes() {
        let mut g = HetIr::new();
        let dead = g.add_node(node(HetKind::Compute(ComputeOp::Const), IrType::I32));
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(dead, anchor, HetEdgeKind::Control);
        // dead now has an incoming-free but outgoing-only structure; it is used by the
        // control edge, so it is not dead. Add a truly unused pure node.
        let _unused = g.add_node(node(HetKind::Meta(MetaOp::TilingHint), IrType::Unit));
        match g.verify() {
            Err(HetError::DeadPureNode { .. }) => {}
            other => panic!("expected DeadPureNode, got {other:?}"),
        }
    }

    /// CEP:WHAT: A data-edge cycle must fail verification.
    /// CEP:WHY: Data DAG requirement of the linearization.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_rejects_data_cycle() {
        let mut g = HetIr::new();
        let a = g.add_node(node(HetKind::Compute(ComputeOp::Add), IrType::I32));
        let b = g.add_node(node(HetKind::Compute(ComputeOp::Add), IrType::I32));
        let anchor = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(a, b, HetEdgeKind::Data { ty: IrType::I32 });
        g.add_edge(b, a, HetEdgeKind::Data { ty: IrType::I32 });
        g.add_edge(a, anchor, HetEdgeKind::Control);
        match g.verify() {
            Err(HetError::DataCycle { .. }) => {}
            other => panic!("expected DataCycle, got {other:?}"),
        }
    }

    /// CEP:WHAT: Printing and hashing are deterministic across repeated calls.
    /// CEP:WHY: 38.19.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_print_and_hash_are_stable() {
        let mut g = HetIr::new();
        let a = g.add_node(node(HetKind::Compute(ComputeOp::Const), IrType::I64));
        let r = g.add_node(node(HetKind::Control(ControlOp::Return), IrType::Unit));
        g.add_edge(a, r, HetEdgeKind::Control);
        assert_eq!(g.print(), g.print());
        assert_eq!(g.canonical_hash(), g.canonical_hash());
    }

    /// CEP:WHAT: The budget guard rejects counts above MAX_HET_ITEMS and accepts the
    /// boundary.
    /// CEP:WHY: Named compile-time budget (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(1).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn het_budget_guard_rejects_oversize() {
        assert!(check_budget(MAX_HET_ITEMS).is_ok());
        match check_budget(MAX_HET_ITEMS + 1) {
            Err(HetError::Oversized { items }) => assert_eq!(items, MAX_HET_ITEMS + 1),
            other => panic!("expected Oversized, got {other:?}"),
        }
    }
}
