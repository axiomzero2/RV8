// CEP:FILE: crates/rv8_ir/src/lower_het.rs
// CEP:WHAT: Level transitions 6.2 (Sea-of-Nodes to Heterogeneous IR linearization) and
//   6.3 (Heterogeneous IR to LIR) per docs/specs/multi_level_ir.md: 6.2 topologically
//   sorts nodes, turns control flow edges into explicit branches, and preserves data
//   dependencies as operand edges; 6.3 maps compute nodes to arithmetic instructions,
//   memory nodes to load/store instructions, communication nodes to explicit data
//   movement, and assigns a virtual register to every value.
// CEP:WHY: The autotuner consumes the Heterogeneous IR (spec 3.4, 7) and the backend
//   lowers it to LIR; both steps must be deterministic (38.10, 38.19) and fail closed on
//   unsupported operations.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: LowerError for verifier failures, data cycles, dead ends, and operations
//   without a Level-3 contract (division, remainder, dynamic shifts, general
//   conversions, allocation) which belong to instruction-selection tickets.
// CEP:ASSUMES: one function per graph; Het nodes carry their block from flattening.
// CEP:COST: 6.2 is O(N + E): liveness is a worklist pass and the topological order pops
//   from an ordered ready set keyed by node id, making the order a deterministic
//   function of ids alone. 6.3 is O(N + E).
// CEP:EVIDENCE: unit tests rv8_ir::lower_het::tests::*.
// CEP:SECURITY: iterative traversals only; no recursion on untrusted input (38.13).

use crate::hetir::{
    AddressSpace, CommOp, ComputeOp, ControlOp, HetEdgeKind, HetError, HetIr, HetKind, HetNode,
    HetNodeId, MemoryOp, MetaOp,
};
use crate::lir::{
    LirBlock, LirFunction, LirInstruction, MachineOpcode, RegClassId, SetKind, ValueId,
};
use crate::lowering::LowerError;
use crate::son::{BlockId, IrType, Opcode, SeaOfNodes};
use std::collections::{BTreeMap, BTreeSet};

/// CEP:WHAT: Classifies one Sea-of-Nodes input list into data and control positions.
/// CEP:WHY: The flattening convention is: Region/Merge inputs are all control; Branch
/// inputs are [condition, control]; Return inputs are [value?, control]; Phi inputs are
/// all data; everything else is data only.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: producers follow the flattening input convention.
/// CEP:COST: O(inputs).
/// CEP:EVIDENCE: unit test rv8_ir::lower_het::tests::linearize_straight_line.
fn split_inputs(opcode: Opcode, len: usize) -> (Vec<usize>, Vec<usize>) {
    match opcode {
        Opcode::Region | Opcode::Merge => (Vec::new(), (0..len).collect()),
        Opcode::Branch => (vec![0], if len >= 2 { vec![1] } else { Vec::new() }),
        Opcode::Return => {
            if len >= 2 {
                ((0..len - 1).collect(), vec![len - 1])
            } else {
                (Vec::new(), (0..len).collect())
            }
        }
        _ => ((0..len).collect(), Vec::new()),
    }
}

/// CEP:WHAT: Reports whether an opcode has side effects and is therefore a liveness root.
/// CEP:WHY: Dead pure-node elimination (3.3.4) keeps everything reachable from effects.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: loads count as pure for this contract (poisoning rides on dependency
/// edges at Level 2).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::lower_het::tests::linearize_drops_dead_nodes.
fn is_root(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Store
            | Opcode::Call { .. }
            | Opcode::Drop
            | Opcode::Branch
            | Opcode::Merge
            | Opcode::Region
            | Opcode::Return
    )
}

/// CEP:WHAT: Linearizes a Sea-of-Nodes graph into a Heterogeneous-IR graph (spec 6.2).
/// CEP:WHY: The single entry point used by the middle-end pipeline and tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: LowerError::UnsupportedOp for structural failures and LowerError::Het
/// for graph verification failures; never panics.
/// CEP:ASSUMES: structural validity of the input graph.
/// CEP:COST: O(N + E): one liveness worklist pass, one consumer-adjacency build, one
/// Kahn pass with an ordered ready set keyed by node id.
/// CEP:EVIDENCE: unit tests rv8_ir::lower_het::tests::*.
pub fn son_to_hetir(son: &SeaOfNodes) -> Result<HetIr, LowerError> {
    son.verify_structure()
        .map_err(|_| LowerError::UnsupportedOp { op: "son structure" })?;
    // Liveness: roots are side-effecting nodes; a node is live when a live node consumes
    // it through a data edge. Dead pure nodes are skipped, so the Heterogeneous IR
    // satisfies invariant 3.3.4 by construction.
    let mut live = vec![false; son.nodes.len()];
    let mut work: Vec<usize> = Vec::new();
    for (i, node) in son.nodes.iter().enumerate() {
        if is_root(node.opcode) {
            live[i] = true;
            work.push(i);
        }
    }
    while let Some(i) = work.pop() {
        let node = &son.nodes[i];
        let (data, _) = split_inputs(node.opcode, node.inputs.len());
        for d in data {
            if let Some(&src) = node.inputs.get(d) {
                let s = src.0 as usize;
                if s < live.len() && !live[s] {
                    live[s] = true;
                    work.push(s);
                }
            }
        }
    }
    // Data-consumer adjacency over live nodes, and live indegree. Phi nodes are
    // cycle-closure points (loop-carried values arrive as predecessor-edge copies at
    // LIR lowering), so data edges INTO a phi do not create ordering constraints; a
    // data cycle through phis is legal, a phi-free cycle still rejects below.
    let mut consumers: Vec<Vec<usize>> = vec![Vec::new(); son.nodes.len()];
    let mut indegree = vec![0u32; son.nodes.len()];
    for (j, node) in son.nodes.iter().enumerate() {
        if !live[j] {
            continue;
        }
        if matches!(node.opcode, Opcode::Phi) {
            continue;
        }
        let (data, _) = split_inputs(node.opcode, node.inputs.len());
        for d in data {
            if let Some(&src) = node.inputs.get(d) {
                let s = src.0 as usize;
                if s < live.len() && live[s] {
                    consumers[s].push(j);
                    indegree[j] += 1;
                }
            }
        }
    }
    // Kahn with an ordered ready set: pop order is a function of node ids alone.
    let mut ready: BTreeSet<u32> = (0..son.nodes.len() as u32)
        .filter(|&i| live[i as usize] && indegree[i as usize] == 0)
        .collect();
    let mut order: Vec<u32> = Vec::with_capacity(son.nodes.len());
    while let Some(&i) = ready.iter().next() {
        ready.remove(&i);
        order.push(i);
        for &j in &consumers[i as usize] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.insert(j as u32);
            }
        }
    }
    let emitted = live.iter().filter(|&&l| l).count();
    if order.len() != emitted {
        let stuck = (0..son.nodes.len())
            .find(|&i| live[i] && !order.contains(&(i as u32)))
            .unwrap_or(0);
        return Err(LowerError::Het(HetError::DataCycle {
            node: HetNodeId(stuck as u32),
        }));
    }
    // Map nodes and edges.
    let mut out = HetIr::new();
    let mut map = vec![None; son.nodes.len()];
    for &i in &order {
        let node = &son.nodes[i as usize];
        let (kind, imm) = match node.opcode {
            Opcode::Const(v) => (HetKind::Compute(ComputeOp::Const), v),
            Opcode::Parameter(k) => (HetKind::Compute(ComputeOp::Param), k as i64),
            Opcode::Phi => (HetKind::Compute(ComputeOp::Phi), 0),
            Opcode::Add => (HetKind::Compute(ComputeOp::Add), 0),
            Opcode::Sub => (HetKind::Compute(ComputeOp::Sub), 0),
            Opcode::Mul => (HetKind::Compute(ComputeOp::Mul), 0),
            Opcode::Div => (HetKind::Compute(ComputeOp::Div), 0),
            Opcode::Rem => (HetKind::Compute(ComputeOp::Rem), 0),
            Opcode::And => (HetKind::Compute(ComputeOp::And), 0),
            Opcode::Or => (HetKind::Compute(ComputeOp::Or), 0),
            Opcode::Xor => (HetKind::Compute(ComputeOp::Xor), 0),
            Opcode::Shl => (HetKind::Compute(ComputeOp::Shl), 0),
            Opcode::Shr => (HetKind::Compute(ComputeOp::Shr), 0),
            Opcode::Neg => (HetKind::Compute(ComputeOp::Neg), 0),
            Opcode::Not => (HetKind::Compute(ComputeOp::Not), 0),
            Opcode::CmpEq => (HetKind::Compute(ComputeOp::CmpEq), 0),
            Opcode::CmpLt => (HetKind::Compute(ComputeOp::CmpLt), 0),
            Opcode::CmpLe => (HetKind::Compute(ComputeOp::CmpLe), 0),
            Opcode::Cvt => (HetKind::Compute(ComputeOp::Cvt), 0),
            Opcode::Load => (HetKind::Memory(MemoryOp::Load), 0),
            Opcode::Store => (HetKind::Memory(MemoryOp::Store), 0),
            Opcode::Alloc => (HetKind::Memory(MemoryOp::Alloc), 0),
            Opcode::Call { callee } => (HetKind::Control(ControlOp::Call), callee as i64),
            Opcode::Branch => (HetKind::Control(ControlOp::Branch), 0),
            Opcode::Merge | Opcode::Region => (HetKind::Control(ControlOp::Merge), 0),
            Opcode::Return => (HetKind::Control(ControlOp::Return), 0),
            Opcode::Drop => (HetKind::Control(ControlOp::Call), -1),
            Opcode::EscapePoint => (HetKind::Meta(MetaOp::EscapePoint), 0),
            Opcode::Materialize => (HetKind::Meta(MetaOp::Materialize), 0),
        };
        let addr_space = match kind {
            HetKind::Memory(_) => Some(AddressSpace::Host),
            _ => None,
        };
        let new = HetNode {
            kind,
            ty: node.ty,
            addr_space,
            addr_dst: None,
            sync_scope: None,
            imm,
            block: node.block,
            span: node.span,
        };
        map[i as usize] = Some(out.add_node(new));
    }
    for (ni, node) in son.nodes.iter().enumerate() {
        let from = match map[ni] {
            Some(f) => f,
            None => continue,
        };
        let (data, ctrl) = split_inputs(node.opcode, node.inputs.len());
        for d in data {
            if let Some(&src) = node.inputs.get(d) {
                if let Some(to) = map.get(src.0 as usize).copied().flatten() {
                    let ty = son.nodes[src.0 as usize].ty;
                    out.add_edge(to, from, HetEdgeKind::Data { ty });
                }
            }
        }
        for c in ctrl {
            if let Some(&src) = node.inputs.get(c) {
                if let Some(to) = map.get(src.0 as usize).copied().flatten() {
                    out.add_edge(to, from, HetEdgeKind::Control);
                }
            }
        }
    }
    out.verify().map_err(LowerError::Het)?;
    Ok(out)
}

/// CEP:WHAT: Lowers a Heterogeneous-IR graph to LIR (spec 6.3).
/// CEP:WHY: The backend entry point; every value receives a virtual register with a
/// register class derived from its type, blocks mirror the flattening regions, and every
/// block ends in an explicit control transfer (invariant 4.3.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: LowerError for verifier failures, dead-end blocks, unsupported
/// operations, and blocks with ambiguous successors; never panics.
/// CEP:ASSUMES: the Het graph passed verification; block ids are dense from 0.
/// CEP:COST: O(N + E) with one pass per block; successor extraction is O(E).
/// CEP:EVIDENCE: unit tests rv8_ir::lower_het::tests::*.
pub fn hetir_to_lir(het: &HetIr, name: &str) -> Result<LirFunction, LowerError> {
    het.verify().map_err(LowerError::Het)?;
    // Block universe from node block ids.
    let mut block_count = 0usize;
    for node in &het.nodes {
        match node.block {
            Some(b) => block_count = block_count.max(b.0 as usize + 1),
            None => {
                if !matches!(node.kind, HetKind::Meta(_)) {
                    return Err(LowerError::UnsupportedOp { op: "unblocked node" });
                }
            }
        }
    }
    // Successor map: for every control edge into a merge node, the source block flows
    // to the merge block.
    let mut succ: Vec<Vec<BlockId>> = vec![Vec::new(); block_count];
    for edge in &het.edges {
        if !matches!(edge.kind, HetEdgeKind::Control) {
            continue;
        }
        let (from, to) = match (het.node(edge.from), het.node(edge.to)) {
            (Ok(f), Ok(t)) => (f, t),
            _ => continue,
        };
        if !matches!(to.kind, HetKind::Control(ControlOp::Merge)) {
            continue;
        }
        if let (Some(sb), Some(tb)) = (from.block, to.block) {
            if sb != tb && !succ[sb.0 as usize].contains(&tb) {
                succ[sb.0 as usize].push(tb);
            }
        }
    }
    for list in &mut succ {
        list.sort();
    }
    // Reachability compaction: the flattener assigns one block id per HIR region, and
    // structural regions (a Loop container or an unreachable arm) host no nodes, which
    // would otherwise surface as an unterminated dead block and violate invariant 4.3.3.
    // Keep exactly the blocks reachable from the entry (block 0) and renumber densely;
    // successor lists of kept blocks only contain kept targets by construction.
    let mut reach = vec![false; block_count];
    reach[0] = true;
    let mut stack: Vec<usize> = vec![0];
    while let Some(b) = stack.pop() {
        for &t in &succ[b] {
            if !reach[t.0 as usize] {
                reach[t.0 as usize] = true;
                stack.push(t.0 as usize);
            }
        }
    }
    let mut remap: Vec<Option<BlockId>> = vec![None; block_count];
    let mut next_id = 0u32;
    for (i, &r) in reach.iter().enumerate() {
        if r {
            remap[i] = Some(BlockId(next_id));
            next_id += 1;
        }
    }
    let block_count = next_id as usize;
    for list in &mut succ {
        list.retain(|t| remap[t.0 as usize].is_some());
        for t in list.iter_mut() {
            if let Some(nb) = remap[t.0 as usize] {
                *t = nb;
            }
        }
        list.sort();
    }
    // Reindex the successor table itself: consumers index it by the NEW block ids.
    let mut reindexed: Vec<Vec<BlockId>> = vec![Vec::new(); block_count];
    for (old_id, list) in succ.iter().enumerate() {
        if let Some(nb) = remap[old_id] {
            reindexed[nb.0 as usize] = list.clone();
        }
    }
    succ = reindexed;
    // Value assignment: every value-producing node receives one virtual register.
    let mut func = LirFunction::new(name);
    let mut value_of: Vec<Option<ValueId>> = vec![None; het.nodes.len()];
    for (i, node) in het.nodes.iter().enumerate() {
        let produces = matches!(
            node.kind,
            HetKind::Compute(_)
                | HetKind::Memory(MemoryOp::Load)
                | HetKind::Communication(CommOp::Transfer)
        );
        if produces {
            let class = if node.ty.is_float() {
                RegClassId::Fpr
            } else {
                RegClassId::Gpr
            };
            value_of[i] = Some(func.add_value(node.ty, class));
        }
    }
    // Per-block instruction emission in Het node id order (a per-block subsequence of
    // the global topological order is block-topological).
    let mut blocks: Vec<Vec<LirInstruction>> = vec![Vec::new(); block_count];
    let mut branch_succ: BTreeMap<usize, (BlockId, BlockId)> = BTreeMap::new();
    let mut zero: Option<ValueId> = None;
    for (i, node) in het.nodes.iter().enumerate() {
        let b = match node.block {
            // Nodes of dropped (unreachable) blocks vanish with the block.
            Some(b) => match remap[b.0 as usize] {
                Some(nb) => nb.0 as usize,
                None => continue,
            },
            None => continue,
        };
        let inputs: Vec<usize> = het
            .edges
            .iter()
            .filter(|e| e.to.0 as usize == i)
            .filter_map(|e| match e.kind {
                HetEdgeKind::Data { .. } => Some(e.from.0 as usize),
                _ => None,
            })
            .collect();
        let need_input = |pos: usize, what: &'static str| -> Result<usize, LowerError> {
            inputs
                .get(pos)
                .copied()
                .ok_or(LowerError::UnsupportedOp { op: what })
        };
        let reg = |idx: usize| -> Result<ValueId, LowerError> {
            value_of
                .get(idx)
                .copied()
                .flatten()
                .ok_or(LowerError::UnsupportedOp { op: "missing value" })
        };
        match node.kind {
            HetKind::Compute(op) => match op {
                ComputeOp::Const => {
                    let rv = reg(i)?;
                    blocks[b].push(LirInstruction {
                        opcode: MachineOpcode::MovRegImm32,
                        operands: Vec::new(),
                        result: Some(rv),
                        immediate: node.imm,
                        target_block: None,
                    });
                }
                ComputeOp::Param => {
                    // Parameters are pre-colored by the ABI contract before allocation;
                    // no instruction is emitted at this level.
                }
                // Phi values are delivered by predecessor-edge copies (see the phi
                // lowering pass after the node loop); nothing emits in the merge block.
                ComputeOp::Phi => {}
                ComputeOp::Add
                | ComputeOp::Sub
                | ComputeOp::Mul
                | ComputeOp::And
                | ComputeOp::Or
                | ComputeOp::Xor => {
                    let a = reg(need_input(0, "arity")?)?;
                    let bin = inputs.get(1).copied().map(reg).transpose()?;
                    let opcode = match op {
                        ComputeOp::Add => MachineOpcode::AddRegReg,
                        ComputeOp::Sub => MachineOpcode::SubRegReg,
                        ComputeOp::Mul => MachineOpcode::ImulRegReg,
                        ComputeOp::And => MachineOpcode::AndRegReg,
                        ComputeOp::Or => MachineOpcode::OrRegReg,
                        _ => MachineOpcode::XorRegReg,
                    };
                    blocks[b].push(LirInstruction {
                        opcode,
                        operands: vec![a, bin.unwrap_or(a)],
                        result: Some(reg(i)?),
                        immediate: 0,
                        target_block: None,
                    });
                }
                ComputeOp::CmpEq | ComputeOp::CmpLt | ComputeOp::CmpLe => {
                    let a = reg(need_input(0, "arity")?)?;
                    let bin = inputs.get(1).copied().map(reg).transpose()?;
                    let kind = match op {
                        ComputeOp::CmpEq => SetKind::Eq,
                        ComputeOp::CmpLt => SetKind::Lt,
                        _ => SetKind::Le,
                    };
                    blocks[b].push(LirInstruction {
                        opcode: MachineOpcode::CmpRegReg,
                        operands: vec![a, bin.unwrap_or(a)],
                        result: None,
                        immediate: 0,
                        target_block: None,
                    });
                    blocks[b].push(LirInstruction {
                        opcode: MachineOpcode::Setcc(kind),
                        operands: Vec::new(),
                        result: Some(reg(i)?),
                        immediate: 0,
                        target_block: None,
                    });
                }
                ComputeOp::Neg
                | ComputeOp::Not
                | ComputeOp::Div
                | ComputeOp::Rem
                | ComputeOp::Shl
                | ComputeOp::Shr
                | ComputeOp::Cvt => {
                    return Err(LowerError::UnsupportedOp { op: "unlowered compute op" });
                }
            },
            HetKind::Memory(MemoryOp::Load) => {
                let base = reg(need_input(0, "load addr")?)?;
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::MovRegMem,
                    operands: vec![base],
                    result: Some(reg(i)?),
                    immediate: 0,
                    target_block: None,
                });
            }
            HetKind::Memory(MemoryOp::Store) => {
                let addr = reg(need_input(0, "store addr")?)?;
                let val = reg(need_input(1, "store value")?)?;
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::MovMemReg,
                    operands: vec![addr, val],
                    result: None,
                    immediate: 0,
                    target_block: None,
                });
            }
            HetKind::Memory(_) => {
                return Err(LowerError::UnsupportedOp { op: "alloc/prefetch" });
            }
            HetKind::Control(ControlOp::Branch) => {
                // test cond,0 + jne lower-numbered successor; the other arm falls
                // through. The zero constant is materialized once, in the entry block.
                let z = match zero {
                    Some(zv) => zv,
                    None => {
                        let zv = func.add_value(IrType::I64, RegClassId::Gpr);
                        blocks[0].insert(
                            0,
                            LirInstruction {
                                opcode: MachineOpcode::MovRegImm32,
                                operands: Vec::new(),
                                result: Some(zv),
                                immediate: 0,
                                target_block: None,
                            },
                        );
                        zero = Some(zv);
                        zv
                    }
                };
                let cond = reg(need_input(0, "branch cond")?)?;
                // Branch convention: the LOWER-numbered successor is the TRUE arm.
                // Machine verification requires the fallthrough arm to be the
                // layout-next block, so when the true arm is layout-next the branch
                // condition is inverted (jump-if-false targets the false arm) and the
                // true arm falls through; semantics are identical.
                let layout_next = BlockId((b + 1) as u32);
                let (kind, target) = match succ[b].as_slice() {
                    [t, f] if *f == layout_next => (SetKind::Ne, *t),
                    [t, f] if *t == layout_next => (SetKind::Eq, *f),
                    [t, _] => (SetKind::Ne, *t),
                    _ => {
                        return Err(LowerError::UnsupportedOp {
                            op: "branch without two successors",
                        })
                    }
                };
                // Encoder convention: block successors are [taken, fallthrough] with
                // the fallthrough equal to the layout-next block (spec 5.3 invariant 3).
                let other = if succ[b][0] == target { succ[b][1] } else { succ[b][0] };
                branch_succ.insert(b, (target, other));
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::CmpRegReg,
                    operands: vec![cond, z],
                    result: None,
                    immediate: 0,
                    target_block: None,
                });
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::Jcc(kind),
                    operands: Vec::new(),
                    result: None,
                    immediate: 0,
                    target_block: Some(target),
                });
            }
            HetKind::Control(ControlOp::Return) => {
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::Ret,
                    operands: Vec::new(),
                    result: None,
                    immediate: 0,
                    target_block: None,
                });
            }
            HetKind::Control(ControlOp::Call) => {
                let operands: Vec<ValueId> =
                    inputs.iter().copied().map(reg).collect::<Result<_, _>>()?;
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::Call,
                    operands,
                    result: None,
                    immediate: node.imm,
                    target_block: None,
                });
            }
            HetKind::Control(ControlOp::Merge) | HetKind::Control(ControlOp::Loop) => {
                // Labels, not instructions.
            }
            HetKind::Sync(_) => {
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::Fence,
                    operands: Vec::new(),
                    result: None,
                    immediate: node.imm,
                    target_block: None,
                });
            }
            HetKind::Communication(_) => {
                // Spec 6.3: communication becomes explicit data movement; address-space
                // placement is a backend and autotuner decision downstream.
                let src = reg(need_input(0, "transfer src")?)?;
                blocks[b].push(LirInstruction {
                    opcode: MachineOpcode::MovRegReg,
                    operands: vec![src],
                    result: Some(reg(i)?),
                    immediate: 0,
                    target_block: None,
                });
            }
            HetKind::Meta(_) => {
                // Autotuner annotations are consumed at Level 2 and erased before LIR.
            }
        }
    }
    // Phi lowering pass: each phi [v1..vk] at a merge with control inputs [c1..ck]
    // places `mov phi, vj` at the END of predecessor block cj on the same positional
    // path (spec 6.3). LIR 4.3.1 relaxes single-definition for exactly these values.
    // Constraints enforced: the merge lives in the phi block; each predecessor block
    // has exactly one successor (no copies on branch edges); swap-shaped copy sets
    // break through one scratch register (deterministic ready-order sequencing).
    {
        let reg_of = |idx: usize| -> Result<ValueId, LowerError> {
            value_of
                .get(idx)
                .copied()
                .flatten()
                .ok_or(LowerError::UnsupportedOp { op: "missing value" })
        };
        let mut per_pred: BTreeMap<usize, Vec<(ValueId, ValueId)>> = BTreeMap::new();
        for (i, node) in het.nodes.iter().enumerate() {
            if !matches!(node.kind, HetKind::Compute(ComputeOp::Phi)) {
                continue;
            }
            let b = match node.block {
                Some(x) => x.0 as usize,
                None => return Err(LowerError::UnsupportedOp { op: "phi without block" }),
            };
            let merge_index = het
                .nodes
                .iter()
                .position(|n| {
                    matches!(n.kind, HetKind::Control(ControlOp::Merge))
                        && n.block == Some(BlockId(b as u32))
                })
                .ok_or(LowerError::UnsupportedOp { op: "phi without merge" })?;
            let ctrl_inputs: Vec<usize> = het
                .edges
                .iter()
                .filter(|e| e.to.0 as usize == merge_index)
                .filter(|e| matches!(e.kind, HetEdgeKind::Control))
                .map(|e| e.from.0 as usize)
                .collect();
            let data_inputs: Vec<usize> = het
                .edges
                .iter()
                .filter(|e| e.to.0 as usize == i)
                .filter(|e| matches!(e.kind, HetEdgeKind::Data { .. }))
                .map(|e| e.from.0 as usize)
                .collect();
            if ctrl_inputs.len() != data_inputs.len() || data_inputs.is_empty() {
                return Err(LowerError::UnsupportedOp { op: "phi/control arity mismatch" });
            }
            let dst = reg_of(i)?;
            for (cj, vi) in ctrl_inputs.iter().zip(data_inputs.iter()) {
                let pb = het.nodes[*cj].block.ok_or(LowerError::UnsupportedOp {
                    op: "phi predecessor without block",
                })?;
                // Predecessor blocks are in the pre-compaction id space; map through
                // the compaction remap before consulting the successor table.
                let pbi = match remap[pb.0 as usize] {
                    Some(nb) => nb.0 as usize,
                    None => {
                        return Err(LowerError::UnsupportedOp {
                            op: "phi predecessor in unreachable block",
                        })
                    }
                };
                if pbi == b {
                    return Err(LowerError::UnsupportedOp { op: "phi edge within one block" });
                }
                if succ[pbi].len() != 1 {
                    return Err(LowerError::UnsupportedOp { op: "phi on a branch edge" });
                }
                let src = reg_of(*vi)?;
                per_pred.entry(pbi).or_default().push((dst, src));
            }
        }
        for (pbi, copies) in per_pred {
            let mut pending: Vec<(ValueId, ValueId)> =
                copies.into_iter().filter(|(d, s)| d != s).collect();
            let mut scratch: Option<ValueId> = None;
            let mut forced_last: Option<ValueId> = None;
            let mut sequenced: Vec<(ValueId, ValueId)> = Vec::new();
            while let Some(k) = pending
                .iter()
                .position(|(_, s)| !pending.iter().any(|(d, _)| d == s))
            {
                let c = pending.remove(k);
                sequenced.push(c);
            }
            while !pending.is_empty() {
                // Pure cycle: save the first source in a scratch register, redirect,
                // and deliver the saved value to its destination after everything else.
                let s = *scratch.get_or_insert_with(|| {
                    func.add_value(IrType::I64, RegClassId::Gpr)
                });
                let (d0, s0) = pending[0];
                sequenced.push((s, s0));
                forced_last = Some(d0);
                pending.remove(0);
                while let Some(k) = pending
                    .iter()
                    .position(|(_, s)| !pending.iter().any(|(d, _)| d == s))
                {
                    let c = pending.remove(k);
                    sequenced.push(c);
                }
            }
            if let Some(d0) = forced_last {
                sequenced.push((d0, scratch.expect("scratch set with forced_last")));
            }
            for (d, s) in sequenced {
                blocks[pbi].push(LirInstruction {
                    opcode: MachineOpcode::MovRegReg,
                    operands: vec![s],
                    result: Some(d),
                    immediate: 0,
                    target_block: None,
                });
            }
        }
    }
    // Assemble blocks with terminator synthesis (invariant 4.3.3) and use lists.
    for (bi, instrs) in blocks.into_iter().enumerate() {
        let mut list = instrs;
        let ends_with_transfer = list
            .last()
            .map(|li| {
                matches!(
                    li.opcode,
                    MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret
                )
            })
            .unwrap_or(false);
        if !ends_with_transfer {
            match succ[bi].len() {
                1 => list.push(LirInstruction {
                    opcode: MachineOpcode::Jmp,
                    operands: Vec::new(),
                    result: None,
                    immediate: 0,
                    target_block: Some(succ[bi][0]),
                }),
                0 => return Err(LowerError::UnsupportedOp { op: "dead-end block" }),
                _ => {
                    return Err(LowerError::UnsupportedOp {
                        op: "multi-successor without branch",
                    })
                }
            }
        }
        let mut block = LirBlock::new(BlockId(bi as u32));
        block.successors = match branch_succ.get(&bi) {
            Some((taken, fallthrough)) => vec![*taken, *fallthrough],
            None => succ[bi].clone(),
        };
        for (ii, instr) in list.into_iter().enumerate() {
            for op in &instr.operands {
                if let Some(v) = func.values.get_mut(op.0 as usize) {
                    v.uses.push((BlockId(bi as u32), ii as u32));
                }
            }
            block.instructions.push(instr);
        }
        func.push_block(block);
    }
    // Symmetric predecessor edges.
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); func.blocks.len()];
    for (bi, block) in func.blocks.iter().enumerate() {
        for &s in &block.successors {
            if (s.0 as usize) < preds.len() && !preds[s.0 as usize].contains(&(bi as u32)) {
                preds[s.0 as usize].push(bi as u32);
            }
        }
    }
    for (bi, p) in preds.into_iter().enumerate() {
        func.blocks[bi].predecessors = p.into_iter().map(BlockId).collect();
    }
    Ok(func)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::son::{SeaNode, SeaOfNodes};
    use rv8_core::span::Span;

    /// CEP:WHAT: Blank span for tests.
    /// CEP:WHY: Keeps test bodies readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn sp() -> Span {
        Span::new(rv8_core::span::FileId(0), 0, 0)
    }

    /// CEP:WHAT: Adds a node with blank provenance.
    /// CEP:WHY: Tests control block/span fields explicitly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn add(son: &mut SeaOfNodes, opcode: Opcode, ty: IrType, inputs: Vec<u32>, block: u32) -> u32 {
        let id = son
            .add_node(opcode, ty, inputs.into_iter().map(crate::son::NodeId).collect());
        son.nodes[id.0 as usize].block = Some(BlockId(block));
        son.nodes[id.0 as usize].span = sp();
        id.0
    }

    /// CEP:WHAT: Linearizes const 1 + const 2 feeding a return.
    /// CEP:WHY: Data edges are preserved; control becomes explicit.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn linearize_straight_line() {
        let mut son = SeaOfNodes::new();
        let a = add(&mut son, Opcode::Const(1), IrType::I32, vec![], 0);
        let b = add(&mut son, Opcode::Const(2), IrType::I32, vec![], 0);
        let s = add(&mut son, Opcode::Add, IrType::I32, vec![a, b], 0);
        let r = add(&mut son, Opcode::Region, IrType::Unit, vec![], 0);
        let ret = add(&mut son, Opcode::Return, IrType::Unit, vec![s, r], 0);
        let het = son_to_hetir(&son).expect("linearize");
        assert!(het.verify().is_ok());
        assert_eq!(het.nodes.len(), 5, "all live nodes map");
        let _ = ret;
        // Data edges preserved: sum has two data inputs.
        let data_in = het
            .edges
            .iter()
            .filter(|e| {
                matches!(e.kind, HetEdgeKind::Data { .. })
                    && het.node(e.to).map(|n| n.kind)
                        == Ok(HetKind::Compute(ComputeOp::Add))
            })
            .count();
        assert_eq!(data_in, 2);
    }

    /// CEP:WHAT: An unused pure node is dropped by linearization.
    /// CEP:WHY: Invariant 3.3.4 holds by construction.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn linearize_drops_dead_nodes() {
        let mut son = SeaOfNodes::new();
        let _dead = add(&mut son, Opcode::Const(9), IrType::I32, vec![], 0);
        let a = add(&mut son, Opcode::Const(1), IrType::I32, vec![], 0);
        let r = add(&mut son, Opcode::Region, IrType::Unit, vec![], 0);
        let _ret = add(&mut son, Opcode::Return, IrType::Unit, vec![a, r], 0);
        let het = son_to_hetir(&son).expect("linearize with dead node");
        assert_eq!(het.nodes.len(), 3, "dead const skipped");
    }

    /// CEP:WHAT: The full 6.3 lowering of an if/else function produces blocks with
    /// terminators, a Jcc, and typed virtual registers.
    /// CEP:WHY: Invariants 4.3.2 and 4.3.3 at the LIR boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_if_else_to_lir() {
        let mut son = SeaOfNodes::new();
        // block 0: entry; const c; branch(c, r0)
        let c = add(&mut son, Opcode::Const(1), IrType::Bool, vec![], 0);
        let r0 = add(&mut son, Opcode::Region, IrType::Unit, vec![], 0);
        let br = add(&mut son, Opcode::Branch, IrType::Unit, vec![c, r0], 0);
        // block 1 (then): const 10; merge
        let t10 = add(&mut son, Opcode::Const(10), IrType::I32, vec![], 1);
        let r1 = add(&mut son, Opcode::Merge, IrType::Unit, vec![br], 1);
        // block 2 (else): const 20; merge
        let t20 = add(&mut son, Opcode::Const(20), IrType::I32, vec![], 2);
        let r2 = add(&mut son, Opcode::Merge, IrType::Unit, vec![br], 2);
        // block 3: phi(10, 20); return
        let phi = add(&mut son, Opcode::Phi, IrType::I32, vec![t10, t20], 3);
        let r3 = add(&mut son, Opcode::Merge, IrType::Unit, vec![r1, r2], 3);
        let _ret = add(&mut son, Opcode::Return, IrType::Unit, vec![phi, r3], 3);
        let het = son_to_hetir(&son).expect("linearize if/else");
        let lir = hetir_to_lir(&het, "f").expect("lower to lir");
        // Every block ends in a transfer.
        for block in &lir.blocks {
            let last = block.instructions.last().expect("nonempty block");
            assert!(matches!(
                last.opcode,
                MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret
            ));
        }
        // The true arm (block 1) is the layout-next block, so the branch inverts:
        // Jcc(Eq) jumps to the else arm (2) when the condition is false, and the
        // then arm falls through. Semantics are identical to a non-inverted branch.
        let jcc = lir.blocks[0]
            .instructions
            .iter()
            .find(|li| matches!(li.opcode, MachineOpcode::Jcc(_)))
            .expect("jcc present");
        assert_eq!(jcc.target_block, Some(BlockId(2)));
        // Successors are [taken, fallthrough] with the fallthrough at layout order.
        assert_eq!(lir.blocks[0].successors, vec![BlockId(2), BlockId(1)]);
        // Successors are wired symmetrically.
        assert!(lir.blocks[3].predecessors.contains(&BlockId(1)));
        assert!(lir.blocks[3].predecessors.contains(&BlockId(2)));
    }

    /// CEP:WHAT: Linearization and LIR lowering are deterministic across runs.
    /// CEP:WHY: 38.10 and 38.19.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lower_het_is_deterministic() {
        let build = || {
            let mut son = SeaOfNodes::new();
            let a = add(&mut son, Opcode::Const(7), IrType::I32, vec![], 0);
            let b = add(&mut son, Opcode::Const(8), IrType::I32, vec![], 0);
            let s = add(&mut son, Opcode::Mul, IrType::I32, vec![a, b], 0);
            let r = add(&mut son, Opcode::Region, IrType::Unit, vec![], 0);
            add(&mut son, Opcode::Return, IrType::Unit, vec![s, r], 0);
            son
        };
        let h1 = son_to_hetir(&build()).expect("run 1");
        let h2 = son_to_hetir(&build()).expect("run 2");
        assert_eq!(h1.canonical_hash(), h2.canonical_hash());
        let l1 = hetir_to_lir(&h1, "d").expect("lir 1");
        let l2 = hetir_to_lir(&h2, "d").expect("lir 2");
        assert_eq!(format!("{:?}", l1), format!("{:?}", l2));
    }

    /// CEP:WHAT: A SeaNode reference keeps working after the helper additions.
    /// CEP:WHY: Guards the test helper signature.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn sea_node_helper_consistent() {
        let mut son = SeaOfNodes::new();
        let id = add(&mut son, Opcode::Const(3), IrType::I64, vec![], 0);
        let node: &SeaNode = &son.nodes[id as usize];
        assert!(matches!(node.opcode, Opcode::Const(3)));
    }
}
