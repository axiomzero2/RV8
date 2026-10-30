// CEP:FILE: crates/rv8_backend/src/encode.rs
// CEP:WHAT: The x86-64 binary encoder over post-allocation Machine IR: two-pass layout
//   (validate -> size/block offsets -> emit with resolved branch displacements), the full
//   MachineOpcode set (REX.W where 64-bit, ModRM/SIB from MemOperand with scale 1/2/4/8,
//   disp8/disp32 selection), `e8 rel32` calls as external R_X86_64_PC32 relocations
//   against `call_symbol_name(target)`, internal Jmp/Jcc resolved from block offsets, and
//   deterministic 0x90 padding so every function's byte length is a multiple of
//   ENTRY_ALIGNMENT (16).
// CEP:WHY: Spec 5.4: the encoder is the total function over verified MIR; catching
//   invalid operand shapes, scales, immediates, and ISA mismatches here (defense in depth
//   against unverified input) keeps library code panic-free and fail-closed.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// Note: Abs64 is defined and mapped for the pinned contract but this revision emits
// only PcRel32 (call sites); internal branches are resolved directly.
// CEP:FAILURE: BackendError::UnsupportedIsa for non-x86-64 MIR, InvalidAddressing for
//   illegal MemOperands (scale not in 1/2/4/8, rsp as index, missing base and index,
//   reserved registers), IllegalOperandShape for arity/immediate/register-class
//   violations, Verification for misplaced terminators or a Jcc fallthrough successor
//   that does not match layout order. Library code never panics.
// CEP:ASSUMES: MIR satisfies the machir 5.3 invariants (re-checked here); block order is
//   the layout order, so a conditional branch's successors[1] must be the next block
//   (successors[0] is the taken target; the isel convention); Call's trailing Imm(t) is
//   the callee's symbol ordinal rendered by call_symbol_name as "fn{t}".
// CEP:COST: two linear passes over the instructions plus one validation pass; O(bytes)
//   emission. No recursion; no maps in the output path (38.10).
// CEP:EVIDENCE: unit tests pin exact bytes for every opcode family (mov/lea/alu/imul/
//   shifts/cmp/setcc/jcc/jmp/call/ret/push/pop/cqo/idiv/movsd/cvt/syscall/fence/load/
//   store/ModRM+SIB forms), the ABI add() end-to-end byte listing, diamond-CFG branch
//   displacement arithmetic, determinism, and the negative cases.
// CEP:SECURITY: data-only transforms over bounded operands; displacement arithmetic uses
//   i64 with explicit range checks before narrowing.

use crate::error::BackendError;
use rv8_ir::lir::{MachineOpcode, SetKind};
use rv8_ir::machir::{
    Isa, MachineInstr, MachineIrFunction, MachineOperand, MemOperand, PhysReg, PhysClass,
    opcode_signature, reg_class, SlotClass,
};

/// CEP:WHAT: Entry alignment: every emitted function's byte length is rounded up to a
/// multiple of this with 0x90 padding, so a function placed at a 16-byte-aligned section
/// offset is followed by another 16-byte-aligned entry (SysV stack alignment contract).
/// CEP:WHY: Deterministic padding keeps two-pass sizes stable and reproducible (38.10).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the section carrying the symbol is 16-byte aligned (rv8_object's job).
/// CEP:COST: <= 15 padding bytes per function.
/// CEP:EVIDENCE: abi_add_end_to_end_bytes asserts the exact padded length.
pub const ENTRY_ALIGNMENT: u64 = 16;

/// CEP:WHAT: The padding byte (one-byte NOP).
/// CEP:WHY: Named constant (38.12); executable, deterministic padding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: every byte-listing test.
pub const NOP_PAD: u8 = 0x90;

/// CEP:WHAT: Relocation kind: mirrored structurally by rv8_object::model::RelocKind
/// (field order and elf_type mapping are a pinned cross-agent contract).
/// CEP:WHY: Abs64 (R_X86_64_64) and PcRel32 (R_X86_64_PC32) cover static linking.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: elf_type() mapping is contractual: Abs64 => 1, PcRel32 => 2.
/// CEP:COST: Copy enum.
/// CEP:EVIDENCE: reloc_kind_elf_type_mapping test; rv8_object acceptance test mirrors it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelocKind {
    /// S + A written as a 64-bit absolute value (ELF R_X86_64_64).
    Abs64,
    /// S + A - P written as a signed 32-bit value (ELF R_X86_64_PC32).
    PcRel32,
}

impl RelocKind {
    /// CEP:WHAT: ELF relocation type code for this kind.
    /// CEP:WHY: The serializer embeds it in r_info; part of the pinned contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: x86-64 ELF numbering.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: reloc_kind_elf_type_mapping test.
    pub fn elf_type(self) -> u32 {
        match self {
            RelocKind::Abs64 => 1,
            RelocKind::PcRel32 => 2,
        }
    }
}

/// CEP:WHAT: One relocation: patch `kind` bytes at `offset` inside the emitted code.
/// CEP:WHY: Pinned cross-agent shape mirroring rv8_object::model::Reloc; PcRel32 carries
///   addend -4 and `offset` points at the rel32 FIELD (one byte after the e8 opcode).
/// CEP:STATUS: complete
/// CEP:FAILURE: none (range validation happens in rv8_object at link time).
/// CEP:ASSUMES: offsets are relative to the start of this function's code bytes.
/// CEP:COST: inline data; one String per relocation symbol.
/// CEP:EVIDENCE: call_relocation_is_pcrel32 test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    /// Byte offset from the start of this function's code.
    pub offset: u64,
    /// How to compute and how wide the patch is.
    pub kind: RelocKind,
    /// Symbol name this relocation resolves against.
    pub symbol: String,
    /// Addend A in S + A (Abs64) or S + A - P (PcRel32).
    pub addend: i64,
}

/// CEP:WHAT: The encoder's output for one function: symbol name, code bytes, relocations,
///   and the ABI-conformant frame metadata that shaped the prologue/epilogue.
/// CEP:WHY: The unit rv8_object wraps into a section; the frame rides along so object
///   emission and debugging tools see the same layout the code encodes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: owned buffers.
/// CEP:EVIDENCE: compile_lir end-to-end tests.
#[derive(Debug, Clone)]
pub struct CodegenOutput {
    pub symbol: String,
    pub code: Vec<u8>,
    pub relocs: Vec<Relocation>,
    pub frame: rv8_ir::machir::MachineFrame,
}

/// CEP:WHAT: Deterministic external symbol name for a call-target ordinal.
/// CEP:WHY: The MIR carries the callee as the Call instruction's trailing immediate
///   (the program's function-table ordinal); the encoder must name the relocation. The
///   convention "fn{ordinal}" is a LOUD cross-agent contract: the driver/link side must
///   name functions identically.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: call-target ordinals are non-negative function-table indices.
/// CEP:COST: O(digits).
/// CEP:EVIDENCE: call_relocation_is_pcrel32.
pub fn call_symbol_name(target: i64) -> String {
    format!("fn{target}")
}

/// CEP:WHAT: Condition-code nibble for SetKind in the x86 setcc (0F 9x) and jcc
/// (0F 8x) families: Eq 4, Ne 5, Lt C, Ge D, Le E, Gt F.
/// CEP:WHY: One table shared by set and branch emission keeps the two consistent.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: signed (flag-based) comparison semantics.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: setcc_and_jcc_condition_map test.
fn cc_nibble(kind: SetKind) -> u8 {
    match kind {
        SetKind::Eq => 0x4,
        SetKind::Ne => 0x5,
        SetKind::Lt => 0xC,
        SetKind::Ge => 0xD,
        SetKind::Le => 0xE,
        SetKind::Gt => 0xF,
    }
}

/// CEP:WHAT: One ModRM/SIB-encoded memory operand: the rm field, optional SIB byte,
/// displacement value, and its encoded width; plus the REX.X/REX.B bits the base and
/// index registers contribute.
/// CEP:WHY: Shared by the sizing pass and the emission pass so the two can never disagree.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validated by encode_mem before construction).
/// CEP:ASSUMES: disp_len in {0, 1, 4}.
/// CEP:COST: POD.
/// CEP:EVIDENCE: modrm_sib_encoding test.
#[derive(Debug)]
struct MemEnc {
    rex_x: bool,
    rex_b: bool,
    modrm_rm: u8,
    r#mod: u8,
    sib: Option<u8>,
    disp: i32,
    disp_len: u8,
}

impl MemEnc {
    /// CEP:WHAT: Appends the ModRM byte (with the given reg field), SIB byte, and
    /// displacement to `out`.
    /// CEP:WHY: Single encoding path shared by every memory-form opcode.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(encoded bytes).
    /// CEP:EVIDENCE: modrm_sib_encoding test.
    fn append_to(&self, reg: u8, out: &mut Vec<u8>) {
        out.push((self.r#mod << 6) | ((reg & 7) << 3) | self.modrm_rm);
        if let Some(sib) = self.sib {
            out.push(sib);
        }
        match self.disp_len {
            1 => out.push(self.disp as i8 as u8),
            4 => out.extend_from_slice(&self.disp.to_le_bytes()),
            _ => {}
        }
    }

}

/// CEP:WHAT: Validates and encodes one MemOperand into ModRM/SIB form (x86-64:
/// base + index*scale + disp with scale in 1/2/4/8; rsp never an index; [rbp/r13]-style
/// bases force a displacement; base-less forms use mod 00 rm 101).
/// CEP:WHY: The single addressing-mode gate at encoding time (defense in depth for
/// 5.3.4; the MIR verifier re-checks the same contract upstream).
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidAddressing for scale not in 1/2/4/8 with an index, rsp as index,
/// missing base and index, or a reserved (non-GPR) base/index register.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: modrm_sib_encoding and encode_rejects_invalid_scale tests.
fn encode_mem(mem: &MemOperand) -> Result<MemEnc, BackendError> {
    let invalid = |detail: String| BackendError::InvalidAddressing { detail };
    if mem.base.is_none() && mem.index.is_none() {
        return Err(invalid("memory operand needs a base or index register".into()));
    }
    let class_ok = |r: PhysReg| reg_class(Isa::X86_64, r) == PhysClass::Gpr;
    if let Some(b) = mem.base {
        if !class_ok(b) {
            return Err(invalid(format!("base register {} is not a GPR", b.0)));
        }
    }
    if let Some(x) = mem.index {
        if !class_ok(x) {
            return Err(invalid(format!("index register {} is not a GPR", x.0)));
        }
        if x.0 == 4 {
            return Err(invalid("rsp cannot be an index register".into()));
        }
        if !matches!(mem.scale, 1 | 2 | 4 | 8) {
            return Err(invalid(format!("scale {} not in 1/2/4/8", mem.scale)));
        }
    }
    let scale_bits: u8 = match mem.scale {
        1 => 0b00,
        2 => 0b01,
        4 => 0b10,
        8 => 0b11,
        _ => return Err(invalid(format!("scale {} not in 1/2/4/8", mem.scale))),
    };
    let disp = mem.disp;
    let disp8 = (-128..=127).contains(&disp);
    let base_r = mem.base;
    let needs_sib = mem.index.is_some() || base_r.is_some_and(|b| b.0 & 7 == 4);
    if needs_sib {
        let idx_field: u8 = mem.index.map_or(0b100, |x| (x.0 & 7) as u8);
        let base_field: u8 = base_r.map_or(0b101, |b| (b.0 & 7) as u8);
        let sib: u8 = (scale_bits << 6) | (idx_field << 3) | base_field;
        let (r#mod, disp_len) = match base_r {
            None => (0b00, 4u8), // base-less SIB form: [index*scale + disp32]
            Some(b) if b.0 & 7 == 5 => {
                // [rbp/r13 + ...] has no mod-00 form; disp8 even when zero.
                if disp8 {
                    (0b01, 1)
                } else {
                    (0b10, 4)
                }
            }
            Some(_) if disp == 0 => (0b00, 0),
            Some(_) if disp8 => (0b01, 1),
            Some(_) => (0b10, 4),
        };
        Ok(MemEnc {
            rex_x: mem.index.is_some_and(rex_ext),
            rex_b: base_r.is_some_and(rex_ext),
            modrm_rm: 0b100,
            r#mod,
            sib: Some(sib),
            disp,
            disp_len,
        })
    } else {
        let b = base_r.expect("base present when no SIB is needed");
        let (r#mod, disp_len) = if b.0 & 7 == 5 {
            if disp8 {
                (0b01, 1)
            } else {
                (0b10, 4)
            }
        } else if disp == 0 {
            (0b00, 0)
        } else if disp8 {
            (0b01, 1)
        } else {
            (0b10, 4)
        };
        Ok(MemEnc {
            rex_x: false,
            rex_b: b.0 >= 8,
            modrm_rm: (b.0 & 7) as u8,
            r#mod,
            sib: None,
            disp,
            disp_len,
        })
    }
}

/// CEP:WHAT: Appends a REX prefix byte (0x40 | W<<3 | R<<2 | X<<1 | B) when any bit is
/// needed.
/// CEP:WHY: Single prefix-construction point; `force` covers byte-register forms (setcc
/// on spl/bpl/sil/dil requires a REX prefix even with no extension bits).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: every byte-listing test.
fn push_rex(out: &mut Vec<u8>, w: bool, r: bool, x: bool, b: bool, force: bool) {
    if w || r || x || b || force {
        out.push(
            0x40 | ((w as u8) << 3) | ((r as u8) << 2) | ((x as u8) << 1) | (b as u8),
        );
    }
}

/// CEP:WHAT: Register-field accessor: the low three bits of a register number.
/// CEP:WHY: ModRM/SIB/opcode-reg fields encode only the low three bits.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn r3(reg: PhysReg) -> u8 {
    (reg.0 & 7) as u8
}

/// CEP:WHAT: Whether a register needs its REX.R/REX.B extension bit: r8..r15 for GPRs
/// (PhysReg 8..15) and xmm8..xmm15 for FPRs (PhysReg 24..31).
/// CEP:WHY: FPR numbers are 16..31, so a bare `reg.0 >= 8` test would wrongly promote
/// every xmm0..xmm7 to its extended twin; masking the in-class 4-bit number handles
/// both classes with one rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: register numbers within one class are contiguous multiples of the
/// class base (machir's reg_class partition).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: movsd/cvtsi2sd byte-listing tests (xmm0..xmm7 stay unextended).
fn rex_ext(reg: PhysReg) -> bool {
    (reg.0 & 15) >= 8
}

/// CEP:WHAT: Extracts the register operand at `idx` or fails with IllegalOperandShape.
/// CEP:WHY: Total shape handling without panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalOperandShape when the operand is missing or not a register.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative-shape tests.
fn reg_at(instr: &MachineInstr, idx: usize, what: &str) -> Result<PhysReg, BackendError> {
    match instr.operands.get(idx) {
        Some(MachineOperand::Reg(r)) => Ok(*r),
        other => Err(BackendError::IllegalOperandShape {
            opcode: opcode_name(instr.opcode),
            detail: format!("{what} operand {idx} must be a register, got {other:?}"),
        }),
    }
}

/// CEP:WHAT: Extracts the immediate operand at `idx` (range-checked) or fails.
/// CEP:WHY: Total shape handling without panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalOperandShape when missing, not an immediate, or out of range.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative-shape tests.
fn imm_at(instr: &MachineInstr, idx: usize, lo: i64, hi: i64) -> Result<i64, BackendError> {
    match instr.operands.get(idx) {
        Some(MachineOperand::Imm(v)) if (lo..=hi).contains(v) => Ok(*v),
        Some(MachineOperand::Imm(v)) => Err(BackendError::IllegalOperandShape {
            opcode: opcode_name(instr.opcode),
            detail: format!("immediate {v} outside {lo}..={hi}"),
        }),
        other => Err(BackendError::IllegalOperandShape {
            opcode: opcode_name(instr.opcode),
            detail: format!("operand {idx} must be an immediate in {lo}..={hi}, got {other:?}"),
        }),
    }
}

/// CEP:WHAT: Extracts the memory operand at `idx` or fails.
/// CEP:WHY: Total shape handling without panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalOperandShape when the operand is missing or not a memory ref.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative-shape tests.
fn mem_at(instr: &MachineInstr, idx: usize, what: &str) -> Result<MemOperand, BackendError> {
    match instr.operands.get(idx) {
        Some(MachineOperand::Mem(m)) => Ok(*m),
        other => Err(BackendError::IllegalOperandShape {
            opcode: opcode_name(instr.opcode),
            detail: format!("{what} operand {idx} must be a memory operand, got {other:?}"),
        }),
    }
}

/// CEP:WHAT: Rejects wrong arity for an opcode shape.
/// CEP:WHY: Total shape handling without panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalOperandShape on mismatch.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative-shape tests.
fn check_arity(instr: &MachineInstr, n: usize) -> Result<(), BackendError> {
    if instr.operands.len() != n {
        return Err(BackendError::IllegalOperandShape {
            opcode: opcode_name(instr.opcode),
            detail: format!("expects {} operands, got {}", n, instr.operands.len()),
        });
    }
    Ok(())
}

/// CEP:WHAT: Stable mnemonic for diagnostics (mirrors machir::opcode_name's table).
/// CEP:WHY: Error messages name the opcode without Debug noise; kept local so the
/// encoder does not depend on a private rv8_ir helper.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative tests.
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

/// CEP:WHAT: One instruction's pending external relocation: `at` is the byte offset of
/// the rel32 FIELD within the buffer `encode_instr` appended to (in pass 2 that buffer
/// is the whole function's code, so `at` is already the final code offset).
/// CEP:WHY: Calls are the only external references; the rel32 field starts one byte
/// after the E8 opcode, and the pinned contract stores field offsets (addend -4).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: emitted only in pass 2, where `out` is the function-wide code buffer.
/// CEP:COST: POD plus one String.
/// CEP:EVIDENCE: call_relocation_is_pcrel32.
struct PendingReloc {
    at: u64,
    kind: RelocKind,
    symbol: String,
    addend: i64,
}

/// CEP:WHAT: Encodes one instruction into `out` (appending); `branch_disp` supplies the
/// resolved rel32 for Jmp/Jcc (pass 1 passes a placeholder; both passes agree on size
/// because branch displacement is always exactly 4 bytes). Returns the pending external
/// relocation, if any.
/// CEP:WHY: The single opcode-to-bytes translation point (spec 5.4).
/// CEP:STATUS: complete
/// CEP:FAILURE: IllegalOperandShape for malformed operands or immediates outside the
/// encodable range; InvalidAddressing from encode_mem; Verification when a Jmp/Jcc has
/// no usable successor displacement (branch_disp None at emission).
/// CEP:ASSUMES: operand shapes were validated (validate_instr); register classes
/// re-checked here via the opcode-signature table.
/// CEP:COST: O(encoded bytes).
/// CEP:EVIDENCE: the full byte-listing test suite.
#[allow(clippy::too_many_lines)]
fn encode_instr(
    instr: &MachineInstr,
    branch_disp: Option<i32>,
    out: &mut Vec<u8>,
) -> Result<Option<PendingReloc>, BackendError> {
    let shape = |detail: String| BackendError::IllegalOperandShape {
        opcode: opcode_name(instr.opcode),
        detail,
    };
    // Register-class re-check (5.3.2 defense in depth) via the frozen signature table.
    if let Some((_, slots)) = opcode_signature(instr.opcode) {
        for (oi, slot) in slots.iter().enumerate() {
            match (instr.operands.get(oi), slot) {
                (Some(MachineOperand::Reg(r)), Some(SlotClass::Gpr)) => {
                    if reg_class(Isa::X86_64, *r) != PhysClass::Gpr {
                        return Err(shape(format!("operand {oi} must be a GPR")));
                    }
                }
                (Some(MachineOperand::Reg(r)), Some(SlotClass::Fpr)) => {
                    if reg_class(Isa::X86_64, *r) != PhysClass::Fpr {
                        return Err(shape(format!("operand {oi} must be an FPR")));
                    }
                }
                (Some(MachineOperand::Reg(_)), None) => {
                    return Err(shape(format!("operand {oi} must be an immediate or memory ref")))
                }
                _ => {}
            }
        }
    }
    match instr.opcode {
        MachineOpcode::MovRegReg | MachineOpcode::AddRegReg | MachineOpcode::SubRegReg
        | MachineOpcode::AndRegReg | MachineOpcode::OrRegReg | MachineOpcode::XorRegReg
        | MachineOpcode::CmpRegReg => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let src = reg_at(instr, 1, "source")?;
            let op: u8 = match instr.opcode {
                MachineOpcode::MovRegReg => 0x89,
                MachineOpcode::AddRegReg => 0x01,
                MachineOpcode::SubRegReg => 0x29,
                MachineOpcode::AndRegReg => 0x21,
                MachineOpcode::OrRegReg => 0x09,
                MachineOpcode::XorRegReg => 0x31,
                _ => 0x39,
            };
            push_rex(out, true, src.0 >= 8, false, dst.0 >= 8, false);
            out.push(op);
            out.push((0b11 << 6) | (r3(src) << 3) | r3(dst)); // r/m <- reg
            Ok(None)
        }
        MachineOpcode::MovRegImm32 | MachineOpcode::AddRegImm32 | MachineOpcode::SubRegImm32
        | MachineOpcode::CmpRegImm32 => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let v = imm_at(instr, 1, i32::MIN as i64, i32::MAX as i64)?;
            // mov uses C7 /0 (MOV r/m64, imm32 sign-extended); the ALU group shares
            // 0x81 with its own /ext digits. NOTE: C7 /7 is XBEGIN, never MOV.
            let (op, ext): (u8, u8) = match instr.opcode {
                MachineOpcode::MovRegImm32 => (0xC7, 0),
                MachineOpcode::AddRegImm32 => (0x81, 0),
                MachineOpcode::SubRegImm32 => (0x81, 5),
                _ => (0x81, 7),
            };
            push_rex(out, true, false, false, dst.0 >= 8, false);
            out.push(op);
            out.push((0b11 << 6) | (ext << 3) | r3(dst));
            out.extend_from_slice(&(v as i32).to_le_bytes());
            Ok(None)
        }
        MachineOpcode::ImulRegReg => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let src = reg_at(instr, 1, "source")?;
            push_rex(out, true, dst.0 >= 8, false, src.0 >= 8, false);
            out.extend_from_slice(&[0x0F, 0xAF]);
            out.push((0b11 << 6) | (r3(dst) << 3) | r3(src)); // dst <- dst * src
            Ok(None)
        }
        MachineOpcode::ShlRegImm8 | MachineOpcode::SarRegImm8 => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let v = imm_at(instr, 1, 0, 63)?;
            let ext: u8 = if instr.opcode == MachineOpcode::ShlRegImm8 { 4 } else { 7 };
            push_rex(out, true, false, false, dst.0 >= 8, false);
            out.push(0xC1);
            out.push((0b11 << 6) | (ext << 3) | r3(dst));
            out.push(v as u8);
            Ok(None)
        }
        MachineOpcode::XorRegSelf => {
            check_arity(instr, 1)?;
            let dst = reg_at(instr, 0, "destination")?;
            push_rex(out, true, false, false, dst.0 >= 8, false);
            out.push(0x31);
            out.push((0b11 << 6) | (r3(dst) << 3) | r3(dst)); // xor r/m, r with r == r/m
            Ok(None)
        }
        MachineOpcode::Lea => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let mem = encode_mem(&mem_at(instr, 1, "address")?)?;
            push_rex(out, true, dst.0 >= 8, mem.rex_x, mem.rex_b, false);
            out.push(0x8D);
            mem.append_to(r3(dst), out);
            Ok(None)
        }
        MachineOpcode::MovRegMem | MachineOpcode::LoadStack => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let mem = encode_mem(&mem_at(instr, 1, "source")?)?;
            push_rex(out, true, dst.0 >= 8, mem.rex_x, mem.rex_b, false);
            out.push(0x8B);
            mem.append_to(r3(dst), out);
            Ok(None)
        }
        MachineOpcode::MovMemReg | MachineOpcode::StoreStack => {
            check_arity(instr, 2)?;
            let src = reg_at(instr, 1, "source")?;
            let mem = encode_mem(&mem_at(instr, 0, "destination")?)?;
            push_rex(out, true, src.0 >= 8, mem.rex_x, mem.rex_b, false);
            out.push(0x89);
            mem.append_to(r3(src), out);
            Ok(None)
        }
        MachineOpcode::StoreStackImm => {
            // `mov qword [mem], imm32`: REX.W + C7 /0 + ModRM(0) + imm32. The
            // immediate is sign-extended to 64 bits, which is exactly the
            // MovRegImm32 spill contract; no register is touched, so peak-pressure
            // definitions spill without a scratch.
            check_arity(instr, 2)?;
            let mem = encode_mem(&mem_at(instr, 0, "destination")?)?;
            let imm = imm_at(
                instr,
                1,
                i32::MIN as i64,
                i32::MAX as i64,
            )?;
            let imm32 = imm as i32;
            push_rex(out, true, false, mem.rex_x, mem.rex_b, false);
            out.push(0xC7);
            mem.append_to(0, out);
            out.extend_from_slice(&imm32.to_le_bytes());
            Ok(None)
        }
        MachineOpcode::Setcc(kind) => {
            check_arity(instr, 1)?;
            let dst = reg_at(instr, 0, "destination")?;
            // setcc writes a byte register: REX is required to reach spl/bpl/sil/dil
            // and to address r8b..r15b, so it is always emitted.
            push_rex(out, false, false, false, dst.0 >= 8, true);
            out.extend_from_slice(&[0x0F, 0x90 | cc_nibble(kind)]);
            out.push((0b11 << 6) | r3(dst));
            Ok(None)
        }
        MachineOpcode::Jmp => {
            check_arity(instr, 0)?;
            let disp = branch_disp.ok_or_else(|| BackendError::Verification {
                detail: "jmp without a resolved target".into(),
            })?;
            out.push(0xE9);
            out.extend_from_slice(&disp.to_le_bytes());
            Ok(None)
        }
        MachineOpcode::Jcc(kind) => {
            check_arity(instr, 0)?;
            let disp = branch_disp.ok_or_else(|| BackendError::Verification {
                detail: "jcc without a resolved target".into(),
            })?;
            out.extend_from_slice(&[0x0F, 0x80 | cc_nibble(kind)]);
            out.extend_from_slice(&disp.to_le_bytes());
            Ok(None)
        }
        MachineOpcode::Call => {
            if instr.operands.is_empty() {
                return Err(shape("call needs at least a target immediate".into()));
            }
            let (rest, last) = instr.operands.split_at(instr.operands.len() - 1);
            for (oi, op) in rest.iter().enumerate() {
                if !matches!(op, MachineOperand::Reg(_)) {
                    return Err(shape(format!("call argument {oi} must be a register")));
                }
            }
            let target = match last[0] {
                MachineOperand::Imm(t) => t,
                ref other => {
                    return Err(shape(format!(
                        "call target must be an immediate, got {other:?}"
                    )))
                }
            };
            let disp = branch_disp.unwrap_or(0); // placeholder: patched by the linker reloc
            out.push(0xE8);
            let at = out.len() as u64;
            out.extend_from_slice(&disp.to_le_bytes());
            Ok(Some(PendingReloc {
                at,
                kind: RelocKind::PcRel32,
                symbol: call_symbol_name(target),
                addend: -4,
            }))
        }
        MachineOpcode::Ret => {
            check_arity(instr, 0)?;
            out.push(0xC3);
            Ok(None)
        }
        MachineOpcode::Push | MachineOpcode::Pop => {
            check_arity(instr, 1)?;
            let r = reg_at(instr, 0, "stack")?;
            let base: u8 = if instr.opcode == MachineOpcode::Push { 0x50 } else { 0x58 };
            push_rex(out, false, false, false, r.0 >= 8, false);
            out.push(base + r3(r));
            Ok(None)
        }
        MachineOpcode::Cqo => {
            check_arity(instr, 0)?;
            out.extend_from_slice(&[0x48, 0x99]); // REX.W 99: sign-extend rax into rdx:rax
            Ok(None)
        }
        MachineOpcode::IdivReg => {
            check_arity(instr, 1)?;
            let r = reg_at(instr, 0, "divisor")?;
            push_rex(out, true, false, false, r.0 >= 8, false);
            out.push(0xF7);
            out.push((0b11 << 6) | (7 << 3) | r3(r));
            Ok(None)
        }
        MachineOpcode::MovsdRegReg => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let src = reg_at(instr, 1, "source")?;
            // Legacy prefix first, REX immediately before the opcode (x86 prefix order).
            // No REX.W: the F2 prefix fixes the operand width (64-bit double).
            out.push(0xF2);
            push_rex(out, false, rex_ext(dst), false, rex_ext(src), false);
            out.extend_from_slice(&[0x0F, 0x10]);
            out.push((0b11 << 6) | (r3(dst) << 3) | r3(src));
            Ok(None)
        }
        MachineOpcode::CvtSi2sd => {
            check_arity(instr, 2)?;
            let dst = reg_at(instr, 0, "destination")?;
            let src = reg_at(instr, 1, "source")?;
            // Legacy prefix first, then REX.W (64-bit integer source form of 0F 2A /r).
            out.push(0xF2);
            push_rex(out, true, rex_ext(dst), false, rex_ext(src), false);
            out.extend_from_slice(&[0x0F, 0x2A]);
            out.push((0b11 << 6) | (r3(dst) << 3) | r3(src));
            Ok(None)
        }
        MachineOpcode::Syscall => {
            check_arity(instr, 0)?;
            out.extend_from_slice(&[0x0F, 0x05]);
            Ok(None)
        }
        MachineOpcode::Fence => {
            check_arity(instr, 0)?;
            out.extend_from_slice(&[0x0F, 0xAE, 0xF0]); // mfence
            Ok(None)
        }
    }
}

/// CEP:WHAT: Validates one instruction's operand shapes before any bytes are emitted:
/// arity, operand kinds, immediate ranges (i32 for imm32 forms, 0..=63 for shifts),
/// register classes via the machir signature table, memory-operand legality (encode_mem),
/// and terminator placement (only the last instruction of a block may transfer control).
/// CEP:WHY: The encoder must be a total, panic-free function over arbitrary (possibly
/// unverified) MIR; validation happens before layout so failures precede any emission.
/// CEP:STATUS: complete
/// CEP:FAILURE: the BackendError variants documented at module level.
/// CEP:ASSUMES: none
/// CEP:COST: O(operands).
/// CEP:EVIDENCE: negative tests (scale, immediate, terminator placement).
fn validate_instr(instr: &MachineInstr, is_last: bool) -> Result<(), BackendError> {
    let shape = |detail: String| BackendError::IllegalOperandShape {
        opcode: opcode_name(instr.opcode),
        detail,
    };
    match instr.opcode {
        MachineOpcode::MovRegReg | MachineOpcode::AddRegReg | MachineOpcode::SubRegReg
        | MachineOpcode::AndRegReg | MachineOpcode::OrRegReg | MachineOpcode::XorRegReg
        | MachineOpcode::CmpRegReg | MachineOpcode::ImulRegReg | MachineOpcode::MovsdRegReg
        | MachineOpcode::CvtSi2sd => {
            check_arity(instr, 2)?;
            reg_at(instr, 0, "destination")?;
            reg_at(instr, 1, "source")?;
        }
        MachineOpcode::MovRegImm32 | MachineOpcode::AddRegImm32 | MachineOpcode::SubRegImm32
        | MachineOpcode::CmpRegImm32 => {
            check_arity(instr, 2)?;
            reg_at(instr, 0, "destination")?;
            imm_at(instr, 1, i32::MIN as i64, i32::MAX as i64)?;
        }
        MachineOpcode::ShlRegImm8 | MachineOpcode::SarRegImm8 => {
            check_arity(instr, 2)?;
            reg_at(instr, 0, "destination")?;
            imm_at(instr, 1, 0, 63)?;
        }
        MachineOpcode::XorRegSelf => {
            check_arity(instr, 1)?;
            reg_at(instr, 0, "destination")?;
        }
        MachineOpcode::Lea => {
            check_arity(instr, 2)?;
            reg_at(instr, 0, "destination")?;
            encode_mem(&mem_at(instr, 1, "address")?)?;
        }
        MachineOpcode::MovRegMem | MachineOpcode::LoadStack => {
            check_arity(instr, 2)?;
            reg_at(instr, 0, "destination")?;
            encode_mem(&mem_at(instr, 1, "source")?)?;
        }
        MachineOpcode::MovMemReg | MachineOpcode::StoreStack => {
            check_arity(instr, 2)?;
            encode_mem(&mem_at(instr, 0, "destination")?)?;
            reg_at(instr, 1, "source")?;
        }
        MachineOpcode::StoreStackImm => {
            check_arity(instr, 2)?;
            encode_mem(&mem_at(instr, 0, "destination")?)?;
            imm_at(instr, 1, i32::MIN as i64, i32::MAX as i64)?;
        }
        MachineOpcode::Setcc(_) => {
            check_arity(instr, 1)?;
            reg_at(instr, 0, "destination")?;
        }
        MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret | MachineOpcode::Cqo
        | MachineOpcode::Syscall | MachineOpcode::Fence => {
            check_arity(instr, 0)?;
        }
        MachineOpcode::Call => {
            if instr.operands.is_empty() {
                return Err(shape("call needs at least a target immediate".into()));
            }
            let (rest, last) = instr.operands.split_at(instr.operands.len() - 1);
            for (oi, op) in rest.iter().enumerate() {
                if !matches!(op, MachineOperand::Reg(_)) {
                    return Err(shape(format!("call argument {oi} must be a register")));
                }
            }
            if !matches!(last[0], MachineOperand::Imm(_)) {
                return Err(shape("call target must be an immediate".into()));
            }
        }
        MachineOpcode::Push | MachineOpcode::Pop | MachineOpcode::IdivReg => {
            check_arity(instr, 1)?;
            reg_at(instr, 0, "register")?;
        }
    }
    if !is_last && matches!(
        instr.opcode,
        MachineOpcode::Jmp | MachineOpcode::Jcc(_) | MachineOpcode::Ret
    ) {
        return Err(BackendError::Verification {
            detail: format!("{} is not the last instruction of its block", opcode_name(instr.opcode)),
        });
    }
    Ok(())
}

/// CEP:WHAT: Emits one function: pass 0 validates every instruction; pass 1 sizes each
/// instruction (branch displacement fixed at rel32, so sizes are stable) and derives the
/// block offsets; pass 2 emits with resolved internal branch displacements
/// (disp = block_offset\[target\] - end_of_instruction) and instruction-relative
/// relocations lifted to code offsets; finally the code is padded with 0x90 to a
/// multiple of ENTRY_ALIGNMENT.
/// CEP:WHY: The pinned public contract (spec 5.4: MIR in, machine code out).
/// CEP:STATUS: complete
/// CEP:FAILURE: see the module CEP:FAILURE list; the function is total over any MIR.
/// CEP:ASSUMES: the module ASSUMES (layout order, successor conventions, call ordinals).
/// CEP:COST: three linear passes; O(code bytes).
/// CEP:EVIDENCE: every test in this module plus the compile_lir pipeline tests.
pub fn emit_function(mir: &MachineIrFunction, symbol: &str) -> Result<CodegenOutput, BackendError> {
    if mir.isa != Isa::X86_64 {
        return Err(BackendError::UnsupportedIsa {
            isa: match mir.isa {
                Isa::Aarch64 => "Aarch64",
                Isa::X86_64 => "x86-64",
            },
        });
    }

    // Pass 0: validation (fail before writing any bytes).
    for block in &mir.blocks {
        if block.instructions.is_empty() {
            return Err(BackendError::Verification { detail: "empty block has no terminator".into() });
        }
        let last = block.instructions.len() - 1;
        for (ii, instr) in block.instructions.iter().enumerate() {
            validate_instr(instr, ii == last)?;
        }
    }

    // Pass 1: sizes and block offsets (branch displacement assumed rel32 = 4 bytes).
    let mut block_offset = vec![0u64; mir.blocks.len()];
    let mut sizes: Vec<Vec<u64>> = Vec::with_capacity(mir.blocks.len());
    let mut cursor: u64 = 0;
    let mut scratch: Vec<u8> = Vec::new();
    for (bi, block) in mir.blocks.iter().enumerate() {
        block_offset[bi] = cursor;
        let mut block_sizes = Vec::with_capacity(block.instructions.len());
        for instr in &block.instructions {
            scratch.clear();
            let placeholder = matches!(instr.opcode, MachineOpcode::Jmp | MachineOpcode::Jcc(_));
            encode_instr(instr, if placeholder { Some(0) } else { None }, &mut scratch)?;
            block_sizes.push(scratch.len() as u64);
            cursor += scratch.len() as u64;
        }
        sizes.push(block_sizes);
    }

    // Pass 2: emit with resolved branches.
    let mut code: Vec<u8> = Vec::with_capacity(cursor as usize);
    let mut relocs: Vec<Relocation> = Vec::new();
    for (bi, block) in mir.blocks.iter().enumerate() {
        for (ii, instr) in block.instructions.iter().enumerate() {
            let instr_off = code.len() as u64;
            let branch_disp = match instr.opcode {
                MachineOpcode::Jmp | MachineOpcode::Jcc(_) => {
                    let target = *block.successors.first().ok_or_else(|| {
                        BackendError::Verification {
                            detail: format!("block {bi} branches without a successor"),
                        }
                    })? as usize;
                    if target >= mir.blocks.len() {
                        return Err(BackendError::Verification {
                            detail: format!("block {bi} branches out of range"),
                        });
                    }
                    if let MachineOpcode::Jcc(_) = instr.opcode {
                        if block.successors.len() > 1 {
                            let fallthrough = block.successors[1] as usize;
                            if fallthrough != bi + 1 {
                                return Err(BackendError::Verification {
                                    detail: format!(
                                        "block {bi} jcc fallthrough successor {fallthrough} \
                                         does not match layout order (next block {})",
                                        bi + 1
                                    ),
                                });
                            }
                        }
                    }
                    let end = instr_off + sizes[bi][ii];
                    let disp = block_offset[target] as i64 - end as i64;
                    if !(i32::MIN as i64..=i32::MAX as i64).contains(&disp) {
                        return Err(BackendError::Verification {
                            detail: format!("branch displacement {disp} outside rel32"),
                        });
                    }
                    Some(disp as i32)
                }
                _ => None,
            };
            let before = code.len();
            let pending = encode_instr(instr, branch_disp, &mut code)?;
            debug_assert_eq!(code.len() - before, sizes[bi][ii] as usize, "size stability");
            if let Some(p) = pending {
                relocs.push(Relocation {
                    // p.at was captured from the function-wide buffer (absolute field
                    // offset, one byte past the E8 opcode); no instruction base added.
                    offset: p.at,
                    kind: p.kind,
                    symbol: p.symbol,
                    addend: p.addend,
                });
            }
        }
    }

    // Entry alignment: deterministic 0x90 padding to a 16-byte multiple.
    let aligned = (code.len() as u64).div_ceil(ENTRY_ALIGNMENT) * ENTRY_ALIGNMENT;
    code.resize(aligned as usize, NOP_PAD);

    Ok(CodegenOutput {
        symbol: symbol.to_string(),
        code,
        relocs,
        frame: mir.frame.clone(),
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::abi;
    use rv8_ir::lir::{LirBlock, LirFunction, LirInstruction, RegClassId, ValueId};
    use rv8_ir::machir::{MachineBlock, MachineFrame, MemOperand};
    use rv8_ir::son::{BlockId, IrType};
    use MachineOperand::{Imm, Reg};

    /// CEP:WHAT: Builds a hand-written MIR function with the given blocks/frame.
    /// CEP:WHY: Test brevity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn mir(name: &str, blocks: Vec<MachineBlock>, frame: MachineFrame) -> MachineIrFunction {
        MachineIrFunction { name: name.into(), isa: Isa::X86_64, blocks, frame }
    }

    /// CEP:WHAT: Default ABI-conformant test frame.
    /// CEP:WHY: Test brevity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn frame(size: u32) -> MachineFrame {
        MachineFrame {
            stack_size: size,
            alignment: 16,
            has_red_zone: false,
            saved_registers: Vec::new(),
        }
    }

    /// CEP:WHAT: Pins the RelocKind -> ELF type mapping (pinned cross-agent contract).
    /// CEP:WHY: rv8_object mirrors this structurally; drift must be caught here first.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn reloc_kind_elf_type_mapping() {
        assert_eq!(RelocKind::Abs64.elf_type(), 1);
        assert_eq!(RelocKind::PcRel32.elf_type(), 2);
    }

    /// CEP:WHAT: Pins exact bytes for the register-register and immediate ALU families,
    /// xor-self, shifts, imul, cqo, idiv, push/pop (with and without REX.B), and
    /// movsd/cvtsi2sd/syscall/fence.
    /// CEP:WHY: The encoder is the last gate before bytes; each form is pinned once.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn opcode_families_encode_exact_bytes() {
        let enc = |op: MachineOpcode, ops: Vec<MachineOperand>| -> Vec<u8> {
            let mut out = Vec::new();
            encode_instr(&MachineInstr { opcode: op, operands: ops, predicates: Vec::new() }, None, &mut out)
                .expect("encodes");
            out
        };
        // mov rax, rbx: 48 89 D8 (89 /r: modrm 11 011 000)
        assert_eq!(enc(MachineOpcode::MovRegReg, vec![Reg(abi::RAX), Reg(abi::RBX)]),
            vec![0x48, 0x89, 0xD8]);
        // mov r15, r8: 4D 89 C7 (REX.WRB, modrm 11 000 111)
        assert_eq!(enc(MachineOpcode::MovRegReg, vec![Reg(PhysReg(15)), Reg(abi::R8)]),
            vec![0x4D, 0x89, 0xC7]);
        // mov rcx, 7 (C7 /0 imm32 sign-extended): 48 C7 C1 07000000; /7 would be XBEGIN
        assert_eq!(enc(MachineOpcode::MovRegImm32, vec![Reg(abi::RCX), Imm(7)]),
            vec![0x48, 0xC7, 0xC1, 0x07, 0x00, 0x00, 0x00]);
        // add rax, rdi: 48 01 F8
        assert_eq!(enc(MachineOpcode::AddRegReg, vec![Reg(abi::RAX), Reg(abi::RDI)]),
            vec![0x48, 0x01, 0xF8]);
        // add rdi, 1000: 48 81 C7 E8030000
        assert_eq!(enc(MachineOpcode::AddRegImm32, vec![Reg(abi::RDI), Imm(1000)]),
            vec![0x48, 0x81, 0xC7, 0xE8, 0x03, 0x00, 0x00]);
        // sub rsp, 8: 48 81 EC 08000000
        assert_eq!(enc(MachineOpcode::SubRegImm32, vec![Reg(abi::RSP), Imm(8)]),
            vec![0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00]);
        // sub r9, rdx: 49 29 D1 (REX.W+B: r9 is the rm field, rdx the reg field needs
        // no R bit; modrm 11 010 001). The 0x4C form would wrongly set REX.R+X.
        assert_eq!(enc(MachineOpcode::SubRegReg, vec![Reg(abi::R9), Reg(abi::RDX)]),
            vec![0x49, 0x29, 0xD1]);
        // imul rax, rcx: 48 0F AF C1
        assert_eq!(enc(MachineOpcode::ImulRegReg, vec![Reg(abi::RAX), Reg(abi::RCX)]),
            vec![0x48, 0x0F, 0xAF, 0xC1]);
        // shl rax, 3: 48 C1 E0 03
        assert_eq!(enc(MachineOpcode::ShlRegImm8, vec![Reg(abi::RAX), Imm(3)]),
            vec![0x48, 0xC1, 0xE0, 0x03]);
        // sar r11, 1: 49 C1 FB 01
        assert_eq!(enc(MachineOpcode::SarRegImm8, vec![Reg(PhysReg(11)), Imm(1)]),
            vec![0x49, 0xC1, 0xFB, 0x01]);
        // and rax, rdx: 48 21 D0; or rax, rdx: 48 09 D0; xor rax, rdx: 48 31 D0
        assert_eq!(enc(MachineOpcode::AndRegReg, vec![Reg(abi::RAX), Reg(abi::RDX)]),
            vec![0x48, 0x21, 0xD0]);
        assert_eq!(enc(MachineOpcode::OrRegReg, vec![Reg(abi::RAX), Reg(abi::RDX)]),
            vec![0x48, 0x09, 0xD0]);
        assert_eq!(enc(MachineOpcode::XorRegReg, vec![Reg(abi::RAX), Reg(abi::RDX)]),
            vec![0x48, 0x31, 0xD0]);
        // xor eax, eax (zero idiom): 48 31 C0
        assert_eq!(enc(MachineOpcode::XorRegSelf, vec![Reg(abi::RAX)]), vec![0x48, 0x31, 0xC0]);
        // cmp rax, rdi: 48 39 F8; cmp rdi, 5: 48 81 FF 05000000
        assert_eq!(enc(MachineOpcode::CmpRegReg, vec![Reg(abi::RAX), Reg(abi::RDI)]),
            vec![0x48, 0x39, 0xF8]);
        assert_eq!(enc(MachineOpcode::CmpRegImm32, vec![Reg(abi::RDI), Imm(5)]),
            vec![0x48, 0x81, 0xFF, 0x05, 0x00, 0x00, 0x00]);
        // push rbp: 55; push r12: 41 54; pop rbp: 5D; pop r13: 41 5D
        assert_eq!(enc(MachineOpcode::Push, vec![Reg(abi::RBP)]), vec![0x55]);
        assert_eq!(enc(MachineOpcode::Push, vec![Reg(PhysReg(12))]), vec![0x41, 0x54]);
        assert_eq!(enc(MachineOpcode::Pop, vec![Reg(abi::RBP)]), vec![0x5D]);
        assert_eq!(enc(MachineOpcode::Pop, vec![Reg(PhysReg(13))]), vec![0x41, 0x5D]);
        // cqo: 48 99; idiv rbx: 48 F7 FB
        assert_eq!(enc(MachineOpcode::Cqo, vec![]), vec![0x48, 0x99]);
        assert_eq!(enc(MachineOpcode::IdivReg, vec![Reg(abi::RBX)]), vec![0x48, 0xF7, 0xFB]);
        // movsd xmm1, xmm0: F2 0F 10 C8 (no REX: neither operand is xmm8..15; the
        // destination sits in the reg field, the source in rm).
        assert_eq!(enc(MachineOpcode::MovsdRegReg, vec![Reg(PhysReg(17)), Reg(PhysReg(16))]),
            vec![0xF2, 0x0F, 0x10, 0xC8]);
        // movsd xmm9, xmm8: F2 45 0F 10 C8 (REX.R for the reg field, REX.B for rm).
        assert_eq!(enc(MachineOpcode::MovsdRegReg, vec![Reg(PhysReg(25)), Reg(PhysReg(24))]),
            vec![0xF2, 0x45, 0x0F, 0x10, 0xC8]);
        // cvtsi2sd xmm0, rdi: F2 48 0F 2A C7
        assert_eq!(enc(MachineOpcode::CvtSi2sd, vec![Reg(PhysReg(16)), Reg(abi::RDI)]),
            vec![0xF2, 0x48, 0x0F, 0x2A, 0xC7]);
        // syscall: 0F 05; fence (mfence): 0F AE F0; ret: C3
        assert_eq!(enc(MachineOpcode::Syscall, vec![]), vec![0x0F, 0x05]);
        assert_eq!(enc(MachineOpcode::Fence, vec![]), vec![0x0F, 0xAE, 0xF0]);
        assert_eq!(enc(MachineOpcode::Ret, vec![]), vec![0xC3]);
    }

    /// CEP:WHAT: Pins setcc and jcc condition nibbles against the x86 map
    /// (0F 94/95/9C/9D/9E/9F and 0F 84/85/8C/8D/8E/8F).
    /// CEP:WHY: A swapped condition code silently flips branches.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn setcc_and_jcc_condition_map() {
        let kinds = [
            (SetKind::Eq, 0x94u8, 0x84u8),
            (SetKind::Ne, 0x95, 0x85),
            (SetKind::Lt, 0x9C, 0x8C),
            (SetKind::Ge, 0x9D, 0x8D),
            (SetKind::Le, 0x9E, 0x8E),
            (SetKind::Gt, 0x9F, 0x8F),
        ];
        for (kind, set_op, jcc_op) in kinds {
            let mut out = Vec::new();
            encode_instr(
                &MachineInstr {
                    opcode: MachineOpcode::Setcc(kind),
                    operands: vec![Reg(abi::RAX)],
                    predicates: Vec::new(),
                },
                None,
                &mut out,
            )
            .expect("setcc");
            assert_eq!(out, vec![0x40, 0x0F, set_op, 0xC0], "{kind:?}");
            let mut out = Vec::new();
            encode_instr(
                &MachineInstr { opcode: MachineOpcode::Jcc(kind), operands: vec![], predicates: Vec::new() },
                Some(0x12345678u32 as i32),
                &mut out,
            )
            .expect("jcc");
            assert_eq!(out, vec![0x0F, jcc_op, 0x78, 0x56, 0x34, 0x12], "{kind:?}");
        }
        // jmp: E9 rel32
        let mut out = Vec::new();
        encode_instr(
            &MachineInstr { opcode: MachineOpcode::Jmp, operands: vec![], predicates: Vec::new() },
            Some(-16),
            &mut out,
        )
        .expect("jmp");
        assert_eq!(out, vec![0xE9, 0xF0, 0xFF, 0xFF, 0xFF]);
    }

    /// CEP:WHAT: Pins ModRM/SIB encoding across the addressing-mode matrix: [rax],
    /// [rbx+disp8], [rsp+disp8/disp32] (SIB), [rbp+0] (forced disp8), [rdi+rsi*8+16],
    /// absolute [disp32], and base-less scaled [rsi*4+8].
    /// CEP:WHY: Addressing-mode bytes are the most error-prone encoding surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn modrm_sib_encoding() {
        let load = |m: MemOperand| -> Vec<u8> {
            let mut out = Vec::new();
            encode_instr(
                &MachineInstr {
                    opcode: MachineOpcode::MovRegMem,
                    operands: vec![Reg(abi::RAX), MachineOperand::Mem(m)],
                    predicates: Vec::new(),
                },
                None,
                &mut out,
            )
            .expect("encodes");
            out
        };
        let mem = |base: Option<PhysReg>, index: Option<PhysReg>, scale: u8, disp: i32| MemOperand {
            base,
            index,
            scale,
            disp,
        };
        // mov rax, [rax]: 48 8B 00
        assert_eq!(load(mem(Some(abi::RAX), None, 1, 0)), vec![0x48, 0x8B, 0x00]);
        // mov rax, [rbx+8]: 48 8B 43 08
        assert_eq!(load(mem(Some(abi::RBX), None, 1, 8)), vec![0x48, 0x8B, 0x43, 0x08]);
        // mov rax, [rbx-1]: 48 8B 43 FF
        assert_eq!(load(mem(Some(abi::RBX), None, 1, -1)), vec![0x48, 0x8B, 0x43, 0xFF]);
        // mov rax, [rsp+8] (SIB, no index): 48 8B 44 24 08
        assert_eq!(load(mem(Some(abi::RSP), None, 1, 8)), vec![0x48, 0x8B, 0x44, 0x24, 0x08]);
        // mov rax, [rsp+1000] (SIB, disp32): 48 8B 84 24 E8030000
        assert_eq!(
            load(mem(Some(abi::RSP), None, 1, 1000)),
            vec![0x48, 0x8B, 0x84, 0x24, 0xE8, 0x03, 0x00, 0x00]
        );
        // mov rax, [rsp] (SIB, no disp): 48 8B 04 24
        assert_eq!(load(mem(Some(abi::RSP), None, 1, 0)), vec![0x48, 0x8B, 0x04, 0x24]);
        // mov rax, [rbp+0]: forced disp8: 48 8B 45 00
        assert_eq!(load(mem(Some(abi::RBP), None, 1, 0)), vec![0x48, 0x8B, 0x45, 0x00]);
        // mov rax, [rbp-8]: 48 8B 45 F8
        assert_eq!(load(mem(Some(abi::RBP), None, 1, -8)), vec![0x48, 0x8B, 0x45, 0xF8]);
        // mov rax, [rdi+rsi*8+16]: 48 8B 44 F7 10
        assert_eq!(
            load(mem(Some(abi::RDI), Some(abi::RSI), 8, 16)),
            vec![0x48, 0x8B, 0x44, 0xF7, 0x10]
        );
        // mov rax, [rdi+r9*2-4]: REX.XB: 4C? dst rax -> 4A? W=1 R=0 X=1 B=0 -> 4A 8B 44 4F FC
        assert_eq!(
            load(mem(Some(abi::RDI), Some(PhysReg(9)), 2, -4)),
            vec![0x4A, 0x8B, 0x44, 0x4F, 0xFC]
        );
        // mov rax, [rsi*4+8]: SIB base=101: 48 8B 04 B5 08000000
        assert_eq!(
            load(mem(None, Some(abi::RSI), 4, 8)),
            vec![0x48, 0x8B, 0x04, 0xB5, 0x08, 0x00, 0x00, 0x00]
        );
        // store: mov [rax], rdi: 48 89 38
        let mut out = Vec::new();
        encode_instr(
            &MachineInstr {
                opcode: MachineOpcode::MovMemReg,
                operands: vec![
                    MachineOperand::Mem(mem(Some(abi::RAX), None, 1, 0)),
                    Reg(abi::RDI),
                ],
                predicates: Vec::new(),
            },
            None,
            &mut out,
        )
        .expect("encodes");
        assert_eq!(out, vec![0x48, 0x89, 0x38]);
    }

    /// CEP:WHAT: Negative shapes: scale 3 and rsp-as-index are InvalidAddressing;
    /// shift count 64 and imm32 overflow are IllegalOperandShape; AArch64 MIR is
    /// UnsupportedIsa; a mid-block jmp is Verification.
    /// CEP:WHY: Fail-closed over corrupt MIR; the encoder never guesses or panics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn encode_rejects_invalid_operands() {
        let bad_scale = MemOperand { base: Some(abi::RAX), index: Some(abi::RDI), scale: 3, disp: 0 };
        match encode_mem(&bad_scale) {
            Err(BackendError::InvalidAddressing { .. }) => {}
            other => panic!("expected InvalidAddressing, got {other:?}"),
        }
        let rsp_index = MemOperand { base: Some(abi::RAX), index: Some(abi::RSP), scale: 1, disp: 0 };
        assert!(matches!(
            encode_mem(&rsp_index),
            Err(BackendError::InvalidAddressing { .. })
        ));
        let mut out = Vec::new();
        let over_shift = MachineInstr {
            opcode: MachineOpcode::ShlRegImm8,
            operands: vec![Reg(abi::RAX), Imm(64)],
            predicates: Vec::new(),
        };
        assert!(matches!(
            encode_instr(&over_shift, None, &mut out),
            Err(BackendError::IllegalOperandShape { .. })
        ));
        let over_imm = MachineInstr {
            opcode: MachineOpcode::MovRegImm32,
            operands: vec![Reg(abi::RAX), Imm(1 << 40)],
            predicates: Vec::new(),
        };
        assert!(matches!(
            validate_instr(&over_imm, true),
            Err(BackendError::IllegalOperandShape { .. })
        ));
        // AArch64 MIR.
        let mut aa = MachineIrFunction::new("a", Isa::Aarch64);
        aa.push_block(MachineBlock {
            instructions: vec![MachineInstr {
                opcode: MachineOpcode::MovRegReg,
                operands: vec![Reg(PhysReg(0)), Reg(PhysReg(1))],
                predicates: Vec::new(),
            }],
            successors: vec![],
        });
        assert!(matches!(
            emit_function(&aa, "a"),
            Err(BackendError::UnsupportedIsa { .. })
        ));
        // Mid-block terminator.
        let mid = MachineInstr { opcode: MachineOpcode::Jmp, operands: vec![], predicates: Vec::new() };
        assert!(matches!(
            validate_instr(&mid, false),
            Err(BackendError::Verification { .. })
        ));
    }

    /// CEP:WHAT: Call encoding: e8 with an external PcRel32 relocation whose offset
    /// points at the rel32 field, addend -4, symbol fn{target}.
    /// CEP:WHY: The pinned relocation contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn call_relocation_is_pcrel32() {
        let f = mir(
            "caller",
            vec![MachineBlock {
                instructions: vec![
                    MachineInstr {
                        opcode: MachineOpcode::MovRegImm32,
                        operands: vec![Reg(abi::RDI), Imm(6)],
                        predicates: Vec::new(),
                    },
                    MachineInstr {
                        opcode: MachineOpcode::Call,
                        operands: vec![Reg(abi::RDI), Imm(1)],
                        predicates: Vec::new(),
                    },
                    MachineInstr { opcode: MachineOpcode::Ret, operands: vec![], predicates: Vec::new() },
                ],
                successors: vec![],
            }],
            frame(16),
        );
        f.verify().expect("valid");
        let out = emit_function(&f, "caller").expect("emits");
        assert_eq!(out.symbol, "caller");
        // mov rdi, 6 (C7 /0: modrm C7, 7 bytes) then call rel32 at offset 7.
        assert_eq!(&out.code[..7], &[0x48, 0xC7, 0xC7, 0x06, 0x00, 0x00, 0x00]);
        assert_eq!(out.code[7], 0xE8);
        assert_eq!(out.relocs.len(), 1);
        let r = &out.relocs[0];
        assert_eq!(r.offset, 8, "offset points at the rel32 field");
        assert_eq!(r.kind, RelocKind::PcRel32);
        assert_eq!(r.symbol, "fn1");
        assert_eq!(r.addend, -4);
        assert_eq!(out.frame.stack_size, 16);
        assert_eq!(out.code.len() % ENTRY_ALIGNMENT as usize, 0, "entry-aligned output");
    }

    /// CEP:WHAT: THE ABI end-to-end acceptance: hand-built LIR `fn add(a, b) = a + b`
    /// with RDI/RSI parameters and an RAX result compiled through the full pipeline;
    /// asserts the exact documented 32 bytes and the ABI-conformant frame.
    /// CEP:WHY: STANDARDS 5.9 acceptance; exact byte documentation for reviewers.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: SysV AMD64; the documented pipeline (isel lea, rsp-relative frame).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test (byte listing below).
    #[test]
    fn abi_add_end_to_end_bytes() {
        // LIR: params a(v0), b(v1); s(v2) = a + b; ret s.
        let mut lir = LirFunction::new("add");
        let a = lir.add_value(IrType::I64, RegClassId::Gpr);
        let b = lir.add_value(IrType::I64, RegClassId::Gpr);
        let s = lir.add_value(IrType::I64, RegClassId::Gpr);
        let mut blk = LirBlock::new(BlockId(0));
        let add = |ops: Vec<ValueId>, res: ValueId| LirInstruction {
            opcode: MachineOpcode::AddRegReg,
            operands: ops,
            result: Some(res),
            immediate: 0,
            target_block: None,
        };
        blk.push(add(vec![a, b], s));
        blk.push(LirInstruction {
            opcode: MachineOpcode::Ret,
            operands: vec![s],
            result: None,
            immediate: 0,
            target_block: None,
        });
        lir.push_block(blk);

        let out = crate::compile_lir(&lir, "add").expect("compiles");
        assert!(out.frame.alignment.is_power_of_two(), "alignment power of two");
        assert!(out.frame.alignment >= 16, "ABI minimum alignment");
        assert_eq!(out.frame.alignment, 16);
        assert_eq!(out.frame.stack_size % out.frame.alignment, 0, "frame size multiple");
        assert_eq!(out.frame.stack_size, 16, "one 8-byte adjustment padded to 16");
        assert_eq!(out.frame.saved_registers, Vec::new());
        assert_eq!(out.relocs.len(), 0, "leaf function: no calls, no relocations");

        // Exact documented bytes (see the CEP:EVIDENCE listing in this module's docs):
        //   0: 48 81 EC 08 00 00 00   sub rsp, 8            (frame: 16-byte ABI rhythm)
        //   7: 48 8D 04 37            lea rax, [rdi+rsi*1]  (isel folds add into lea)
        //  11: 48 81 C4 08 00 00 00   add rsp, 8            (epilogue)
        //  18: 48 89 C0               mov rax, rax          (return pin move)
        //  21: C3                     ret
        //  22..32: 0x90 x10                                (entry alignment padding)
        assert_eq!(
            out.code,
            vec![
                0x48, 0x81, 0xEC, 0x08, 0x00, 0x00, 0x00, // sub rsp, 8
                0x48, 0x8D, 0x04, 0x37, // lea rax, [rdi+rsi*1]
                0x48, 0x81, 0xC4, 0x08, 0x00, 0x00, 0x00, // add rsp, 8
                0x48, 0x89, 0xC0, // mov rax, rax
                0xC3, // ret
                0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, // pad to 32
            ]
        );
        assert_eq!(out.code.len(), 32);
        assert_eq!(out.symbol, "add");
    }

    /// CEP:WHAT: Diamond CFG branch encoding: the Jcc and Jmp rel32 displacements are
    /// decoded from the emitted bytes and checked against the block offsets derived
    /// from the same MIR the encoder laid out; the fallthrough successor must match
    /// layout order.
    /// CEP:WHY: Branch displacement arithmetic is the classic silent-corruption bug.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: [taken, fallthrough] successor convention.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test (offsets and displacements in the assertions).
    #[test]
    fn diamond_cfg_branch_displacements() {
        // b0: z = 0; cmp c, z; jne b2 (taken); fallthrough b1
        // b1: w = 1; jmp b2
        // b2: ret c
        let mut f = LirFunction::new("diamond");
        let c = f.add_value(IrType::I64, RegClassId::Gpr); // param
        let z = f.add_value(IrType::I64, RegClassId::Gpr);
        let w = f.add_value(IrType::I64, RegClassId::Gpr);
        let mut b0 = LirBlock::new(BlockId(0));
        b0.push(LirInstruction {
            opcode: MachineOpcode::MovRegImm32,
            operands: vec![],
            result: Some(z),
            immediate: 0,
            target_block: None,
        });
        b0.push(LirInstruction {
            opcode: MachineOpcode::CmpRegReg,
            operands: vec![c, z],
            result: None,
            immediate: 0,
            target_block: None,
        });
        b0.push(LirInstruction {
            opcode: MachineOpcode::Jcc(SetKind::Ne),
            operands: vec![],
            result: None,
            immediate: 0,
            target_block: Some(BlockId(2)),
        });
        b0.successors = vec![BlockId(2), BlockId(1)];
        let mut b1 = LirBlock::new(BlockId(1));
        b1.push(LirInstruction {
            opcode: MachineOpcode::MovRegImm32,
            operands: vec![],
            result: Some(w),
            immediate: 1,
            target_block: None,
        });
        b1.push(LirInstruction {
            opcode: MachineOpcode::Jmp,
            operands: vec![],
            result: None,
            immediate: 0,
            target_block: Some(BlockId(2)),
        });
        b1.successors = vec![BlockId(2)];
        let mut b2 = LirBlock::new(BlockId(2));
        b2.push(LirInstruction {
            opcode: MachineOpcode::Ret,
            operands: vec![c],
            result: None,
            immediate: 0,
            target_block: None,
        });
        f.push_block(b0);
        f.push_block(b1);
        f.push_block(b2);

        // Full pipeline (the encoder consumes scheduled, verified MIR).
        let mut m = crate::isel::select(&f).expect("isel");
        let alloc = crate::regalloc::allocate(&f).expect("alloc");
        crate::regalloc::rewrite(&mut m, &f, &alloc).expect("rewrite");
        crate::schedule::schedule(&mut m).expect("schedule");
        m.verify().expect("verified MIR");

        // Expected block offsets from the encoder's own sizing rules. The function is a
        // leaf with no spills, but the ABI frame rhythm still emits sub/add rsp, 8:
        //   b0 = [sub rsp,8 (7) | mov z,0 (7) | cmp (3) | jcc (6)]  -> b1 at 23
        //   b1 = [mov w,1 (7) | jmp (5)]                            -> b2 at 35
        //   b2 = [add rsp,8 (7) | mov rax,c (3) | ret (1)]
        // The jcc's taken target is b2 (skipping b1); its fallthrough is b1, the next
        // block in layout order, as the successor convention requires.
        let sizes: Vec<u64> = m
            .blocks
            .iter()
            .map(|b| b.instructions.iter().map(|i| instr_len(i).expect("size") as u64).sum())
            .collect();
        let off1: u64 = sizes[0];
        let off2: u64 = sizes[0] + sizes[1];
        assert_eq!(sizes[0], 23, "sub 7 + movimm 7 + cmp 3 + jcc 6");
        assert_eq!(sizes[1], 12, "movimm 7 + jmp 5");

        let out = crate::encode::emit_function(&m, "diamond").expect("emits");
        assert_eq!(out.code.len() % ENTRY_ALIGNMENT as usize, 0);

        // Locate the jcc (0F 85): it is the last instruction of block 0.
        let jcc_off = off1 - 6;
        assert_eq!(&out.code[jcc_off as usize..jcc_off as usize + 2], &[0x0F, 0x85], "jne");
        let disp = i32::from_le_bytes(
            out.code[jcc_off as usize + 2..jcc_off as usize + 6]
                .try_into()
                .expect("4 bytes"),
        );
        // rel32 = target - end_of_instruction. The jcc's TAKEN successor is block 2
        // (successors[0]), so the branch skips block 1 entirely: disp = 35 - 23 = 12
        // and the branch lands exactly on off2, never on the fallthrough off1.
        assert_eq!(
            jcc_off as i64 + 6 + disp as i64,
            off2 as i64,
            "jcc displacement lands exactly on block 2, its taken target (disp = {disp})"
        );
        assert_ne!(
            jcc_off as i64 + 6 + disp as i64,
            off1 as i64,
            "the taken branch must not land on the fallthrough block"
        );

        // Locate the jmp (E9): the last instruction of block 1.
        let jmp_off = off2 - 5;
        assert_eq!(out.code[jmp_off as usize], 0xE9);
        let disp = i32::from_le_bytes(
            out.code[jmp_off as usize + 1..jmp_off as usize + 5]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(
            jmp_off as i64 + 5 + disp as i64,
            off2 as i64,
            "jmp displacement lands exactly on block 2 (disp = {disp})"
        );

        // Determinism through the whole pipeline.
        let again = crate::compile_lir(&f, "diamond").expect("second compile");
        assert_eq!(again.code, out.code);
    }

    /// CEP:WHAT: Byte length of one instruction using the same encoder (test helper for
    /// layout arithmetic).
    /// CEP:WHY: Tests must derive offsets from the encoder, not duplicate it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: propagates encoding errors.
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn instr_len(instr: &MachineInstr) -> Result<usize, BackendError> {
        let mut out = Vec::new();
        let placeholder = matches!(instr.opcode, MachineOpcode::Jmp | MachineOpcode::Jcc(_));
        encode_instr(instr, if placeholder { Some(0) } else { None }, &mut out)?;
        Ok(out.len())
    }
}
