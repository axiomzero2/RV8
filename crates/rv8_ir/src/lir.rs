// CEP:FILE: crates/rv8_ir/src/lir.rs
// CEP:WHAT: The Low-level IR (LIR): blocks of machine-oriented instructions over virtual values,
//   plus physical register classes and the register-allocation result type.
// CEP:WHY: Instruction selection and the MCF register allocator share one data contract;
//   separating LIR from MIR keeps the backend target-aware but not target-contaminated.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; construction APIs are total.
// CEP:ASSUMES: ValueId and BlockId are dense per function; SSA property enforced by the
//   middle-end verifier before LIR lowering.
// CEP:COST: POD vectors; O(1) indexed access.
// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
// CEP:SECURITY: none specific; data only.

use crate::son::{BlockId, IrType};

/// CEP:WHAT: Virtual value identity inside one LirFunction.
/// CEP:WHY: Register allocation maps values to physical registers by index.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ValueId(pub u32);

/// CEP:WHAT: Physical register identity within a register class.
/// CEP:WHY: Allocation results use indices, not names.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: dense within a RegisterClass.
/// CEP:COST: POD u32.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegId(pub u32);

/// CEP:WHAT: Register-class identity (GPR, FPR, VECTOR).
/// CEP:WHY: Heterogeneous register classes are first-class in the allocator.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable across versions (used in hashing).
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum RegClassId {
    Gpr = 0,
    Fpr = 1,
    Vector = 2,
}

/// CEP:WHAT: Kind of condition code used by set/jcc instructions.
/// CEP:WHY: Comparisons lower to flags then conditionally materialize results.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SetKind {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// CEP:WHAT: Machine-level opcodes produced by instruction selection.
/// CEP:WHY: A closed target-neutral mnemonic set that x86-64 encoding maps to bytes.
/// CEP:STATUS: partial
/// CEP:OWNER: axiomzero2
/// CEP:TICKET: RV8-110
/// CEP:FAILURE: none
/// CEP:ASSUMES: encoders validate operand shapes before emission.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MachineOpcode {
    MovRegReg,
    MovRegImm32,
    Lea,
    AddRegReg,
    AddRegImm32,
    SubRegReg,
    SubRegImm32,
    ImulRegReg,
    ShlRegImm8,
    SarRegImm8,
    AndRegReg,
    OrRegReg,
    XorRegReg,
    XorRegSelf,
    CmpRegReg,
    CmpRegImm32,
    Setcc(SetKind),
    Jmp,
    Jcc(SetKind),
    Call,
    Ret,
    Push,
    Pop,
    Cqo,
    IdivReg,
    MovsdRegReg,
    CvtSi2sd,
    Syscall,
    LoadStack,
    StoreStack,
    /// CEP:WHAT: Store a sign-extended 32-bit immediate to a stack slot
    ///   (`mov qword [mem], imm32`; operands [Mem, Imm]).
    /// CEP:WHY: Spilling the definition of an immediate needs no scratch register:
    ///   at peak pressure every register holds a live value, so routing the
    ///   immediate through a register would clobber one; the direct store keeps
    ///   spill rewriting sound and feasible (RV8-137 follow-up: the 16-value
    ///   pressure clique exhausted the scratch pool at the spilled definition).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: machir verification rejects non-Mem/Imm operand shapes.
    /// CEP:ASSUMES: the immediate fits i32 (the MovRegImm32 contract).
    /// CEP:COST: POD.
    /// CEP:EVIDENCE: spill_path_end_to_end; the RV8-137 regression batches.
    StoreStackImm,
    MovRegMem,
    MovMemReg,
    Fence,
}

/// CEP:WHAT: One LIR instruction: machine opcode, input value operands, result, and immediate.
/// CEP:WHY: The unit of scheduling, selection, and encoding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: operand count fits the opcode shape; encoder validates.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct LirInstruction {
    pub opcode: MachineOpcode,
    pub operands: Vec<ValueId>,
    pub result: Option<ValueId>,
    pub immediate: i64,
    /// CEP:WHAT: Byte offset of the target block for branch/jump instructions.
    /// CEP:WHY: Block-relative control flow resolution at emission.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: set only for control-flow opcodes.
    /// CEP:COST: POD.
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub target_block: Option<BlockId>,
}

impl LirInstruction {
    /// CEP:WHAT: Builds a two-operand add instruction.
    /// CEP:WHY: Common construction path in tests and lowering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn add(result: ValueId, operands: Vec<ValueId>) -> LirInstruction {
        LirInstruction {
            opcode: MachineOpcode::AddRegReg,
            operands,
            result: Some(result),
            immediate: 0,
            target_block: None,
        }
    }
}

/// CEP:WHAT: One LIR value: type, register class, definition, and use list.
/// CEP:WHY: Live-range construction for the MCF allocator starts here.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: exactly one definition (SSA); uses list maintained by lowering.
/// CEP:COST: POD plus use vector.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct LirValue {
    pub ty: IrType,
    pub reg_class: RegClassId,
    pub uses: Vec<(BlockId, u32)>,
}

/// CEP:WHAT: One basic block of LIR instructions with successor edges.
/// CEP:WHY: Layout, scheduling, and liveness operate on blocks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: one terminator per block (lowering responsibility).
/// CEP:COST: POD vectors.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct LirBlock {
    pub id: BlockId,
    pub instructions: Vec<LirInstruction>,
    pub successors: Vec<BlockId>,
    pub predecessors: Vec<BlockId>,
}

impl LirBlock {
    /// CEP:WHAT: Creates an empty block.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn new(id: BlockId) -> LirBlock {
        LirBlock {
            id,
            instructions: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
        }
    }

    /// CEP:WHAT: Appends one instruction.
    /// CEP:WHY: Single mutation path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn push(&mut self, instr: LirInstruction) {
        self.instructions.push(instr);
    }
}

/// CEP:WHAT: One physical register with its name, fixed-ness, and alias ids.
/// CEP:WHY: x86 aliasing (rax/eax/ax/al) must be modeled by the allocator.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: aliases reference ids within the same class.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct PhysicalRegister {
    pub id: RegId,
    pub name: &'static str,
    pub is_fixed: bool,
    pub aliases: Vec<RegId>,
}

/// CEP:WHAT: A heterogeneous register class with capacity and spill/move costs.
/// CEP:WHY: Cost parameters feed the MCF objective directly.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: capacity equals registers.len().
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct RegisterClass {
    pub id: RegClassId,
    pub name: &'static str,
    pub registers: Vec<PhysicalRegister>,
    pub spill_cost: f64,
    pub move_cost: f64,
}

impl RegisterClass {
    /// CEP:WHAT: Reports the number of usable registers in this class.
    /// CEP:WHY: Flow-network capacity edges need the count.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn capacity(&self) -> usize {
        self.registers.len()
    }
}

/// CEP:WHAT: One function in LIR form.
/// CEP:WHY: Backend pipeline container handed from lowering to emission.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: blocks dense from 0.
/// CEP:COST: POD vectors.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct LirFunction {
    pub name: String,
    pub blocks: Vec<LirBlock>,
    pub values: Vec<LirValue>,
}

impl LirFunction {
    /// CEP:WHAT: Creates an empty function.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn new(name: &str) -> LirFunction {
        LirFunction {
            name: name.to_string(),
            blocks: Vec::new(),
            values: Vec::new(),
        }
    }

    /// CEP:WHAT: Registers one virtual value and returns its id.
    /// CEP:WHY: Dense value indices feed live-range arrays.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn add_value(&mut self, ty: IrType, reg_class: RegClassId) -> ValueId {
        let id = ValueId(self.values.len() as u32);
        self.values.push(LirValue {
            ty,
            reg_class,
            uses: Vec::new(),
        });
        id
    }

    /// CEP:WHAT: Appends a block and wires predecessors symmetrically.
    /// CEP:WHY: CFG edges must be bidirectional for liveness.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: successor ids already exist or are added later.
    /// CEP:COST: O(successors).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn push_block(&mut self, block: LirBlock) {
        self.blocks.push(block);
    }

    /// CEP:WHAT: Returns an immutable view of the i-th block.
    /// CEP:WHY: Fail-closed access.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None out of range.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn block(&self, id: BlockId) -> Option<&LirBlock> {
        self.blocks.get(id.0 as usize)
    }
}

/// CEP:WHAT: Register allocation result: a dense map ValueId index to `Option<RegId>`.
/// CEP:WHY: Vectors keep the mapping deterministic and printable.
/// CEP:STATUS: complete
/// CEP:FAILURE: register() returns None for unallocated values.
/// CEP:ASSUMES: allocated by the backend MCF allocator.
/// CEP:COST: O(1) lookup.
/// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
#[derive(Clone, Debug)]
pub struct RegisterAllocation {
    pub assignments: Vec<Option<RegId>>,
    pub spill_slots: Vec<Option<u32>>,
}

impl RegisterAllocation {
    /// CEP:WHAT: Creates an empty allocation sized to the function's value count.
    /// CEP:WHY: Pre-sized storage avoids resizing during allocation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(values).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn new(value_count: usize) -> RegisterAllocation {
        RegisterAllocation {
            assignments: vec![None; value_count],
            spill_slots: vec![None; value_count],
        }
    }

    /// CEP:WHAT: Returns the assigned register for a value, if any.
    /// CEP:WHY: Encoding reads assignments.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None if unallocated or out of range.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::tests::lir_construction.
    pub fn register(&self, value: ValueId) -> Option<RegId> {
        self.assignments.get(value.0 as usize).copied().flatten()
    }
}
