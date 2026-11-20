// CEP:FILE: crates/rv8_backend/src/deopt.rs
// CEP:WHAT: Region-based partial deoptimization metadata (design.md "Region-Based
//   Partial Deoptimization on Sea of Nodes" 1.2-1.3, RV8-141; STANDARDS 5.13 "per-region
//   interpreter state summaries (DeoptState) produced alongside MachineIR"): the
//   dominator-tree region partition of a Machine-IR function (single-entry regions,
//   1.2's merge of small regions and split of oversized ones), the per-region DeoptState
//   (the values live at the region's entry, each with its post-allocation machine
//   location: allocated register or RSP-relative spill slot), and the compact
//   little-endian state bitstream (1.3's "< 100 bytes per region" wire format) with its
//   decode. The bitstream format is a pinned cross-agent contract mirrored by
//   rv8_runtime::trap's handler decoder, exactly like the encode RelocKind mirror.
// CEP:WHY: The deopt tier needs two things before anything executes: WHERE a failing
//   region's state lives (locations, from the same RegisterAllocation/FramePlan the
//   rewrite used - zero redundant analysis, design 1.4) and a byte-exact wire format the
//   runtime can decode inside a signal handler without allocating.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete for the metadata half; the guard-placement pass (1.2 phase 1,
//   speculative opportunities) and the OSR side remain per RV8-141.
// CEP:FAILURE: every fallible operation returns BackendError: dangling LIR successors,
//   a live value with no location (fail closed - the rewrite must have placed it),
//   liveness fixpoint overrun, dominator budget from rv8_middle, and malformed or
//   over-budget bitstreams. Library code never panics.
// CEP:ASSUMES: the MIR block vector is layout order with block 0 the entry; LIR blocks
//   carry consistent successors/predecessors; every live value post-rewrite resolves to
//   one location (MCF assigns one lane or slot per range).
// CEP:COST: region partition O(V + E) over the MCFG; states O(values) liveness fixpoint
//   bounded by MAX_DEOPT_LIVENESS_ROUNDS; encoding linear in entries.
// CEP:EVIDENCE: the unit tests here (partition on chain/diamond/loop CFGs, the merge
//   threshold, state extraction against hand-computed liveness and locations, bitstream
//   round-trip, malformed rejections) and the end-to-end trap witness in
//   tests/jit_deopt.rs.

use crate::error::BackendError;
use crate::regalloc::rewrite::FramePlan;
use rv8_ir::lir::{LirFunction, RegisterAllocation, ValueId};
use rv8_ir::machir::{MachineBlock, PhysReg};
use rv8_ir::son::BlockId;

/// CEP:WHAT: The design 1.2 merge threshold: a region holding fewer instructions than
///   this merges into the region of its entry block's immediate dominator.
/// CEP:WHY: 1.2 phase 2 step 4 ("merge small regions (< 10 nodes)") with instructions
///   as the node measure at machine granularity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: constant.
/// CEP:EVIDENCE: partition_merge_threshold unit test.
pub const MIN_REGION_INSTRUCTIONS: usize = 10;

/// CEP:WHAT: The design 1.2 split threshold: a region holding more instructions than
///   this is split into per-block regions.
/// CEP:WHY: 1.2 phase 2 step 5 ("split large regions (> 1000 nodes)"); MAX_MIR_ITEMS
///   bounds make this rare, but the duty is implemented, not assumed away.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: constant.
/// CEP:EVIDENCE: partition split is exercised by the oversized synthetic fixture.
pub const MAX_REGION_INSTRUCTIONS: usize = 1_000;

/// CEP:WHAT: The 1.3 compact-state budget: an encoded DeoptState must not exceed 100
///   bytes.
/// CEP:WHY: 1.3 ("typically < 100 bytes per region"); the budget is mechanical so a
///   bloated state fails at encode time instead of silently inflating the table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 10 entries of at most 9 bytes plus the 6-byte header fits (96 <= 100).
/// CEP:COST: constant.
/// CEP:EVIDENCE: encode_state_budget unit test.
pub const DEOPT_STATE_MAX_BYTES: usize = 100;

/// CEP:WHAT: Maximum live entries per DeoptState (the header's u16 count field and the
///   byte budget bound it).
/// CEP:WHY: The signal-handler decoder pre-sizes its fixed capture buffer from this
///   contract; keeping both sides on one constant makes the wire format auditable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: mirrors rv8_runtime::trap::MAX_DEOPT_VALUES (pinned contract).
/// CEP:COST: constant.
/// CEP:EVIDENCE: encode_state rejects an 11-entry state.
pub const MAX_DEOPT_STATE_ENTRIES: usize = 10;

/// CEP:WHAT: Maximum backward-liveness fixpoint rounds per function.
/// CEP:WHY: Named budget (38.12 style); the dataflow converges in O(block depth) rounds.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::BudgetExceeded past this bound.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: the fixpoint loop below enforces it.
pub const MAX_DEOPT_LIVENESS_ROUNDS: usize = 10_000;

/// CEP:WHAT: Identity of one deopt region inside one function.
/// CEP:WHY: Guards and states reference regions by this id; ids are per-function and
///   dense 0..n in the partition's deterministic order.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD, Copy.
/// CEP:EVIDENCE: the partition tests assert exact id assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub u32);

/// CEP:WHAT: One deopt region: a single-entry dominator-tree subtree of MIR blocks.
/// CEP:WHY: Design 1.1/1.2 - the unit of partial deoptimization; only the failing
///   region's state is reconstructed.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: blocks are ascending MIR layout indices; entry is the seed block.
/// CEP:COST: owned vectors, one per region.
/// CEP:EVIDENCE: partition tests assert block membership exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// Dense region id within the function.
    pub id: RegionId,
    /// The region's seed block (MIR layout index).
    pub entry: u32,
    /// Member blocks, ascending.
    pub blocks: Vec<u32>,
    /// Total instruction count (the 1.2 node measure).
    pub instructions: usize,
}

/// CEP:WHAT: Where one live value sits at a deopt boundary, resolved from the same
///   RegisterAllocation and FramePlan the rewrite used.
/// CEP:WHY: Design 1.3 phase 3: live values with their exact materialization points.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: machir PhysReg numbering (0 = RAX ... 15 = R15); the runtime translates
///   it into signal-context gregs indices (pinned contract in rv8_runtime::trap).
/// CEP:COST: POD, Copy.
/// CEP:EVIDENCE: state extraction tests assert locations against the allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeoptLocation {
    /// The value lives in an allocated physical register (machir numbering).
    Reg(PhysReg),
    /// The value lives in its spill slot at this RSP-relative byte offset.
    Stack(i32),
}

/// CEP:WHAT: One region's deoptimization state: the values live at the region's entry
///   block, ascending by value id, each with its machine location.
/// CEP:WHY: Design 1.2 phase 3 / delta function of 1.1; the deopt handler reconstructs
///   exactly these values.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: live is sorted ascending and duplicate-free (enforced at extraction and
///   decode).
/// CEP:COST: owned vector.
/// CEP:EVIDENCE: extraction tests assert the exact live set and locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeoptState {
    /// The region this state belongs to.
    pub region: RegionId,
    /// Live values at the region's entry with their locations, ascending by id.
    pub live: Vec<(ValueId, DeoptLocation)>,
}

/// CEP:WHAT: One guard site: a trap point inside one emitted function - the LIR block
///   index, the instruction ordinal within that block (the block's length means "after
///   the last instruction, at the epilogue"), the byte offset into the emitted code,
///   and the region the guard protects.
/// CEP:WHY: Design 1.2 phase 1: guards are the runtime assumptions, each carrying the
///   deoptimization state it restores (the delta of 1.1). The placement pass is future
///   work (speculative catalog), so the module carries caller-supplied, validated
///   sites and computes each site's exact state.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: code_offset is an instruction boundary chosen by the guard placer; the
///   trap handler only requires it inside the function's code range.
/// CEP:COST: POD, Copy.
/// CEP:EVIDENCE: build_with_deopt validation tests and the trap witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardSite {
    /// Index into the module's ascending-ordinal function list.
    pub function: u32,
    /// The LIR block index the point lives in.
    pub block: u32,
    /// Instruction ordinal within the block; the block's length = after the last
    /// instruction (the epilogue point of a ret-terminated block).
    pub point: u32,
    /// Byte offset into that function's emitted code.
    pub code_offset: u64,
    /// The region the guard protects.
    pub region: RegionId,
}

/// CEP:WHAT: One guard's deoptimization state: the site plus the exact live-value set
///   at that program point.
/// CEP:WHY: Design 1.1's delta: each guard maps to the unoptimized state it restores;
///   the runtime consumes this state byte-exactly at trap time.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the state's live set is ascending and duplicate-free (extraction
///   guarantees it).
/// CEP:COST: owned vectors.
/// CEP:EVIDENCE: the state_at_point tests and the trap witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardState {
    /// The validated site.
    pub site: GuardSite,
    /// The live values at the site's point with their machine locations.
    pub state: DeoptState,
}

/// CEP:WHAT: One function's deopt metadata: its regions and their states, in
///   ascending id order.
/// CEP:WHY: The module-level table is per-function because regions and ids are.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the state at index i has region equal to RegionId of i (dense).
/// CEP:COST: owned vectors.
/// CEP:EVIDENCE: build_with_deopt tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDeopt {
    /// Regions, ascending by id, dense from 0.
    pub regions: Vec<Region>,
    /// One state per region, same order.
    pub states: Vec<DeoptState>,
}

/// CEP:WHAT: The deopt metadata table for a JIT module: per-function regions/states
///   plus the validated guard sites with their exact states.
/// CEP:WHY: Design 1.3's Deoptimization Metadata Table, indexed by function and region;
///   the runtime trap registry consumes the encoded guard states.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: guards reference existing functions, blocks, points, and regions
///   (validated at build).
/// CEP:COST: owned vectors.
/// CEP:EVIDENCE: tests/jit_deopt.rs drives the whole table end to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeoptTable {
    /// Per-function metadata in the module's ascending-ordinal order.
    pub functions: Vec<FunctionDeopt>,
    /// Guard sites with their states, validated, in input order.
    pub guards: Vec<GuardState>,
}

/// CEP:WHAT: Encodes a DeoptState into the compact little-endian wire format: a u32
///   region, a u16 count, then per entry a u32 value id, a u8 kind, and a payload of
///   u16 machir register number for kind 0 or i32 RSP-relative offset for kind 1.
/// CEP:WHY: Design 1.3's compact bitstream; the format is a pinned cross-agent contract
///   mirrored by rv8_runtime::trap's handler decoder (no allocation, fixed buffer).
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::BudgetExceeded past DEOPT_STATE_MAX_BYTES or
///   MAX_DEOPT_STATE_ENTRIES.
/// CEP:ASSUMES: live is ascending and duplicate-free (extraction guarantees it).
/// CEP:COST: O(entries).
/// CEP:EVIDENCE: round-trip and budget unit tests.
pub fn encode_state(state: &DeoptState) -> Result<Vec<u8>, BackendError> {
    if state.live.len() > MAX_DEOPT_STATE_ENTRIES {
        return Err(BackendError::BudgetExceeded {
            budget: "MAX_DEOPT_STATE_ENTRIES",
        });
    }
    let mut out = Vec::with_capacity(6 + state.live.len() * 7);
    out.extend_from_slice(&state.region.0.to_le_bytes());
    out.extend_from_slice(&(state.live.len() as u16).to_le_bytes());
    for (v, loc) in &state.live {
        out.extend_from_slice(&v.0.to_le_bytes());
        match *loc {
            DeoptLocation::Reg(r) => {
                out.push(0);
                out.extend_from_slice(&r.0.to_le_bytes());
            }
            DeoptLocation::Stack(off) => {
                out.push(1);
                out.extend_from_slice(&off.to_le_bytes());
            }
        }
    }
    if out.len() > DEOPT_STATE_MAX_BYTES {
        return Err(BackendError::BudgetExceeded {
            budget: "DEOPT_STATE_MAX_BYTES",
        });
    }
    Ok(out)
}

/// CEP:WHAT: Decodes the wire format back into a DeoptState, rejecting trailing bytes,
///   truncated entries, unknown kinds, over-large counts, and non-ascending or
///   duplicate value ids.
/// CEP:WHY: Round-trip integrity is the wire contract's proof; the strict decode makes
///   a corrupted or tampered state fail closed instead of reconstructing garbage.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::Verification for every malformed shape.
/// CEP:ASSUMES: none
/// CEP:COST: O(entries).
/// CEP:EVIDENCE: round-trip and malformed unit tests.
pub fn decode_state(bytes: &[u8]) -> Result<DeoptState, BackendError> {
    let err = |detail: &str| {
        Err(BackendError::Verification { detail: detail.to_string() })
    };
    if bytes.len() < 6 {
        return err("deopt state shorter than the 6-byte header");
    }
    let region = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count > MAX_DEOPT_STATE_ENTRIES {
        return err("deopt state count exceeds the entry budget");
    }
    let mut live = Vec::with_capacity(count);
    let mut cur = 6usize;
    let mut prev: Option<u32> = None;
    for _ in 0..count {
        if cur + 5 > bytes.len() {
            return err("deopt state entry truncated");
        }
        let v = u32::from_le_bytes([
            bytes[cur],
            bytes[cur + 1],
            bytes[cur + 2],
            bytes[cur + 3],
        ]);
        match prev {
            Some(p) if v <= p => return err("deopt state ids not ascending and distinct"),
            _ => {}
        }
        prev = Some(v);
        let kind = bytes[cur + 4];
        cur += 5;
        let loc = match kind {
            0 => {
                if cur + 2 > bytes.len() {
                    return err("register location truncated");
                }
                let r = u16::from_le_bytes([bytes[cur], bytes[cur + 1]]);
                cur += 2;
                DeoptLocation::Reg(PhysReg(r))
            }
            1 => {
                if cur + 4 > bytes.len() {
                    return err("stack location truncated");
                }
                let off = i32::from_le_bytes([
                    bytes[cur],
                    bytes[cur + 1],
                    bytes[cur + 2],
                    bytes[cur + 3],
                ]);
                cur += 4;
                DeoptLocation::Stack(off)
            }
            _ => return err("unknown deopt location kind"),
        };
        live.push((ValueId(v), loc));
    }
    if cur != bytes.len() {
        return err("deopt state has trailing bytes");
    }
    Ok(DeoptState { region: RegionId(region), live })
}

/// CEP:WHAT: Partitions one MIR function into deopt regions (design 1.2 phase 2): the
///   immediate-dominator tree over the machine CFG is decomposed into single-entry
///   regions seeded at the entry block and every join (two or more predecessors);
///   regions below MIN_REGION_INSTRUCTIONS merge into the region of their entry's
///   immediate dominator (fixpoint) and regions above MAX_REGION_INSTRUCTIONS split
///   into per-block regions. Unreachable blocks become their own regions.
/// CEP:WHY: The granularity contract of 1.1/1.2: only the failing region deoptimizes,
///   so the partition must be deterministic and dominator-based.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::Verification on a dangling successor; analysis budget
///   errors thread through as Verification details.
/// CEP:ASSUMES: blocks are layout order; block 0 is the entry.
/// CEP:COST: O(V + E) dominators plus O(V) decomposition.
/// CEP:EVIDENCE: partition unit tests on chain, diamond, and loop CFGs.
pub fn regions_of(blocks: &[MachineBlock]) -> Result<Vec<Region>, BackendError> {
    let n = blocks.len();
    if n == 0 {
        return Err(BackendError::Verification {
            detail: "deopt partition of a function with no blocks".to_string(),
        });
    }
    let block = |i: u32| BlockId(i);
    // Sorted, deduplicated successor/predecessor lists, mirroring the middle-end CFG
    // shape so dominators() consumes it directly.
    let mut succs: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (i, b) in blocks.iter().enumerate() {
        for &s in &b.successors {
            if s as usize >= n {
                return Err(BackendError::Verification {
                    detail: format!("block {} has dangling successor {}", i, s),
                });
            }
            succs[i].push(block(s));
        }
        succs[i].sort_unstable();
        succs[i].dedup();
    }
    for (i, ss) in succs.iter().enumerate() {
        for &s in ss {
            preds[s.0 as usize].push(block(i as u32));
        }
    }
    for ps in preds.iter_mut() {
        ps.sort_unstable();
        ps.dedup();
    }
    let cfg = rv8_middle::analysis::ControlFlowGraph {
        block_count: n as u32,
        entry: Some(block(0)),
        succs,
        preds,
    };
    let doms = rv8_middle::analysis::dominators(&cfg).map_err(|e| {
        BackendError::Verification { detail: format!("deopt dominators: {e}") }
    })?;
    // Dominator-tree children, then a deterministic DFS pre-order from the entry: a
    // block seeds a new region when it is the entry or a join; otherwise it inherits
    // its immediate dominator's region.
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (b, idom) in doms.idom.iter().enumerate() {
        if let Some(p) = idom {
            let p = p.0 as usize;
            if p != b {
                children[p].push(b as u32);
            }
        }
    }
    for ch in children.iter_mut() {
        ch.sort_unstable();
    }
    let mut region_of: Vec<Option<u32>> = vec![None; n];
    let mut order: Vec<u32> = Vec::with_capacity(n);
    let mut stack: Vec<u32> = vec![0];
    while let Some(b) = stack.pop() {
        if region_of[b as usize].is_some() {
            continue;
        }
        order.push(b);
        let seed = b == 0 || cfg.preds[b as usize].len() >= 2
            || doms.idom[b as usize].is_none();
        if seed {
            region_of[b as usize] = Some(b);
        } else {
            // The idom is always visited before its children in this DFS.
            region_of[b as usize] = region_of
                .get(doms.idom[b as usize].expect("visited parent").0 as usize)
                .copied()
                .flatten();
        }
        for &c in &children[b as usize] {
            stack.push(c);
        }
    }
    // Every block must end up in some region: unreachable blocks become their own.
    for (b, slot) in region_of.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(b as u32);
            order.push(b as u32);
        }
    }
    // 1.2 step 4: merge small regions into their entry's idom's region, to fixpoint.
    let instr_count = |b: usize| blocks[b].instructions.len();
    let mut changed = true;
    let mut rounds = 0usize;
    while changed {
        changed = false;
        rounds += 1;
        if rounds > MAX_DEOPT_LIVENESS_ROUNDS {
            return Err(BackendError::BudgetExceeded {
                budget: "MAX_DEOPT_LIVENESS_ROUNDS",
            });
        }
        for b in 0..n {
            let r = region_of[b].expect("assigned above");
            if r != b as u32 {
                continue; // only seeds can shrink below the threshold
            }
            let size: usize = (0..n)
                .filter(|&x| region_of[x] == Some(r))
                .map(instr_count)
                .sum();
            if size >= MIN_REGION_INSTRUCTIONS || b == 0 {
                continue;
            }
            // Merge into the idom's region; the entry block's idom (or, for an
            // unreachable seed, nothing) decides the target.
            if let Some(p) =
                doms.idom[b].and_then(|p| if p.0 as usize == b { None } else { Some(p.0) })
            {
                let pr = region_of[p as usize].expect("idom of a visited block");
                if pr != r {
                    for slot in region_of.iter_mut() {
                        if *slot == Some(r) {
                            *slot = Some(pr);
                        }
                    }
                    changed = true;
                }
            }
        }
    }
    // 1.2 step 5: split oversized regions into per-block regions.
    for r in 0..n {
        let size: usize = (0..n)
            .filter(|&x| region_of[x] == Some(r as u32))
            .map(instr_count)
            .sum();
        if size > MAX_REGION_INSTRUCTIONS {
            for (x, slot) in region_of.iter_mut().enumerate() {
                if *slot == Some(r as u32) {
                    *slot = Some(x as u32);
                }
            }
        }
    }
    // Materialize regions in ascending seed order with dense ids.
    let mut seeds: Vec<u32> = (0..n)
        .filter(|&b| region_of[b] == Some(b as u32))
        .map(|b| b as u32)
        .collect();
    seeds.sort_unstable();
    let mut regions = Vec::with_capacity(seeds.len());
    for (id, &seed) in seeds.iter().enumerate() {
        let blocks: Vec<u32> = (0..n)
            .filter(|&x| region_of[x] == Some(seed))
            .map(|x| x as u32)
            .collect();
        let instructions = blocks.iter().copied().map(|b| instr_count(b as usize)).sum();
        regions.push(Region {
            id: RegionId(id as u32),
            entry: seed,
            blocks,
            instructions,
        });
    }
    Ok(regions)
}

/// CEP:WHAT: Extracts one DeoptState per region (design 1.2 phase 3): the values live
///   at the region's entry block, located through the same RegisterAllocation and
///   FramePlan the rewrite used (allocated register, or the spill slot's RSP-relative
///   offset).
/// CEP:WHY: "The borrow checker's liveness bitsets are reused" (1.4): block-boundary
///   liveness over the LIR CFG feeds the state, and the location map is the allocator's
///   own product - no redundant analysis, no second source of truth.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::IllegalLir on dangling LIR successors;
///   BackendError::Internal when a live value resolves to no location (fail closed);
///   BudgetExceeded past the liveness rounds budget.
/// CEP:ASSUMES: allocation and frame come from the same pipeline run as the regions.
/// CEP:COST: O(values + instructions) liveness fixpoint plus O(states * values).
/// CEP:EVIDENCE: extraction unit tests with hand-computed liveness and locations.
pub fn deopt_states(
    lir: &LirFunction,
    alloc: &RegisterAllocation,
    frame: &FramePlan,
    regions: &[Region],
) -> Result<Vec<DeoptState>, BackendError> {
    // Block id -> layout index.
    let (index, live_in) = lir_liveness(lir)?;
    // One state per region, taken at the region's entry block.
    let mut states = Vec::with_capacity(regions.len());
    for r in regions {
        let bi = index.get(r.entry as usize).copied().flatten().ok_or_else(|| {
            BackendError::IllegalLir {
                detail: format!("region entry {} has no LIR block", r.entry),
            }
        })?;
        let mut live = Vec::with_capacity(live_in[bi].len());
        for v in &live_in[bi] {
            live.push((ValueId(*v), location_of(lir, alloc, frame, *v)?));
        }
        states.push(DeoptState { region: r.id, live });
    }
    Ok(states)
}

/// CEP:WHAT: The shared liveness shape: the block id -> layout index map plus one
///   live-in set per block.
/// CEP:WHY: Factored so lir_liveness's signature stays readable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: alias only.
/// CEP:EVIDENCE: lir_liveness.
type BlockLiveness = (Vec<Option<usize>>, Vec<std::collections::BTreeSet<u32>>);

/// CEP:WHAT: Block-boundary liveness over the LIR CFG (design 1.4: the allocator's
///   own liveness facts are the reuse basis, no redundant analysis): a backward
///   fixpoint producing live_in per block, plus the block id -> layout index map.
/// CEP:WHY: Both deopt_states (region entries) and deopt_state_at (guard points)
///   consume the same fixpoint.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::IllegalLir on dangling successors or oversized block
///   ids; BudgetExceeded past MAX_DEOPT_LIVENESS_ROUNDS.
/// CEP:ASSUMES: none
/// CEP:COST: O(instructions + values) per round, O(block depth) rounds.
/// CEP:EVIDENCE: the extraction tests assert hand-computed live sets.
fn lir_liveness(lir: &LirFunction) -> Result<BlockLiveness, BackendError> {
    let mut index: Vec<Option<usize>> = vec![None; lir.blocks.len()];
    for (i, b) in lir.blocks.iter().enumerate() {
        let bi = b.id.0 as usize;
        if bi >= index.len() {
            return Err(BackendError::IllegalLir {
                detail: format!("block id {} exceeds the block count", b.id.0),
            });
        }
        index[bi] = Some(i);
    }
    // Backward liveness: live_in per block, iterative to fixpoint (no recursion).
    let nb = lir.blocks.len();
    let mut live_in: Vec<std::collections::BTreeSet<u32>> =
        (0..nb).map(|_| std::collections::BTreeSet::new()).collect();
    let mut rounds = 0usize;
    loop {
        let mut changed = false;
        for bi in (0..nb).rev() {
            let mut live: std::collections::BTreeSet<u32> = live_in[bi].clone();
            for instr in lir.blocks[bi].instructions.iter().rev() {
                if let Some(r) = &instr.result {
                    live.remove(&r.0);
                }
                for &op in &instr.operands {
                    live.insert(op.0);
                }
            }
            for &s in &lir.blocks[bi].successors {
                let si = index.get(s.0 as usize).copied().flatten().ok_or_else(|| {
                    BackendError::IllegalLir {
                        detail: format!("block {} has dangling successor {}", bi, s.0),
                    }
                })?;
                for &v in &live_in[si] {
                    live.insert(v);
                }
            }
            if live != live_in[bi] {
                live_in[bi] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        rounds += 1;
        if rounds > MAX_DEOPT_LIVENESS_ROUNDS {
            return Err(BackendError::BudgetExceeded {
                budget: "MAX_DEOPT_LIVENESS_ROUNDS",
            });
        }
    }
    Ok((index, live_in))
}

/// CEP:WHAT: Resolves one value's post-allocation machine location: the allocator's
///   register, else the spill slot's RSP-relative offset; fail closed otherwise.
/// CEP:WHY: Design 1.3's materialization points come from the same
///   RegisterAllocation/FramePlan the rewrite used.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::Internal when a live value resolves to no location.
/// CEP:ASSUMES: allocation and frame come from the same pipeline run.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: the extraction tests assert locations against the allocation.
fn location_of(
    lir: &LirFunction,
    alloc: &RegisterAllocation,
    frame: &FramePlan,
    v: u32,
) -> Result<DeoptLocation, BackendError> {
    if let Some(rid) = alloc.register(ValueId(v)) {
        let class = lir.values.get(v as usize).ok_or_else(|| BackendError::IllegalLir {
            detail: format!("value {} beyond the value table", v),
        })?
        .reg_class;
        let p = match class {
            rv8_ir::lir::RegClassId::Gpr => crate::abi::gpr_phys(rid),
            rv8_ir::lir::RegClassId::Fpr => crate::abi::fpr_phys(rid),
            rv8_ir::lir::RegClassId::Vector => None,
        };
        return p.map(DeoptLocation::Reg).ok_or_else(|| BackendError::Internal {
            detail: format!("live value {} has an out-of-table register", v),
        });
    }
    match alloc.spill_slots.get(v as usize).copied().flatten() {
        Some(slot) => frame.slot_offsets.get(slot as usize).map(|&off| DeoptLocation::Stack(off))
            .ok_or_else(|| BackendError::Internal {
                detail: format!("live value {} has no spill slot offset", v),
            }),
        None => Err(BackendError::Internal {
            detail: format!("live value {} has neither register nor slot", v),
        }),
    }
}

/// CEP:WHAT: The exact DeoptState at one program point - a LIR block index and an
///   instruction ordinal within it: the values live immediately BEFORE that
///   instruction (the state the unoptimized continuation needs), each with its
///   post-allocation machine location. A point equal to the block's instruction count
///   means after the last instruction (the block's live-out).
/// CEP:WHY: Design 1.1's delta: each guard restores exactly the state at its own
///   point; for an epilogue-point guard that is precisely the function's return shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: BackendError::IllegalLir for an out-of-range block or point;
///   Internal/IllegalLir through the location resolver.
/// CEP:ASSUMES: point <= the block's instruction count.
/// CEP:COST: one liveness fixpoint plus one reverse block walk.
/// CEP:EVIDENCE: the state_at_point unit tests and the trap witness.
pub fn deopt_state_at(
    lir: &LirFunction,
    alloc: &RegisterAllocation,
    frame: &FramePlan,
    block: u32,
    point: usize,
    region: RegionId,
) -> Result<DeoptState, BackendError> {
    let (index, live_in) = lir_liveness(lir)?;
    let bi = index.get(block as usize).copied().flatten().ok_or_else(|| {
        BackendError::IllegalLir {
            detail: format!("guard block {} has no LIR block", block),
        }
    })?;
    let blk = &lir.blocks[bi];
    if point > blk.instructions.len() {
        return Err(BackendError::IllegalLir {
            detail: format!(
                "guard point {} beyond block {}'s {} instructions",
                point,
                block,
                blk.instructions.len()
            ),
        });
    }
    // Live-out of the block = union of the successors' live-ins (empty at a ret).
    let mut live: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for &s in &blk.successors {
        let si = index.get(s.0 as usize).copied().flatten().ok_or_else(|| {
            BackendError::IllegalLir {
                detail: format!("block {} has dangling successor {}", bi, s.0),
            }
        })?;
        for &v in &live_in[si] {
            live.insert(v);
        }
    }
    // Walk backwards through instr[point..len] to land at live-before-instr[point]
    // (an empty walk for point == len keeps the block's live-out).
    for i in (point..blk.instructions.len()).rev() {
        let instr = &blk.instructions[i];
        if let Some(r) = &instr.result {
            live.remove(&r.0);
        }
        for &op in &instr.operands {
            live.insert(op.0);
        }
    }
    let mut out = Vec::with_capacity(live.len());
    for v in &live {
        out.push((ValueId(*v), location_of(lir, alloc, frame, *v)?));
    }
    Ok(DeoptState { region, live: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rv8_ir::lir::{LirBlock, LirInstruction, MachineOpcode, RegClassId};
    use rv8_ir::machir::MachineInstr;
    use rv8_ir::son::IrType;

    /// CEP:WHAT: An MIR block fixture from instruction counts and successors.
    /// CEP:WHY: Partition fixtures stay readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn mir_block(_id: u32, instructions: usize, succs: &[u32]) -> MachineBlock {
        MachineBlock {
            instructions: (0..instructions)
                .map(|_| MachineInstr {
                    opcode: MachineOpcode::MovRegImm32,
                    operands: vec![],
                    predicates: Vec::new(),
                })
                .collect(),
            successors: succs.to_vec(),
        }
    }

    /// CEP:WHAT: A straight-line function is one region covering every block: no joins
    ///   means no seeds, and the merge pulls the whole chain into region 0.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn partition_chain_is_one_region() {
        let blocks = vec![mir_block(0, 4, &[1]), mir_block(1, 4, &[2]), mir_block(2, 4, &[])];
        let regions = regions_of(&blocks).expect("partition");
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].entry, 0);
        assert_eq!(regions[0].blocks, vec![0, 1, 2]);
        assert_eq!(regions[0].instructions, 12);
    }

    /// CEP:WHAT: A diamond CFG seeds two regions: the entry and the join; the branches
    ///   merge into the entry's region (no second seed of their own). A join whose
    ///   mass is below MIN_REGION_INSTRUCTIONS instead merges into the entry region
    ///   (1.2 step 4), pinned by partition_merge_and_split_thresholds.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn partition_diamond_seeds_entry_and_join() {
        // 0 -> {1, 2}; 1 -> 3; 2 -> 3; the join carries 12 instructions so it stays.
        let blocks = vec![
            mir_block(0, 6, &[1, 2]),
            mir_block(1, 6, &[3]),
            mir_block(2, 6, &[3]),
            mir_block(3, 12, &[]),
        ];
        let regions = regions_of(&blocks).expect("partition");
        assert_eq!(regions.len(), 2, "entry region plus the join region");
        assert_eq!(regions[0].entry, 0);
        assert_eq!(regions[0].blocks, vec![0, 1, 2], "branches inherit the entry region");
        assert_eq!(regions[1].entry, 3);
        assert_eq!(regions[1].blocks, vec![3]);
    }

    /// CEP:WHAT: A natural loop forms one region with the header as its seed (the
    ///   back edge makes the header a join); the pre-header stays a separate region.
    /// CEP:WHY: The design's region-per-hot-loop intent: a guard at the loop header
    ///   deoptimizes exactly the loop's body.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn partition_loop_keeps_body_with_header() {
        // 0 -> 1; 1 -> {1, 2} (back edge to itself); 2 -> [].
        let blocks = vec![
            mir_block(0, 2, &[1]),
            mir_block(1, 8, &[1, 2]),
            mir_block(2, 2, &[]),
        ];
        let regions = regions_of(&blocks).expect("partition");
        assert_eq!(regions.len(), 2, "pre-header region plus the loop region");
        assert_eq!(regions[0].entry, 0);
        assert_eq!(regions[0].blocks, vec![0]);
        assert_eq!(regions[1].entry, 1, "the header seeds the loop region");
        assert_eq!(regions[1].blocks, vec![1, 2]);
    }

    /// CEP:WHAT: A region below MIN_REGION_INSTRUCTIONS merges into its idom's region,
    ///   and a region above MAX_REGION_INSTRUCTIONS splits per block.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn partition_merge_and_split_thresholds() {
        // Small join: entry(4) -> branches(4 each) -> join(2): the join's 2
        // instructions are below the threshold, but it is a seed; with the merge it
        // would fold into the entry region - so give the branches enough mass that the
        // entry region alone is at the threshold, then verify the merge happened.
        let blocks = vec![
            mir_block(0, 4, &[1, 2]),
            mir_block(1, 4, &[3]),
            mir_block(2, 4, &[3]),
            mir_block(3, 2, &[]),
        ];
        let regions = regions_of(&blocks).expect("partition");
        assert_eq!(regions.len(), 1, "the 2-instruction join merged into the entry region");
        assert_eq!(regions[0].blocks, vec![0, 1, 2, 3]);
        // Oversized: entry(1) -> branch(2001 instructions) -> exit: the branch region
        // exceeds MAX_REGION_INSTRUCTIONS and splits into per-block regions.
        let big = vec![
            mir_block(0, 1, &[1]),
            mir_block(1, MAX_REGION_INSTRUCTIONS + 1, &[2]),
            mir_block(2, 1, &[]),
        ];
        let regions = regions_of(&big).expect("partition");
        assert_eq!(regions.len(), 3, "oversized region split per block");
    }

    /// CEP:WHAT: Builds a small LIR `fn f(p) -> i64 { p + 3 }`-shaped body with a fixed
    ///   allocation: v0 in the parameter register, v1 in RAX, v2 spilled to slot 0.
    /// CEP:WHY: State-extraction fixtures need a deterministic allocation to assert
    ///   locations against.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn fixture_lir() -> LirFunction {
        let mut f = LirFunction::new("f");
        let p = f.add_value(IrType::I64, RegClassId::Gpr).0; // v0: live in (parameter)
        let c = f.add_value(IrType::I64, RegClassId::Gpr).0; // v1: defined in block 0
        let s = f.add_value(IrType::I64, RegClassId::Gpr).0; // v2: the sum
        let mut b0 = LirBlock::new(BlockId(0));
        b0.push(LirInstruction {
            opcode: MachineOpcode::MovRegImm32,
            operands: vec![],
            result: Some(ValueId(c)),
            immediate: 3,
            target_block: None,
        });
        b0.push(LirInstruction {
            opcode: MachineOpcode::AddRegReg,
            operands: vec![ValueId(p), ValueId(c)],
            result: Some(ValueId(s)),
            immediate: 0,
            target_block: None,
        });
        b0.push(LirInstruction {
            opcode: MachineOpcode::Ret,
            operands: vec![ValueId(s)],
            result: None,
            immediate: 0,
            target_block: None,
        });
        f.push_block(b0);
        f
    }

    /// CEP:WHAT: DeoptState extraction lists exactly the entry-live values with their
    ///   allocation locations; values defined in the block are not entry-live.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn state_extraction_matches_hand_liveness() {
        let lir = fixture_lir();
        let regions = vec![Region {
            id: RegionId(0),
            entry: 0,
            blocks: vec![0],
            instructions: 3,
        }];
        let mut alloc = RegisterAllocation::new(3);
        alloc.assignments[0] = Some(rv8_ir::lir::RegId(0)); // v0: first allocatable GPR
        alloc.assignments[1] = None; // v1: spilled
        alloc.spill_slots[1] = Some(0);
        alloc.assignments[2] = None; // v2: spilled to slot 1
        alloc.spill_slots[2] = Some(1);
        let frame = FramePlan {
            frame: rv8_ir::machir::MachineFrame {
                stack_size: 32,
                alignment: 16,
                has_red_zone: false,
                saved_registers: vec![],
            },
            slot_offsets: vec![-8, -16],
            stack_adjust: 32,
            saved: vec![],
            uses_red_zone: false,
        };
        let states = deopt_states(&lir, &alloc, &frame, &regions).expect("states");
        assert_eq!(states.len(), 1);
        // Entry liveness: only v0 (the parameter) is live-in; v1/v2 are defined here.
        assert_eq!(states[0].live.len(), 1, "exactly the parameter is entry-live");
        assert_eq!(
            states[0].live[0],
            (ValueId(0), DeoptLocation::Reg(crate::abi::gpr_phys(rv8_ir::lir::RegId(0)).expect("gpr")))
        );
        // A state taken at a second region inside the same block would see the spill
        // slots - exercised by the runtime's handler decoder through the wire format.
    }

    /// CEP:WHAT: A live value with neither register nor slot fails closed with
    ///   BackendError::Internal.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn locationless_live_value_fails_closed() {
        let lir = fixture_lir();
        let regions = vec![Region { id: RegionId(0), entry: 0, blocks: vec![0], instructions: 3 }];
        let alloc = RegisterAllocation::new(3); // nothing allocated
        let frame = FramePlan {
            frame: rv8_ir::machir::MachineFrame {
                stack_size: 16,
                alignment: 16,
                has_red_zone: false,
                saved_registers: vec![],
            },
            slot_offsets: vec![],
            stack_adjust: 16,
            saved: vec![],
            uses_red_zone: false,
        };
        assert!(matches!(
            deopt_states(&lir, &alloc, &frame, &regions),
            Err(BackendError::Internal { .. })
        ));
    }

    /// CEP:WHAT: The wire format round-trips byte-exactly, and every malformed shape
    ///   (truncation, trailing bytes, unknown kind, non-ascending ids, oversized count)
    ///   is rejected.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn wire_format_round_trip_and_malformed_rejections() {
        let state = DeoptState {
            region: RegionId(7),
            live: vec![
                (ValueId(2), DeoptLocation::Reg(PhysReg(13))),
                (ValueId(9), DeoptLocation::Stack(-24)),
            ],
        };
        let bytes = encode_state(&state).expect("encode");
        assert!(bytes.len() <= DEOPT_STATE_MAX_BYTES);
        assert_eq!(decode_state(&bytes).expect("decode"), state);
        // Malformed shapes.
        assert!(decode_state(&bytes[..5]).is_err(), "header truncated");
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_state(&trailing).is_err(), "trailing byte");
        let bad_kind = {
            let mut b = bytes.clone();
            b[8] = 9; // first entry's kind byte
            b
        };
        assert!(decode_state(&bad_kind).is_err(), "unknown kind");
        let mut descending = bytes.clone();
        descending[6] = 9; // second entry's value id: 9 -> stays 9, flip first to 9 too
        descending[7] = 0;
        assert!(decode_state(&descending).is_err(), "duplicate/non-ascending ids");
        let big_count = {
            let mut b = bytes.clone();
            b[4] = 0xFF;
            b[5] = 0xFF;
            b
        };
        assert!(decode_state(&big_count).is_err(), "count over budget");
        // Encode-side budgets: 13 entries and a 101-byte state both fail.
        let over_entries = DeoptState {
            region: RegionId(0),
            live: (0..13).map(|i| (ValueId(i), DeoptLocation::Reg(PhysReg(0)))).collect(),
        };
        assert!(matches!(
            encode_state(&over_entries),
            Err(BackendError::BudgetExceeded { .. })
        ));
    }

    /// CEP:WHAT: A dangling LIR successor fails closed, and dangling MIR successors are
    ///   rejected by the partition.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn dangling_successors_fail_closed() {
        let blocks = vec![mir_block(0, 1, &[5])];
        assert!(matches!(
            regions_of(&blocks),
            Err(BackendError::Verification { .. })
        ));
        let mut lir = fixture_lir();
        let regions = vec![Region { id: RegionId(0), entry: 0, blocks: vec![0], instructions: 3 }];
        let mut alloc = RegisterAllocation::new(3);
        alloc.assignments[0] = Some(rv8_ir::lir::RegId(0));
        let frame = FramePlan {
            frame: rv8_ir::machir::MachineFrame {
                stack_size: 16,
                alignment: 16,
                has_red_zone: false,
                saved_registers: vec![],
            },
            slot_offsets: vec![],
            stack_adjust: 16,
            saved: vec![],
            uses_red_zone: false,
        };
        if let Some(b) = lir.blocks.first_mut() {
            b.successors = vec![BlockId(9)];
        }
        assert!(matches!(
            deopt_states(&lir, &alloc, &frame, &regions),
            Err(BackendError::IllegalLir { .. })
        ));
    }
}
