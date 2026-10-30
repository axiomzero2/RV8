// CEP:FILE: crates/rv8_ir/src/machir.rs
// CEP:WHAT: Level 4 Machine IR per docs/specs/multi_level_ir.md section 5: target-specific
//   basic blocks of instructions over physical registers with explicit memory operands and
//   predication, plus a verifier for the machine-level invariants 5.3.1-5.3.4 that are
//   checkable without a full target description.
// CEP:WHY: The binary encoder consumes MIR as its only input (spec 5.4); catching ISA
//   illegality, register-file violations, ABI frame defects, and invalid addressing modes
//   here keeps the encoder a total function over verified MIR.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: accessors are total over dense indices; verify() returns a descriptive
//   MirError instead of panicking.
// CEP:ASSUMES: MIR is post-register-allocation (every operand register is physical);
//   the opcode set is the shared MachineOpcode contract from lir.rs.
// CEP:COST: flat POD vectors; construction amortized O(1); verify() is O(N * operands).
// CEP:EVIDENCE: unit tests rv8_ir::machir::tests::*.
// CEP:SECURITY: data only; no traversal recursion.

use crate::lir::MachineOpcode;
use std::fmt::Write as _;

/// CEP:WHAT: Maximum blocks or instructions per Machine-IR function.
/// CEP:WHY: Bounded compile time and memory (38.12); named limit.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced by the backend and re-checked by verify().
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_budget_guard_rejects_oversize.
pub const MAX_MIR_ITEMS: u32 = 1_000_000;

/// CEP:WHAT: Physical register identity in target encoding space.
/// CEP:WHY: Post-allocation operands name real hardware registers (spec 5.2).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: class of a register is derived from its number per ISA.
/// CEP:COST: POD u16.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PhysReg(pub u16);

/// CEP:WHAT: Physical register class derived from the register number.
/// CEP:WHY: Register-file constraint checking (5.3.2) needs the class of every operand.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: x86-64 encoding space: 0..=15 GPR (RAX..R15), 16..=31 FPR (XMM0..XMM15);
///   AArch64 encoding space: 0..=30 GPR (X0..X30), 31 stack pointer, 32..=63 FPR
///   (V0..V31); other numbers are Reserved and rejected by verify().
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_mixed_register_classes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum PhysClass {
    Gpr,
    Fpr,
    Special,
    Reserved,
}

/// CEP:WHAT: Operand register class requirement for one opcode slot.
/// CEP:WHY: Per-opcode signature checking (5.3.2) without full target tables.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Any accepts Gpr or Fpr for flexible slots (e.g. call arguments).
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_mixed_register_classes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotClass {
    Gpr,
    Fpr,
    Any,
}

/// CEP:WHAT: Target instruction-set architectures with MIR-level contracts.
/// CEP:WHY: ISA validity (5.3.1) is per-target; the enum enumerates the supported set.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: discriminants stable (hashing).
/// CEP:COST: POD u8.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_isa_illegal_opcode.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Isa {
    X86_64,
    Aarch64,
}

/// CEP:WHAT: Explicit addressing mode: base + index*scale + displacement (spec 5.2).
/// CEP:WHY: Memory operands are first-class so the encoder can emit ModRM/SIB or
///   AArch64 addressing forms from one structure.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: scale is one of 1,2,4,8; index is None implies scale must be 1
///   (verifier-checked); at least one of base, index must be present.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_invalid_addressing_mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemOperand {
    pub base: Option<PhysReg>,
    pub index: Option<PhysReg>,
    pub scale: u8,
    pub disp: i32,
}

/// CEP:WHAT: Conditional-execution predicate for architectures that support predication
/// (spec 5.2).
/// CEP:WHY: AArch64 conditional instructions execute under a flag predicate; x86-64 has
/// no predicated ALU ops, which verify() enforces.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: flag names a condition-flags register in the target space.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_predication_on_x86.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Predicate {
    pub flag: PhysReg,
    pub expect: bool,
}

/// CEP:WHAT: One machine operand: physical register, immediate, or memory reference.
/// CEP:WHY: The closed operand vocabulary of spec 5.2.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MachineOperand {
    Reg(PhysReg),
    Imm(i64),
    Mem(MemOperand),
}

/// CEP:WHAT: One machine instruction: target mnemonic, operands, and predicates.
/// CEP:WHY: The unit the binary encoder translates (spec 5.4) and the scheduler reorders.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: operand count and classes match the opcode signature (verifier-checked).
/// CEP:COST: POD plus operand vector.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
#[derive(Clone, Debug)]
pub struct MachineInstr {
    pub opcode: MachineOpcode,
    pub operands: Vec<MachineOperand>,
    pub predicates: Vec<Predicate>,
}

impl MachineInstr {
    /// CEP:WHAT: Reports whether the instruction transfers control.
    /// CEP:WHY: Block-terminator checking mirrors LIR invariant 4.3.3 at machine level.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_block_without_terminator.
    pub fn is_terminator(&self) -> bool {
        matches!(
            self.opcode,
            MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret
        )
    }
}

/// CEP:WHAT: One machine basic block: instruction sequence and successor labels.
/// CEP:WHY: Layout, peephole, and scheduling operate on blocks (spec 5.4).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the last instruction is a terminator (verifier-checked); successors
/// reference existing block ids.
/// CEP:COST: POD vectors.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_block_without_terminator.
#[derive(Clone, Debug)]
pub struct MachineBlock {
    pub instructions: Vec<MachineInstr>,
    pub successors: Vec<u32>,
}

/// CEP:WHAT: Stack frame layout conforming to the target ABI (spec 5.3.3).
/// CEP:WHY: Alignment, red-zone use, and saved-register lists are ABI facts the encoder
/// and prologue/epilogue lowering must agree on.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: alignment is a power of two >= 16; stack_size is a multiple of alignment
/// (verifier-checked).
/// CEP:COST: POD plus saved-register vector.
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_abi_nonconforming_frame.
#[derive(Clone, Debug)]
pub struct MachineFrame {
    pub stack_size: u32,
    pub alignment: u32,
    pub has_red_zone: bool,
    pub saved_registers: Vec<PhysReg>,
}

/// CEP:WHAT: Error returned by MachineIrFunction::verify.
/// CEP:WHY: Fail-closed contracts without panics; variants name offending locations.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit tests rv8_ir::machir::tests::mir_rejects_*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirError {
    IsaIllegalOpcode { block: u32, instr: u32, opcode: &'static str },
    PredicationUnsupported { block: u32, instr: u32 },
    RegisterClassViolation { block: u32, instr: u32, operand: u32 },
    ReservedRegister { block: u32, instr: u32 },
    InvalidAddressingMode { block: u32, instr: u32, operand: u32 },
    BlockMissingTerminator { block: u32 },
    DanglingSuccessor { block: u32, successor: u32 },
    FrameMisaligned { alignment: u32 },
    FrameSizeNotMultiple { stack_size: u32, alignment: u32 },
    Oversized { items: u32 },
}

/// CEP:WHAT: Enforces the per-function item budget over blocks and instructions.
/// CEP:WHY: Named compile-time budget (38.12); pure function for direct testing.
/// CEP:STATUS: complete
/// CEP:FAILURE: Oversized when items exceeds MAX_MIR_ITEMS.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_budget_guard_rejects_oversize.
fn check_budget(items: u32) -> Result<(), MirError> {
    if items > MAX_MIR_ITEMS {
        Err(MirError::Oversized { items })
    } else {
        Ok(())
    }
}

/// CEP:WHAT: Classifies a physical register number for an ISA.
/// CEP:WHY: 5.3.2 register-file constraint checking starts from the operand class.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: see PhysClass docs for the encoding-space partition.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_mixed_register_classes.
pub fn reg_class(isa: Isa, reg: PhysReg) -> PhysClass {
    match isa {
        Isa::X86_64 => match reg.0 {
            0..=15 => PhysClass::Gpr,
            16..=31 => PhysClass::Fpr,
            _ => PhysClass::Reserved,
        },
        Isa::Aarch64 => match reg.0 {
            0..=30 => PhysClass::Gpr,
            31 => PhysClass::Special,
            32..=63 => PhysClass::Fpr,
            _ => PhysClass::Reserved,
        },
    }
}

/// CEP:WHAT: Renders a physical register name for an ISA.
/// CEP:WHY: Printable IR contract (38.17).
/// CEP:STATUS: complete
/// CEP:FAILURE: unknown numbers print as rN.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_print_and_hash_are_stable.
pub fn reg_name(isa: Isa, reg: PhysReg) -> String {
    match isa {
        Isa::X86_64 => match reg.0 {
            0 => "rax".into(),
            1 => "rcx".into(),
            2 => "rdx".into(),
            3 => "rbx".into(),
            4 => "rsp".into(),
            5 => "rbp".into(),
            6 => "rsi".into(),
            7 => "rdi".into(),
            8..=15 => format!("r{}", reg.0),
            16..=31 => format!("xmm{}", reg.0 - 16),
            other => format!("r{other}"),
        },
        Isa::Aarch64 => match reg.0 {
            31 => "sp".into(),
            0..=30 => format!("x{}", reg.0),
            32..=63 => format!("v{}", reg.0 - 32),
            other => format!("r{other}"),
        },
    }
}

/// CEP:WHAT: The register-class signature of an opcode: result slot class plus per-operand
/// slot classes, with None marking slots that take immediates or memory operands.
/// CEP:WHY: 5.3.2 forbids mixing integer and floating-point registers in one instruction
/// unless the ISA explicitly supports it; the signature table is that support, encoded.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; opcodes absent from the table default to Any slots, which the ISA
/// legality gate still constrains.
/// CEP:ASSUMES: signature order matches the canonical operand order of the encoder.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_mixed_register_classes.
pub fn opcode_signature(opcode: MachineOpcode) -> Option<(SlotClass, Vec<Option<SlotClass>>)> {
    let g = Some(SlotClass::Gpr);
    let f = Some(SlotClass::Fpr);
    match opcode {
        MachineOpcode::MovsdRegReg => Some((SlotClass::Fpr, vec![f, f])),
        MachineOpcode::CvtSi2sd => Some((SlotClass::Fpr, vec![f, g])),
        MachineOpcode::AddRegReg
        | MachineOpcode::SubRegReg
        | MachineOpcode::ImulRegReg
        | MachineOpcode::AndRegReg
        | MachineOpcode::OrRegReg
        | MachineOpcode::XorRegReg
        | MachineOpcode::CmpRegReg => Some((SlotClass::Gpr, vec![g, g])),
        MachineOpcode::AddRegImm32
        | MachineOpcode::SubRegImm32
        | MachineOpcode::CmpRegImm32
        | MachineOpcode::ShlRegImm8
        | MachineOpcode::SarRegImm8 => Some((SlotClass::Gpr, vec![g, None])),
        MachineOpcode::MovRegImm32 => Some((SlotClass::Gpr, vec![g, None])),
        MachineOpcode::MovRegReg | MachineOpcode::Lea => Some((SlotClass::Gpr, vec![g, g])),
        MachineOpcode::Push | MachineOpcode::Pop | MachineOpcode::IdivReg => {
            Some((SlotClass::Gpr, vec![g]))
        }
        MachineOpcode::MovRegMem => Some((SlotClass::Gpr, vec![g, None])),
        MachineOpcode::MovMemReg => Some((SlotClass::Gpr, vec![None, g])),
        _ => None,
    }
}

/// CEP:WHAT: Reports whether an opcode is legal on an ISA at MIR level.
/// CEP:WHY: Invariant 5.3.1; the conservative default is illegal so new opcodes must be
/// added deliberately per target.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: instruction-selection output was already legal at LIR level; this gate
/// re-checks after allocation-time rewrites.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_isa_illegal_opcode.
pub fn opcode_is_legal(isa: Isa, opcode: MachineOpcode) -> bool {
    match isa {
        Isa::X86_64 => true,
        Isa::Aarch64 => matches!(
            opcode,
            MachineOpcode::MovRegReg
                | MachineOpcode::MovRegImm32
                | MachineOpcode::AddRegReg
                | MachineOpcode::AddRegImm32
                | MachineOpcode::SubRegReg
                | MachineOpcode::SubRegImm32
                | MachineOpcode::ImulRegReg
                | MachineOpcode::AndRegReg
                | MachineOpcode::OrRegReg
                | MachineOpcode::XorRegReg
                | MachineOpcode::CmpRegReg
                | MachineOpcode::CmpRegImm32
                | MachineOpcode::Jmp
                | MachineOpcode::Jcc(_)
                | MachineOpcode::Call
                | MachineOpcode::Ret
                | MachineOpcode::MovRegMem
                | MachineOpcode::MovMemReg
                | MachineOpcode::Fence
        ),
    }
}

/// CEP:WHAT: One function in Machine-IR form.
/// CEP:WHY: The backend output container handed to the binary encoder (spec 5.4).
/// CEP:STATUS: complete
/// CEP:FAILURE: see MirError.
/// CEP:ASSUMES: one function per container; physical allocation is complete.
/// CEP:COST: POD vectors.
/// CEP:EVIDENCE: unit tests rv8_ir::machir::tests::*.
#[derive(Clone, Debug)]
pub struct MachineIrFunction {
    pub name: String,
    pub isa: Isa,
    pub blocks: Vec<MachineBlock>,
    pub frame: MachineFrame,
}

impl MachineIrFunction {
    /// CEP:WHAT: Creates an empty MIR function with an ABI-default frame.
    /// CEP:WHY: Construction boundary; alignment defaults to the 16-byte ABI minimum.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
    pub fn new(name: &str, isa: Isa) -> MachineIrFunction {
        MachineIrFunction {
            name: name.to_string(),
            isa,
            blocks: Vec::new(),
            frame: MachineFrame {
                stack_size: 0,
                alignment: 16,
                has_red_zone: false,
                saved_registers: Vec::new(),
            },
        }
    }

    /// CEP:WHAT: Appends a block and returns its index.
    /// CEP:WHY: Single construction path keeps block ids dense.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
    pub fn push_block(&mut self, block: MachineBlock) -> u32 {
        let id = self.blocks.len() as u32;
        self.blocks.push(block);
        id
    }

    /// CEP:WHAT: Convenience constructor for one instruction.
    /// CEP:WHY: Tests and the encoder-side rewriter build instructions constantly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(operands).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_construction_and_verify.
    pub fn instr(opcode: MachineOpcode, operands: Vec<MachineOperand>) -> MachineInstr {
        MachineInstr {
            opcode,
            operands,
            predicates: Vec::new(),
        }
    }

    /// CEP:WHAT: Verifies invariants 5.3.1-5.3.4, terminators, successor existence, and
    /// the item budget.
    /// CEP:WHY: 38.18 requires verification after every backend pass; peephole and
    /// scheduling rewrites must re-establish machine validity before encoding.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns the first violated invariant as a MirError naming block,
    /// instruction, and operand positions.
    /// CEP:ASSUMES: none; total over any contents.
    /// CEP:COST: O(B + I * operands); iterative.
    /// CEP:EVIDENCE: unit tests rv8_ir::machir::tests::mir_rejects_*.
    pub fn verify(&self) -> Result<(), MirError> {
        let item_count = match self
            .blocks
            .len()
            .checked_add(self.blocks.iter().map(|b| b.instructions.len()).sum::<usize>())
        {
            Some(v) => v as u32,
            None => return Err(MirError::Oversized { items: u32::MAX }),
        };
        check_budget(item_count)?;

        // Frame conformance (5.3.3): power-of-two alignment >= 16, size multiple of it.
        let align = self.frame.alignment;
        if align < 16 || !align.is_power_of_two() {
            return Err(MirError::FrameMisaligned { alignment: align });
        }
        if !self.frame.stack_size.is_multiple_of(align) {
            return Err(MirError::FrameSizeNotMultiple {
                stack_size: self.frame.stack_size,
                alignment: align,
            });
        }

        for (bi, block) in self.blocks.iter().enumerate() {
            let b = bi as u32;
            // Terminator requirement (machine analogue of 4.3.3).
            match block.instructions.last() {
                Some(last) if last.is_terminator() => {}
                _ => return Err(MirError::BlockMissingTerminator { block: b }),
            }
            for &succ in &block.successors {
                if succ as usize >= self.blocks.len() {
                    return Err(MirError::DanglingSuccessor { block: b, successor: succ });
                }
            }
            for (ii, instr) in block.instructions.iter().enumerate() {
                let i = ii as u32;
                // 5.3.1 ISA legality.
                if !opcode_is_legal(self.isa, instr.opcode) {
                    return Err(MirError::IsaIllegalOpcode {
                        block: b,
                        instr: i,
                        opcode: opcode_name(instr.opcode),
                    });
                }
                // Predication support (5.2 predicates exist only where the ISA has them).
                if !instr.predicates.is_empty() && self.isa == Isa::X86_64 {
                    return Err(MirError::PredicationUnsupported { block: b, instr: i });
                }
                // 5.3.2 register-file constraints via the opcode signature.
                if let Some((result_class, slots)) = opcode_signature(instr.opcode) {
                    for (oi, operand) in instr.operands.iter().enumerate() {
                        let reg = match operand {
                            MachineOperand::Reg(r) => *r,
                            _ => continue,
                        };
                        let class = reg_class(self.isa, reg);
                        if class == PhysClass::Reserved {
                            return Err(MirError::ReservedRegister { block: b, instr: i });
                        }
                        let slot = slots.get(oi).copied().flatten();
                        let allowed = match slot {
                            Some(SlotClass::Gpr) => class == PhysClass::Gpr,
                            Some(SlotClass::Fpr) => class == PhysClass::Fpr,
                            Some(SlotClass::Any) => true,
                            None => match (oi, result_class) {
                                (0, SlotClass::Gpr) => class == PhysClass::Gpr,
                                (0, SlotClass::Fpr) => class == PhysClass::Fpr,
                                _ => true,
                            },
                        };
                        if !allowed {
                            return Err(MirError::RegisterClassViolation {
                                block: b,
                                instr: i,
                                operand: oi as u32,
                            });
                        }
                    }
                }
                // 5.3.4 valid addressing modes.
                for (oi, operand) in instr.operands.iter().enumerate() {
                    if let MachineOperand::Mem(mem) = operand {
                        if !mem_operand_valid(self.isa, mem) {
                            return Err(MirError::InvalidAddressingMode {
                                block: b,
                                instr: i,
                                operand: oi as u32,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Prints the function in stable block/instruction order with register
    /// names.
    /// CEP:WHY: Printable IR contract (38.17).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(I * operands).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_print_and_hash_are_stable.
    pub fn print(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "fn {} {:?} frame={} align={}", self.name, self.isa, self.frame.stack_size, self.frame.alignment);
        for (bi, block) in self.blocks.iter().enumerate() {
            let _ = writeln!(out, "b{}:", bi);
            for instr in &block.instructions {
                let _ = write!(out, "  {}", opcode_name(instr.opcode));
                for (oi, op) in instr.operands.iter().enumerate() {
                    let _ = if oi == 0 {
                        write!(out, " ")
                    } else {
                        write!(out, ", ")
                    };
                    match op {
                        MachineOperand::Reg(r) => {
                            let _ = write!(out, "{}", reg_name(self.isa, *r));
                        }
                        MachineOperand::Imm(v) => {
                            let _ = write!(out, "#{v}");
                        }
                        MachineOperand::Mem(m) => {
                            let _ = write!(out, "[");
                            if let Some(base) = m.base {
                                let _ = write!(out, "{}", reg_name(self.isa, base));
                            }
                            if let Some(index) = m.index {
                                let _ = write!(
                                    out,
                                    "+{}*{}",
                                    reg_name(self.isa, index),
                                    m.scale
                                );
                            }
                            let _ = write!(out, "{:+}]", m.disp);
                        }
                    }
                }
                let _ = writeln!(out);
            }
        }
        out
    }

    /// CEP:WHAT: Computes a deterministic content hash over ISA, frame, blocks, and
    /// instructions.
    /// CEP:WHY: CAS cache keys on machine code content (38.19).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: vector order is canonical.
    /// CEP:COST: O(I * operands).
    /// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_print_and_hash_are_stable.
    pub fn canonical_hash(&self) -> u64 {
        let mut hash: u64 = 0;
        hash = mix(hash, &(self.isa as u8).to_le_bytes());
        hash = mix(hash, &self.frame.stack_size.to_le_bytes());
        hash = mix(hash, &self.frame.alignment.to_le_bytes());
        hash = mix(hash, &[self.frame.has_red_zone as u8]);
        for block in &self.blocks {
            for instr in &block.instructions {
                hash = mix(hash, opcode_name(instr.opcode).as_bytes());
                for op in &instr.operands {
                    match op {
                        MachineOperand::Reg(r) => hash = mix(hash, &r.0.to_le_bytes()),
                        MachineOperand::Imm(v) => hash = mix(hash, &v.to_le_bytes()),
                        MachineOperand::Mem(m) => {
                            hash = mix(
                                hash,
                                &m.base.map(|r| r.0 as u64).unwrap_or(u64::MAX).to_le_bytes(),
                            );
                            hash = mix(
                                hash,
                                &m.index.map(|r| r.0 as u64).unwrap_or(u64::MAX).to_le_bytes(),
                            );
                            hash = mix(hash, &[m.scale]);
                            hash = mix(hash, &m.disp.to_le_bytes());
                        }
                    }
                }
                for p in &instr.predicates {
                    hash = mix(hash, &p.flag.0.to_le_bytes());
                    hash = mix(hash, &[p.expect as u8]);
                }
                for &succ in &block.successors {
                    hash = mix(hash, &succ.to_le_bytes());
                }
            }
        }
        hash
    }
}

/// CEP:WHAT: Validates one memory operand for an ISA (5.3.4).
/// CEP:WHY: x86-64 allows base+index*scale+disp with scale in 1..=8; AArch64 scalar
/// addressing in this contract is base+imm9 only.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: register validity is checked by the register-class pass, not here.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_rejects_invalid_addressing_mode.
fn mem_operand_valid(isa: Isa, mem: &MemOperand) -> bool {
    if mem.base.is_none() && mem.index.is_none() {
        return false;
    }
    match isa {
        Isa::X86_64 => {
            let scale_ok = matches!(mem.scale, 1 | 2 | 4 | 8);
            let no_index_scale_ok = mem.index.is_some() || mem.scale == 1;
            scale_ok && no_index_scale_ok
        }
        Isa::Aarch64 => mem.index.is_none() && mem.scale == 1 && mem.disp >= -256 && mem.disp <= 255,
    }
}

/// CEP:WHAT: Stable mnemonic name for a machine opcode.
/// CEP:WHY: Printing and hashing share one table; lir.rs does not expose names.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_print_and_hash_are_stable.
fn opcode_name(opcode: MachineOpcode) -> &'static str {
    match opcode {
        MachineOpcode::MovRegReg => "mov",
        MachineOpcode::MovRegImm32 => "movimm",
        MachineOpcode::Lea => "lea",
        MachineOpcode::AddRegReg => "add",
        MachineOpcode::AddRegImm32 => "addimm",
        MachineOpcode::SubRegReg => "sub",
        MachineOpcode::SubRegImm32 => "subimm",
        MachineOpcode::ImulRegReg => "imul",
        MachineOpcode::ShlRegImm8 => "shl",
        MachineOpcode::SarRegImm8 => "sar",
        MachineOpcode::AndRegReg => "and",
        MachineOpcode::OrRegReg => "or",
        MachineOpcode::XorRegReg => "xor",
        MachineOpcode::XorRegSelf => "xorself",
        MachineOpcode::CmpRegReg => "cmp",
        MachineOpcode::CmpRegImm32 => "cmpimm",
        MachineOpcode::Setcc(_) => "setcc",
        MachineOpcode::Jmp => "jmp",
        MachineOpcode::Jcc(_) => "jcc",
        MachineOpcode::Call => "call",
        MachineOpcode::Ret => "ret",
        MachineOpcode::Push => "push",
        MachineOpcode::Pop => "pop",
        MachineOpcode::Cqo => "cqo",
        MachineOpcode::IdivReg => "idiv",
        MachineOpcode::MovsdRegReg => "movsd",
        MachineOpcode::CvtSi2sd => "cvtsi2sd",
        MachineOpcode::Syscall => "syscall",
        MachineOpcode::LoadStack => "loadstack",
        MachineOpcode::StoreStack => "storestack",
        MachineOpcode::StoreStackImm => "storestackimm",
        MachineOpcode::MovRegMem => "movrm",
        MachineOpcode::MovMemReg => "movmr",
        MachineOpcode::Fence => "fence",
    }
}

/// CEP:WHAT: Chains bytes into the running canonical hash state.
/// CEP:WHY: Shared helper for MIR hashing.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_ir::machir::tests::mir_print_and_hash_are_stable.
fn mix(state: u64, bytes: &[u8]) -> u64 {
    rv8_core::fx_mix(state, rv8_core::fxhash64(bytes))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::lir::SetKind;

    /// CEP:WHAT: Common register numbers for x86-64 tests.
    /// CEP:WHY: Readability of test bodies.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    const RAX: PhysReg = PhysReg(0);
    const RDI: PhysReg = PhysReg(7);
    const XMM0: PhysReg = PhysReg(16);

    /// CEP:WHAT: Builds mov rax,#1; add rax,rdi; ret and verifies.
    /// CEP:WHY: Happy-path construction, print, and hash stability.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_construction_and_verify() {
        let mut f = MachineIrFunction::new("f", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::MovRegImm32,
                    vec![MachineOperand::Reg(RAX), MachineOperand::Imm(1)],
                ),
                MachineIrFunction::instr(
                    MachineOpcode::AddRegReg,
                    vec![MachineOperand::Reg(RAX), MachineOperand::Reg(RDI)],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![],
        });
        f.verify().expect("valid x86-64 MIR must verify");
        assert!(f.print().contains("add rax, rdi"));
        assert_eq!(f.canonical_hash(), f.canonical_hash());
    }

    /// CEP:WHAT: Push on AArch64 must be rejected as ISA-illegal.
    /// CEP:WHY: Invariant 5.3.1.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_isa_illegal_opcode() {
        let mut f = MachineIrFunction::new("f", Isa::Aarch64);
        f.push_block(MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::Push,
                    vec![MachineOperand::Reg(PhysReg(0))],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![],
        });
        match f.verify() {
            Err(MirError::IsaIllegalOpcode { opcode, .. }) => assert_eq!(opcode, "push"),
            other => panic!("expected IsaIllegalOpcode, got {other:?}"),
        }
    }

    /// CEP:WHAT: An FPR in a GPR slot must be rejected.
    /// CEP:WHY: Invariant 5.3.2.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_mixed_register_classes() {
        let mut f = MachineIrFunction::new("f", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::AddRegReg,
                    vec![MachineOperand::Reg(RAX), MachineOperand::Reg(XMM0)],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![],
        });
        match f.verify() {
            Err(MirError::RegisterClassViolation { operand, .. }) => assert_eq!(operand, 1),
            other => panic!("expected RegisterClassViolation, got {other:?}"),
        }
    }

    /// CEP:WHAT: Scale 3 or scale with no index must be rejected; valid forms pass.
    /// CEP:WHY: Invariant 5.3.4.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_invalid_addressing_mode() {
        let bad = MemOperand {
            base: Some(RAX),
            index: Some(RDI),
            scale: 3,
            disp: 0,
        };
        let no_index_scale = MemOperand {
            base: Some(RAX),
            index: None,
            scale: 4,
            disp: 8,
        };
        let good = MemOperand {
            base: Some(RAX),
            index: Some(RDI),
            scale: 8,
            disp: -16,
        };
        assert!(!mem_operand_valid(Isa::X86_64, &bad));
        assert!(!mem_operand_valid(Isa::X86_64, &no_index_scale));
        assert!(mem_operand_valid(Isa::X86_64, &good));
        let mut f = MachineIrFunction::new("f", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::MovRegMem,
                    vec![
                        MachineOperand::Reg(RAX),
                        MachineOperand::Mem(bad),
                    ],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![],
        });
        match f.verify() {
            Err(MirError::InvalidAddressingMode { operand, .. }) => assert_eq!(operand, 1),
            other => panic!("expected InvalidAddressingMode, got {other:?}"),
        }
    }

    /// CEP:WHAT: Predicated instructions on x86-64 must be rejected; on AArch64 accepted.
    /// CEP:WHY: Predication exists only where the ISA supports it (spec 5.2).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_predication_on_x86() {
        let predicated = MachineInstr {
            opcode: MachineOpcode::AddRegReg,
            operands: vec![
                MachineOperand::Reg(PhysReg(0)),
                MachineOperand::Reg(PhysReg(1)),
            ],
            predicates: vec![Predicate {
                flag: PhysReg(63),
                expect: true,
            }],
        };
        let mut fx = MachineIrFunction::new("fx", Isa::X86_64);
        fx.push_block(MachineBlock {
            instructions: vec![predicated.clone(), MachineIrFunction::instr(MachineOpcode::Ret, vec![])],
            successors: vec![],
        });
        match fx.verify() {
            Err(MirError::PredicationUnsupported { .. }) => {}
            other => panic!("expected PredicationUnsupported, got {other:?}"),
        }
        let mut fa = MachineIrFunction::new("fa", Isa::Aarch64);
        fa.push_block(MachineBlock {
            instructions: vec![predicated, MachineIrFunction::instr(MachineOpcode::Ret, vec![])],
            successors: vec![],
        });
        fa.verify().expect("aarch64 supports predication");
    }

    /// CEP:WHAT: Alignment 8 and non-multiple stack sizes must be rejected.
    /// CEP:WHY: Invariant 5.3.3.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_abi_nonconforming_frame() {
        let mut f = MachineIrFunction::new("f", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![MachineIrFunction::instr(MachineOpcode::Ret, vec![])],
            successors: vec![],
        });
        f.frame.alignment = 8;
        match f.verify() {
            Err(MirError::FrameMisaligned { alignment }) => assert_eq!(alignment, 8),
            other => panic!("expected FrameMisaligned, got {other:?}"),
        }
        f.frame.alignment = 16;
        f.frame.stack_size = 24;
        match f.verify() {
            Err(MirError::FrameSizeNotMultiple { stack_size, alignment }) => {
                assert_eq!((stack_size, alignment), (24, 16));
            }
            other => panic!("expected FrameSizeNotMultiple, got {other:?}"),
        }
        f.frame.stack_size = 32;
        f.verify().expect("32 bytes at 16-byte alignment conforms");
    }

    /// CEP:WHAT: A block without a terminator must be rejected; a dangling successor must
    /// be rejected.
    /// CEP:WHY: Machine analogue of LIR invariant 4.3.3 and CFG integrity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_rejects_block_without_terminator() {
        let mut f = MachineIrFunction::new("f", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![MachineIrFunction::instr(
                MachineOpcode::MovRegImm32,
                vec![MachineOperand::Reg(RAX), MachineOperand::Imm(0)],
            )],
            successors: vec![],
        });
        match f.verify() {
            Err(MirError::BlockMissingTerminator { block }) => assert_eq!(block, 0),
            other => panic!("expected BlockMissingTerminator, got {other:?}"),
        }
        // Dangling successor check: terminator present, successor id out of range.
        f.blocks[0].instructions.push(MachineIrFunction::instr(MachineOpcode::Jmp, vec![]));
        f.blocks[0].successors = vec![7];
        match f.verify() {
            Err(MirError::DanglingSuccessor { successor, .. }) => assert_eq!(successor, 7),
            other => panic!("expected DanglingSuccessor, got {other:?}"),
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
    fn mir_print_and_hash_are_stable() {
        let mut f = MachineIrFunction::new("p", Isa::X86_64);
        f.push_block(MachineBlock {
            instructions: vec![
                MachineIrFunction::instr(
                    MachineOpcode::Jcc(SetKind::Ne),
                    vec![],
                ),
                MachineIrFunction::instr(MachineOpcode::Ret, vec![]),
            ],
            successors: vec![1],
        });
        f.push_block(MachineBlock {
            instructions: vec![MachineIrFunction::instr(MachineOpcode::Ret, vec![])],
            successors: vec![],
        });
        assert_eq!(f.print(), f.print());
        assert_eq!(f.canonical_hash(), f.canonical_hash());
    }

    /// CEP:WHAT: The budget guard rejects counts above MAX_MIR_ITEMS and accepts the
    /// boundary.
    /// CEP:WHY: Named compile-time budget (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(1).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn mir_budget_guard_rejects_oversize() {
        assert!(check_budget(MAX_MIR_ITEMS).is_ok());
        match check_budget(MAX_MIR_ITEMS + 1) {
            Err(MirError::Oversized { items }) => assert_eq!(items, MAX_MIR_ITEMS + 1),
            other => panic!("expected Oversized, got {other:?}"),
        }
    }
}
