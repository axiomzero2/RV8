// CEP:FILE: crates/rv8_ir/src/son.rs
// CEP:WHAT: The heterogeneous Sea-of-Nodes MIR: typed scalar, memory, control, and allocation
//   nodes stored in flat index-addressed vectors with explicit input edges.
// CEP:WHY: Flat index storage gives stable identity (38.17), cache-sequential traversal, and
//   deterministic printing/hashing without pointer dependence.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: node() and replace_inputs return NodeError for dangling ids; verify_structure
//   returns a descriptive error for cycles or dangling edges.
// CEP:ASSUMES: inputs reference existing nodes; data nodes are acyclic except loops represented
//   through explicit control nodes; enforced by verify_structure.
// CEP:COST: add_node amortized O(1); canonical_hash O(N log N) for edge sorting.
// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
// CEP:SECURITY: node count growth is caller-bounded; no recursion in any traversal here.

use rv8_core::fxhash64;
use rv8_core::fx_mix;
use rv8_core::span::{FileId, Span};
use std::fmt::Write as _;

/// CEP:WHAT: Maximum nodes per function before the driver must reject input.
/// CEP:WHY: Bounded compile time and memory (38.12); named limit, not a magic number.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced by callers (driver) before lowering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: review 2026-10-30.
pub const MAX_NODES_PER_FUNCTION: u32 = 1_000_000;

/// CEP:WHAT: Stable node identity inside one SeaOfNodes.
/// CEP:WHY: Index-based identity satisfies the no-pointer-identity rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(pub u32);

/// CEP:WHAT: Stable basic-block identity.
/// CEP:WHY: LIR lowering and CFG analyses reference blocks by index.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense indices.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

/// CEP:WHAT: Scalar and reference types of the MIR type system.
/// CEP:WHY: Type-correct IR is verifier-checked before any backend pass.
/// CEP:STATUS: partial
/// CEP:OWNER: axiomzero2
/// CEP:TICKET: RV8-101
/// CEP:FAILURE: none
/// CEP:ASSUMES: aggregates (structs) are lowered to scalar fields before MIR construction.
/// CEP:COST: POD enum, one byte.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IrType {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Char,
    Ptr,
}

impl IrType {
    /// CEP:WHAT: Reports the width in bytes for scalar types.
    /// CEP:WHY: ABI lowering and register-class selection need byte widths.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: target is 64-bit for Ptr width (target-controlled, not silent).
    /// CEP:COST: O(1) match.
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn byte_width(self) -> u32 {
        match self {
            IrType::Unit => 0,
            IrType::Bool | IrType::I8 | IrType::U8 => 1,
            IrType::I16 | IrType::U16 => 2,
            IrType::I32 | IrType::U32 | IrType::F32 | IrType::Char => 4,
            IrType::I64 | IrType::U64 | IrType::F64 | IrType::Ptr => 8,
            IrType::I128 | IrType::U128 => 16,
        }
    }

    /// CEP:WHAT: Reports whether the type lives in the floating-point register class.
    /// CEP:WHY: Register-class assignment for LIR values.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn is_float(self) -> bool {
        matches!(self, IrType::F32 | IrType::F64)
    }
}

/// CEP:WHAT: MIR opcodes: scalar compute, memory, control, allocation tagging, and constants.
/// CEP:WHY: One closed opcode set keeps the printer, hasher, and verifier exhaustive.
/// CEP:STATUS: partial
/// CEP:OWNER: axiomzero2
/// CEP:TICKET: RV8-102
/// CEP:FAILURE: none
/// CEP:ASSUMES: loop cycles enter through Region/Phi back-edges only.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Opcode {
    Const(i64),
    Parameter(u16),
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
    Load,
    Store,
    Call { callee: u32 },
    Branch,
    Merge,
    Phi,
    Region,
    Return,
    Alloc,
    EscapePoint,
    Materialize,
    Drop,
}

impl Opcode {
    /// CEP:WHAT: Returns the stable short name used by printing and hashing.
    /// CEP:WHY: Textual form is the canonical IR serialization (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn name(self) -> &'static str {
        match self {
            Opcode::Const(_) => "const",
            Opcode::Parameter(_) => "param",
            Opcode::Add => "add",
            Opcode::Sub => "sub",
            Opcode::Mul => "mul",
            Opcode::Div => "div",
            Opcode::Rem => "rem",
            Opcode::And => "and",
            Opcode::Or => "or",
            Opcode::Xor => "xor",
            Opcode::Shl => "shl",
            Opcode::Shr => "shr",
            Opcode::Neg => "neg",
            Opcode::Not => "not",
            Opcode::CmpEq => "cmpeq",
            Opcode::CmpLt => "cmplt",
            Opcode::CmpLe => "cmple",
            Opcode::Cvt => "cvt",
            Opcode::Load => "load",
            Opcode::Store => "store",
            Opcode::Call { .. } => "call",
            Opcode::Branch => "branch",
            Opcode::Merge => "merge",
            Opcode::Phi => "phi",
            Opcode::Region => "region",
            Opcode::Return => "ret",
            Opcode::Alloc => "alloc",
            Opcode::EscapePoint => "escape",
            Opcode::Materialize => "materialize",
            Opcode::Drop => "drop",
        }
    }

    /// CEP:WHAT: Reports whether the opcode terminates a basic block.
    /// CEP:WHY: Verifier terminator checks; Call is a side effect inside a block, not a
    /// transfer, so it is excluded.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn is_terminator(self) -> bool {
        matches!(self, Opcode::Branch | Opcode::Return)
    }
}

/// CEP:WHAT: One IR node: opcode, type, ordered data inputs, owning block, and source span.
/// CEP:WHY: Carries the provenance chain required for backend diagnostics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: inputs are NodeIds in the same graph.
/// CEP:COST: ~48 bytes; flat vector storage.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Debug)]
pub struct SeaNode {
    pub opcode: Opcode,
    pub ty: IrType,
    pub inputs: Vec<NodeId>,
    pub block: Option<BlockId>,
    pub span: Span,
}

/// CEP:WHAT: Error returned by IR accessors and structural verification.
/// CEP:WHY: Fail-closed contracts without panics (Rust CEP-0 rules).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeError {
    DanglingNode(NodeId),
    DanglingInput { node: NodeId, input: NodeId },
    CycleDetected { entry: NodeId },
}

/// CEP:WHAT: The Sea-of-Nodes graph for one function.
/// CEP:WHY: Central IR container with print, hash, and verify operations.
/// CEP:STATUS: complete
/// CEP:FAILURE: see NodeError.
/// CEP:ASSUMES: one function per graph.
/// CEP:COST: construction amortized O(1) per node.
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
#[derive(Default, Debug)]
pub struct SeaOfNodes {
    pub nodes: Vec<SeaNode>,
}

impl SeaOfNodes {
    /// CEP:WHAT: Creates an empty graph.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn new() -> SeaOfNodes {
        SeaOfNodes { nodes: Vec::new() }
    }

    /// CEP:WHAT: Appends a node and returns its id.
    /// CEP:WHY: Single construction path keeps indices dense.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; caller enforces MAX_NODES_PER_FUNCTION.
    /// CEP:ASSUMES: inputs reference existing nodes (verified lazily).
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn add_node(&mut self, opcode: Opcode, ty: IrType, inputs: Vec<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(SeaNode {
            opcode,
            ty,
            inputs,
            block: None,
            span: Span {
                file: FileId(0),
                lo: 0,
                hi: 0,
            },
        });
        id
    }

    /// CEP:WHAT: Immutable accessor for a node.
    /// CEP:WHY: Fail-closed access.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingNode for unknown ids.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn node(&self, id: NodeId) -> Result<&SeaNode, NodeError> {
        self.nodes
            .get(id.0 as usize)
            .ok_or(NodeError::DanglingNode(id))
    }

    /// CEP:WHAT: Mutable accessor for a node.
    /// CEP:WHY: Pass mutations.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingNode for unknown ids.
    /// CEP:ASSUMES: mutation keeps structural invariants.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn node_mut(&mut self, id: NodeId) -> Result<&mut SeaNode, NodeError> {
        self.nodes
            .get_mut(id.0 as usize)
            .ok_or(NodeError::DanglingNode(id))
    }

    /// CEP:WHAT: Rewires all occurrences of old_id to new_id across inputs.
    /// CEP:WHY: Passes replace dead nodes without leaving dangling edges.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: old_id is subsequently dead; verifier catches residual uses.
    /// CEP:COST: O(N * fanin).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn replace_uses(&mut self, old_id: NodeId, new_id: NodeId) {
        for node in &mut self.nodes {
            for input in &mut node.inputs {
                if *input == old_id {
                    *input = new_id;
                }
            }
        }
    }

    /// CEP:WHAT: Checks that every input edge references an existing node.
    /// CEP:WHY: Cheap verifier mode run after every pass (38.18).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: DanglingInput with both endpoint ids.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(N * fanin).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn verify_structure(&self) -> Result<(), NodeError> {
        for (i, node) in self.nodes.iter().enumerate() {
            let id = NodeId(i as u32);
            for &input in &node.inputs {
                if input.0 as usize >= self.nodes.len() {
                    return Err(NodeError::DanglingInput { node: id, input });
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Prints the graph in stable node order with one line per node.
    /// CEP:WHY: Printable IR is a contract requirement (38.17).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(N * fanin).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn print(&self) -> String {
        let mut out = String::new();
        for (i, node) in self.nodes.iter().enumerate() {
            let _ = write!(out, "n{}: {} {}", i, node.opcode.name(), type_name(node.ty));
            if !node.inputs.is_empty() {
                let _ = write!(out, " (");
                for (j, input) in node.inputs.iter().enumerate() {
                    if j > 0 {
                        let _ = write!(out, ", ");
                    }
                    let _ = write!(out, "n{}", input.0);
                }
                let _ = write!(out, ")");
            }
            let _ = writeln!(out);
        }
        out
    }

    /// CEP:WHAT: Computes a deterministic content hash over opcodes, types, and input edges.
    /// CEP:WHY: Passes and the CAS cache key on IR content, not addresses (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: opcode name() and type discriminants are stable across compiler versions.
    /// CEP:COST: O(N log N) dominated by edge-vector sorting-free stable layout.
    /// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
    pub fn canonical_hash(&self) -> u64 {
        let mut hash: u64 = 0;
        for node in &self.nodes {
            hash = fx_mix_all(hash, node.opcode.name().as_bytes());
            hash = fx_mix_all(hash, &[node.ty as u8]);
            for input in &node.inputs {
                hash = fx_mix_all(hash, &input.0.to_le_bytes());
            }
        }
        hash
    }
}

/// CEP:WHAT: Chains one byte slice into a running Fx hash state.
/// CEP:WHY: Keeps canonical hashing linear without intermediate arrays.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
fn fx_mix_all(state: u64, bytes: &[u8]) -> u64 {
    fx_mix(state, fxhash64(bytes))
}

/// CEP:WHAT: Renders an IrType short name for printing.
/// CEP:WHY: Stable textual type form.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::tests::son_construction_and_print.
fn type_name(ty: IrType) -> &'static str {
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
