// CEP:FILE: crates/rv8_backend/src/abi.rs
// CEP:WHAT: The SysV AMD64 external C ABI model per STANDARDS 5.9: argument and return
//   register assignment, caller/callee-saved partition, physical-register numbering shared
//   with rv8_ir::machir, register-class tables for the MCF allocator, and the frame
//   calculator that turns an allocation into a conforming MachineFrame.
// CEP:WHY: Pre-coloring constrains the allocator (5.9); the frame contract (5.3.3) must be
//   computed once and shared by spill rewriting and the encoder so prologue, epilogue, and
//   frame metadata can never disagree.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete (stack-clash probing is an explicit stub, see AbiConfig)
// CEP:FAILURE: accessors return Option/Result and fail closed on unknown register ids.
// CEP:ASSUMES: PhysReg numbering equals rv8_ir::machir x86-64 numbering exactly:
//   0=rax 1=rcx 2=rdx 3=rbx 4=rsp 5=rbp 6=rsi 7=rdi 8..=15=r8..r15 16..=31=xmm0..xmm15.
//   This is read back from machir::reg_class/reg_name and must never drift.
// CEP:COST: all functions O(1) or O(n) over small constant register tables.
// CEP:EVIDENCE: unit tests abi_precolor_maps_args_and_return, frame_calculator_*.
// CEP:SECURITY: none specific; pure data tables.

use rv8_ir::lir::{RegClassId, RegId, RegisterClass, PhysicalRegister};
use rv8_ir::machir::{Isa, MachineFrame, PhysReg, reg_class, PhysClass};

/// CEP:WHAT: Static register-name table mirroring machir::reg_name for x86-64.
/// CEP:WHY: lir::PhysicalRegister.name is &'static str while machir::reg_name returns
///   String; the class builders need static names. The drift test
///   static_names_match_machir_reg_name keeps this table honest against the frozen
///   contract for all 32 real registers.
/// CEP:STATUS: complete
/// CEP:FAILURE: unknown numbers print as rN (same fallback as machir).
/// CEP:ASSUMES: machir numbering (see module ASSUMES).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: static_names_match_machir_reg_name.
pub fn static_reg_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        0 => "rax",
        1 => "rcx",
        2 => "rdx",
        3 => "rbx",
        4 => "rsp",
        5 => "rbp",
        6 => "rsi",
        7 => "rdi",
        8..=15 => match reg.0 {
            8 => "r8",
            9 => "r9",
            10 => "r10",
            11 => "r11",
            12 => "r12",
            13 => "r13",
            14 => "r14",
            _ => "r15",
        },
        16..=31 => match reg.0 {
            16 => "xmm0",
            17 => "xmm1",
            18 => "xmm2",
            19 => "xmm3",
            20 => "xmm4",
            21 => "xmm5",
            22 => "xmm6",
            23 => "xmm7",
            24 => "xmm8",
            25 => "xmm9",
            26 => "xmm10",
            27 => "xmm11",
            28 => "xmm12",
            29 => "xmm13",
            30 => "xmm14",
            _ => "xmm15",
        },
        _ => "r?",
    }
}

/// CEP:WHAT: x86-64 GPR numbers in machir encoding space (STANDARDS-conformant, verified
/// against machir::reg_name).
/// CEP:WHY: One source of truth for register numbers; every other module references these.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: machir numbering (see module ASSUMES).
/// CEP:COST: compile-time constants.
/// CEP:EVIDENCE: abi_register_numbering_matches_machir test.
pub const RAX: PhysReg = PhysReg(0);
pub const RCX: PhysReg = PhysReg(1);
pub const RDX: PhysReg = PhysReg(2);
pub const RBX: PhysReg = PhysReg(3);
pub const RSP: PhysReg = PhysReg(4);
pub const RBP: PhysReg = PhysReg(5);
pub const RSI: PhysReg = PhysReg(6);
pub const RDI: PhysReg = PhysReg(7);
pub const R8: PhysReg = PhysReg(8);
pub const R9: PhysReg = PhysReg(9);
pub const R10: PhysReg = PhysReg(10);
pub const R11: PhysReg = PhysReg(11);
pub const R12: PhysReg = PhysReg(12);
pub const R13: PhysReg = PhysReg(13);
pub const R14: PhysReg = PhysReg(14);
pub const R15: PhysReg = PhysReg(15);

/// CEP:WHAT: SysV AMD64 integer argument registers, in argument order.
/// CEP:WHY: STANDARDS 5.9 pre-coloring: integer args arrive in RDI,RSI,RDX,RCX,R8,R9.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: SysV AMD64 ABI (no register pairs, no KRB variant).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: abi_precolor_maps_args_and_return.
pub const ARG_REGISTERS: [PhysReg; 6] = [RDI, RSI, RDX, RCX, R8, R9];

/// CEP:WHAT: Primary integer return register (RAX).
/// CEP:WHY: STANDARDS 5.9; precolor_return pins the return value commodity to RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: integer-class returns only in this contract revision.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: abi_precolor_maps_args_and_return.
pub const RETURN_REGISTER: PhysReg = RAX;

/// CEP:WHAT: Secondary integer return register (RDX) for 128-bit / two-register returns.
/// CEP:WHY: SysV returns >64-bit integers in RAX:RDX; reserved here for future lowering.
/// CEP:STATUS: complete (declared; two-register returns not yet selected by isel)
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: abi_register_numbering_matches_machir.
pub const SECONDARY_RETURN_REGISTER: PhysReg = RDX;

/// CEP:WHAT: Callee-saved GPRs per SysV: RBX, RBP, R12, R13, R14, R15.
/// CEP:WHY: Values live across calls must sit in these or be spilled; the frame calculator
///   pushes exactly the set the allocator actually used.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: RSP is never in the allocatable set and never needs saving.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: frame_calculator_layout_matches_alignment_contract.
pub const CALLEE_SAVED_REGISTERS: [PhysReg; 6] = [RBX, RBP, R12, R13, R14, R15];

/// CEP:WHAT: Caller-saved GPRs: every allocatable GPR that is not callee-saved
/// (RAX, RCX, RDX, RSI, RDI, R8, R9, R10, R11).
/// CEP:WHY: Call sites clobber exactly this set; the MCF network blocks these lanes
/// through call points so live-across-call values cannot use them.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
pub const CALLER_SAVED_REGISTERS: [PhysReg; 9] = [RAX, RCX, RDX, RSI, RDI, R8, R9, R10, R11];

/// CEP:WHAT: Allocatable GPRs in RegId order: all 16 GPRs minus RSP (stack pointer is
/// reserved). RegId i of the GPR class maps to ALLOCATABLE_GPRS\[i\].
/// CEP:WHY: lir::RegId is dense within a class; this table is the single RegId->PhysReg
/// mapping shared by allocation, spill rewriting, and encoding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: RBP is allocatable because this backend keeps no frame pointer
/// (rsp-relative slots only).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: gpr_class_round_trips_through_machir_naming.
pub const ALLOCATABLE_GPRS: [PhysReg; 15] = [
    RAX, RCX, RDX, RBX, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14, R15,
];

/// CEP:WHAT: Allocatable FPRs in RegId order: xmm0..xmm15 (machir numbers 16..=31).
/// CEP:WHY: Same role as ALLOCATABLE_GPRS for the floating-point class.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: machir numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: gpr_class_round_trips_through_machir_naming.
pub const ALLOCATABLE_FPRS: [PhysReg; 16] = [
    PhysReg(16),
    PhysReg(17),
    PhysReg(18),
    PhysReg(19),
    PhysReg(20),
    PhysReg(21),
    PhysReg(22),
    PhysReg(23),
    PhysReg(24),
    PhysReg(25),
    PhysReg(26),
    PhysReg(27),
    PhysReg(28),
    PhysReg(29),
    PhysReg(30),
    PhysReg(31),
];

/// CEP:WHAT: SysV stack alignment: 16 bytes, power of two, the verifier minimum.
/// CEP:WHY: machir verify() rejects alignment < 16 or not a power of two (5.3.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: frame_calculator_layout_matches_alignment_contract.
pub const STACK_ALIGNMENT: u32 = 16;

/// CEP:WHAT: SysV red zone: 128 bytes below RSP that leaf functions may use without
/// adjusting RSP.
/// CEP:WHY: Leaf-only spill slots avoid the stack adjustment entirely.
/// CEP:STATUS: complete (modeled and used by FrameCalculator for leaf frames)
/// CEP:FAILURE: none
/// CEP:ASSUMES: signals do not interrupt in the RV8 execution contract for this revision;
///   kernels that interrupt user stacks would invalidate red-zone use.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: frame_calculator_uses_red_zone_for_small_leaf_frames.
pub const RED_ZONE_BYTES: u32 = 128;

/// CEP:WHAT: Slot width of every spill slot in bytes (one 64-bit slot).
/// CEP:WHY: Fixed slot width keeps offsets deterministic across the pipeline.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: only 64-bit integer slots in this revision; sub-64 values occupy a full slot.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: spill_path_inserts_loads_and_stores.
pub const SLOT_BYTES: u32 = 8;

/// CEP:WHAT: True when the register is one of the SysV callee-saved GPRs.
/// CEP:WHY: Save/restore cost modeling and prologue synthesis.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: frame_calculator_layout_matches_alignment_contract.
pub fn is_callee_saved(reg: PhysReg) -> bool {
    CALLEE_SAVED_REGISTERS.contains(&reg)
}

/// CEP:WHAT: True when the register is caller-saved (clobbered by calls).
/// CEP:WHY: The MCF network consults this to block lanes through call program points.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: register is an allocatable GPR.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: mcf_avoids_caller_saved_across_calls.
pub fn is_caller_saved(reg: PhysReg) -> bool {
    CALLER_SAVED_REGISTERS.contains(&reg)
}

/// CEP:WHAT: Pre-colors the first `n` integer parameters to SysV argument registers.
/// CEP:WHY: STANDARDS 5.9 pre-coloring constrains the allocator; params arrive in fixed regs.
/// CEP:STATUS: complete
/// CEP:FAILURE: returns only min(n, 6) entries; the caller (isel/regalloc) rejects
/// functions with more than six register parameters via UnsupportedPattern, so stack-passed
/// parameters are an explicit, reported limitation rather than silent truncation.
/// CEP:ASSUMES: integer-class parameters only; isel rejects floating-point parameters.
/// CEP:COST: O(n).
/// CEP:EVIDENCE: abi_precolor_maps_args_and_return.
pub fn precolor_integer_args(n: usize) -> Vec<PhysReg> {
    ARG_REGISTERS.iter().take(n).copied().collect()
}

/// CEP:WHAT: Pre-colors the integer return value to RAX.
/// CEP:WHY: STANDARDS 5.9: return values are returned in RAX.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: single integer return value.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: abi_precolor_maps_args_and_return.
pub fn precolor_return() -> PhysReg {
    RETURN_REGISTER
}

/// CEP:WHAT: Builds the GPR register-class table consumed by the MCF allocator: 15
/// allocatable registers, spill cost 100.0, move cost 1.0, callee-saved flagged fixed-ish
/// via `is_fixed` on RBX/RBP/R12..R15 (they cost a save/restore pair, not unavailability).
/// CEP:WHY: lir::RegisterClass is the allocator's cost/capacity source (STANDARDS 5.1).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; deterministic content on every call.
/// CEP:ASSUMES: RegId i <-> ALLOCATABLE_GPRS\[i\]; aliases left empty because this revision
/// allocates whole 64-bit registers only (no sub-register aliasing yet).
/// CEP:COST: O(15) per call.
/// CEP:EVIDENCE: gpr_class_round_trips_through_machir_naming.
pub fn gpr_class() -> RegisterClass {
    RegisterClass {
        id: RegClassId::Gpr,
        name: "gpr64",
        registers: ALLOCATABLE_GPRS
            .iter()
            .enumerate()
            .map(|(i, &p)| PhysicalRegister {
                id: RegId(i as u32),
                name: static_reg_name(p),
                is_fixed: is_callee_saved(p),
                aliases: Vec::new(),
            })
            .collect(),
        spill_cost: 100.0,
        move_cost: 1.0,
    }
}

/// CEP:WHAT: Builds the FPR register-class table: xmm0..xmm15, spill cost 100.0, move
/// cost 1.0.
/// CEP:WHY: Heterogeneous register classes are first-class in the allocator (5.1).
/// CEP:STATUS: complete (FPR allocation supported; floating-point param passing is not,
/// see isel UnsupportedPattern)
/// CEP:FAILURE: none; deterministic content on every call.
/// CEP:ASSUMES: RegId i <-> ALLOCATABLE_FPRS\[i\].
/// CEP:COST: O(16) per call.
/// CEP:EVIDENCE: compile_lir determinism tests cover the GPR path; FPR class symmetry
/// is checked by fpr_class_round_trips_through_machir_naming.
pub fn fpr_class() -> RegisterClass {
    RegisterClass {
        id: RegClassId::Fpr,
        name: "fpr64",
        registers: ALLOCATABLE_FPRS
            .iter()
            .enumerate()
            .map(|(i, &p)| PhysicalRegister {
                id: RegId(i as u32),
                name: static_reg_name(p),
                is_fixed: false,
                aliases: Vec::new(),
            })
            .collect(),
        spill_cost: 100.0,
        move_cost: 1.0,
    }
}

/// CEP:WHAT: Maps a GPR-class RegId to its machir PhysReg.
/// CEP:WHY: rewrite() and encode() consume RegIds from RegisterAllocation and need the
/// canonical numbering.
/// CEP:STATUS: complete
/// CEP:FAILURE: None when the id is outside the GPR class.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: gpr_class_round_trips_through_machir_naming.
pub fn gpr_phys(id: RegId) -> Option<PhysReg> {
    ALLOCATABLE_GPRS.get(id.0 as usize).copied()
}

/// CEP:WHAT: Maps an FPR-class RegId to its machir PhysReg.
/// CEP:WHY: Same as gpr_phys for the floating-point class.
/// CEP:STATUS: complete
/// CEP:FAILURE: None when the id is outside the FPR class.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: fpr_class_round_trips_through_machir_naming.
pub fn fpr_phys(id: RegId) -> Option<PhysReg> {
    ALLOCATABLE_FPRS.get(id.0 as usize).copied()
}

/// CEP:WHAT: ABI configuration flags; stack-clash probing is declared but not implemented.
/// CEP:WHY: STANDARDS 5.9 names stack-clash probing as part of the ABI surface; declaring
/// the flag now pins the API while the probing lowering is specified.
/// CEP:STATUS: stub for stack_clash_probing (always false; no probe code is emitted).
///   Turning it on is a deliberate future change with its own tests.
/// CEP:OWNER: axiomzero2
/// CEP:TICKET: RV8-100
/// CEP:FAILURE: none
/// CEP:ASSUMES: RV8 targets do not enable stack-clash protection in this revision.
/// CEP:COST: POD.
/// CEP:EVIDENCE: none (flag is inert); documented honestly as a stub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbiConfig {
    /// CEP:WHAT: Emit stack-clash probes on large stack adjustments.
    /// CEP:WHY: STANDARDS 5.9 surface completeness.
    /// CEP:STATUS: stub (always false in this revision).
    /// CEP:OWNER: axiomzero2
    /// CEP:TICKET: RV8-100
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: see AbiConfig.
    /// CEP:COST: none while false.
    /// CEP:EVIDENCE: none; inert stub.
    pub stack_clash_probing: bool,
}

impl Default for AbiConfig {
    /// CEP:WHAT: Default configuration: probing off.
    /// CEP:WHY: Matches the current emission contract bit-for-bit.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this impl.
    fn default() -> AbiConfig {
        AbiConfig {
            stack_clash_probing: false,
        }
    }
}

/// CEP:WHAT: Fully computed stack-frame layout: ABI-conformant MachineFrame plus the
/// emission facts (sub amount, saved order, red-zone decision, per-slot offsets) shared
/// by spill rewriting and the encoder.
/// CEP:WHY: Prologue (push/sub), epilogue (add/pop), slot addressing, and MachineFrame
/// metadata must all derive from one calculation so they can never disagree (5.3.3).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; construction is total over its inputs.
/// CEP:ASSUMES: slot width is SLOT_BYTES; alignment is STACK_ALIGNMENT.
/// CEP:COST: O(saved + slots).
/// CEP:EVIDENCE: frame_calculator_* tests.
#[derive(Clone, Debug)]
pub struct FrameLayout {
    /// ABI-conformant frame for MachineIrFunction.frame: stack_size is the total frame
    /// (saved-register area + locals) rounded up to STACK_ALIGNMENT, so it always
    /// satisfies the verifier's multiple-of-alignment invariant.
    pub frame: MachineFrame,
    /// Bytes subtracted from RSP by the prologue (0 in red-zone frames).
    pub stack_adjust: u32,
    /// Callee-saved registers actually pushed, in push order (descending PhysReg id,
    /// deterministic).
    pub saved: Vec<PhysReg>,
    /// True when the frame uses the red zone (leaf, no callee-saved, small locals):
    /// no pushes, no sub, slots at negative offsets below RSP.
    pub uses_red_zone: bool,
    /// Number of spill slots.
    pub slot_count: u32,
}

impl FrameLayout {
    /// CEP:WHAT: RSP-relative byte offset of spill slot `slot`.
    /// CEP:WHY: LoadStack/StoreStack MemOperands and their encodings need one formula.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when slot is out of range.
    /// CEP:ASSUMES: slots are addressed after the prologue has run.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: spill_path_inserts_loads_and_stores.
    pub fn slot_offset(&self, slot: u32) -> Option<i32> {
        if slot >= self.slot_count {
            return None;
        }
        if self.uses_red_zone {
            // Inside the 128-byte red zone below RSP; slot 0 sits at [rsp-8].
            Some(-((slot as i32 + 1) * SLOT_BYTES as i32))
        } else {
            Some(slot as i32 * SLOT_BYTES as i32)
        }
    }
}

/// CEP:WHAT: The frame calculator: derives the FrameLayout from the allocation facts
/// (spill slot count, callee-saved registers actually assigned, leafness).
/// CEP:WHY: One function owns the 16-byte alignment arithmetic, call-site alignment
/// (RSP % 16 == 0 at calls), and the red-zone decision.
/// CEP:STATUS: complete
/// CEP:FAILURE: total; no failure modes.
/// CEP:ASSUMES: entry RSP % 16 == 8 (return address pushed by the caller's call).
/// CEP:COST: O(saved + slots).
/// CEP:EVIDENCE: frame_calculator_layout_matches_alignment_contract,
///   frame_calculator_uses_red_zone_for_small_leaf_frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCalculator;

impl FrameCalculator {
    /// CEP:WHAT: Computes the FrameLayout.
    /// CEP:WHY: See type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: `saved` is the set of callee-saved registers the allocation used;
    /// duplicates are tolerated and deduplicated deterministically.
    /// CEP:COST: O(saved + slots).
    /// CEP:EVIDENCE: frame_calculator_* tests.
    ///
    /// Alignment arithmetic (documented so it can be checked):
    /// - At function entry RSP % 16 == 8. After p pushes and a `sub rsp, S`, RSP % 16 ==
    ///   (8 - 8p - S) mod 16. Call sites inside the function require RSP % 16 == 0, so
    ///   8p + S == 8 (mod 16): p even forces S % 16 == 8, p odd forces S % 16 == 0.
    /// - The MIR verifier requires MachineFrame.stack_size % 16 == 0, so stack_size is
    ///   the total occupied frame (8p + S) rounded up to 16, while `stack_adjust` is the
    ///   emitted S. Both are deterministic functions of (slots, saved).
    pub fn calculate(slot_count: u32, saved: &[PhysReg], leaf: bool) -> FrameLayout {
        let mut saved_set: Vec<PhysReg> = saved.to_vec();
        saved_set.sort_unstable(); // deterministic
        saved_set.dedup();
        // Push order: descending register id; epilogue pops in reverse (ascending).
        saved_set.sort_unstable_by(|a, b| b.cmp(a));
        // Keep only true callee-saved registers; anything else must never be saved.
        saved_set.retain(|r| is_callee_saved(*r));

        let spill_bytes = slot_count * SLOT_BYTES;
        let p = saved_set.len() as u32;
        let red_ok = leaf
            && p == 0
            && spill_bytes > 0
            && spill_bytes <= RED_ZONE_BYTES;

        let (stack_adjust, stack_size, uses_red_zone) = if red_ok {
            (0, 0, true)
        } else {
            let base = round_up(spill_bytes, STACK_ALIGNMENT);
            // p even => S % 16 == 8 ; p odd => S % 16 == 0 (see comment above).
            let extra = if p.is_multiple_of(2) { STACK_ALIGNMENT / 2 } else { 0 };
            let s = base + extra;
            let total = round_up(8 * p + s, STACK_ALIGNMENT);
            (s, total, false)
        };

        FrameLayout {
            frame: MachineFrame {
                stack_size,
                alignment: STACK_ALIGNMENT,
                has_red_zone: uses_red_zone,
                saved_registers: saved_set.clone(),
            },
            stack_adjust,
            saved: saved_set,
            uses_red_zone,
            slot_count,
        }
    }
}

/// CEP:WHAT: Rounds `value` up to a multiple of `align` (align must be a power of two).
/// CEP:WHY: Frame arithmetic helper with a named failure mode instead of silent wrapping.
/// CEP:STATUS: complete
/// CEP:FAILURE: saturates at u32::MAX instead of overflowing (callers feed small numbers).
/// CEP:ASSUMES: align is a power of two.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: frame_calculator tests exercise the rounding indirectly.
fn round_up(value: u32, align: u32) -> u32 {
    debug_assert!(align.is_power_of_two());
    let rounded = value.saturating_add(align - 1);
    rounded & !(align - 1)
}

/// CEP:WHAT: Defensive classification check used by the encoder: every physical register
/// in emitted code must be a non-reserved x86-64 register.
/// CEP:WHY: The encoder is the last gate before bytes; it re-checks rather than trusting
/// upstream verification (38.18 defense in depth).
/// CEP:STATUS: complete
/// CEP:FAILURE: false for reserved register numbers.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: encode_rejects_invalid_scale and encode tests.
pub fn is_valid_gpr(reg: PhysReg) -> bool {
    reg_class(Isa::X86_64, reg) == PhysClass::Gpr
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_ir::machir::reg_name;

    /// CEP:WHAT: Asserts pre-coloring maps six args to RDI,RSI,RDX,RCX,R8,R9 and the
    /// return value to RAX.
    /// CEP:WHY: STANDARDS 5.9 acceptance: pre-coloring constrains the allocator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn abi_precolor_maps_args_and_return() {
        let args = precolor_integer_args(6);
        assert_eq!(args, vec![RDI, RSI, RDX, RCX, R8, R9]);
        assert_eq!(precolor_return(), RAX);
        // Truncation is capped at six and reported by callers, never silent.
        assert_eq!(precolor_integer_args(9).len(), 6);
        assert!(precolor_integer_args(0).is_empty());
    }

    /// CEP:WHAT: Asserts the frame layout keeps stack_size a multiple of 16, keeps call-site
    /// RSP alignment (8p + S == 8 mod 16), and orders saved registers deterministically.
    /// CEP:WHY: 5.3.3 acceptance: ABI-conformant frame.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn frame_calculator_layout_matches_alignment_contract() {
        for slots in 0..40u32 {
            for keep in 0..=6usize {
                let saved = &CALLEE_SAVED_REGISTERS[..keep];
                let f = FrameCalculator::calculate(slots, saved, false);
                assert_eq!(f.frame.alignment, 16);
                assert_eq!(f.frame.stack_size % 16, 0, "slots={slots} keep={keep}");
                let p = f.saved.len() as u32;
                assert_eq!(
                    (8 * p + f.stack_adjust) % 16,
                    8,
                    "call-site alignment broken: slots={slots} keep={keep}"
                );
                assert!(f.stack_adjust >= slots * SLOT_BYTES);
                // Descending push order.
                let mut sorted = f.saved.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                assert_eq!(f.saved, sorted);
                // Slot offsets land inside the frame and never collide.
                for s in 0..slots {
                    let off = f.slot_offset(s).expect("slot in range");
                    assert!(off >= 0);
                    assert!((off as u32) < slots * SLOT_BYTES + 16);
                }
            }
        }
    }

    /// CEP:WHAT: Small leaf frames with <= 128 bytes of slots use the red zone: no pushes,
    /// no stack adjustment, negative slot offsets inside 128 bytes.
    /// CEP:WHY: ABI red-zone rule honored, not just declared.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn frame_calculator_uses_red_zone_for_small_leaf_frames() {
        let f = FrameCalculator::calculate(4, &[], true);
        assert!(f.uses_red_zone);
        assert_eq!(f.frame.stack_size, 0);
        assert!(f.frame.has_red_zone);
        assert_eq!(f.slot_offset(0), Some(-8));
        assert_eq!(f.slot_offset(3), Some(-32));
        assert_eq!(f.slot_offset(4), None, "out-of-range slot fails closed");
        // A 16-slot frame exactly fills the 128-byte red zone: slot 15 at [rsp-128].
        let full = FrameCalculator::calculate(16, &[], true);
        assert!(full.uses_red_zone);
        assert_eq!(full.slot_offset(15), Some(-128));
        // Too large for the red zone => normal frame.
        let big = FrameCalculator::calculate(17, &[], true);
        assert!(!big.uses_red_zone);
        assert!(big.stack_adjust >= 17 * SLOT_BYTES);
        // Callee-saved in play => normal frame even when leaf.
        let saved = FrameCalculator::calculate(2, &[RBX], true);
        assert!(!saved.uses_red_zone);
        assert_eq!(saved.saved, vec![RBX]);
    }

    /// CEP:WHAT: RegId -> PhysReg tables round-trip through machir::reg_name so the
    /// numbering assumption is checked against the frozen contract, not just asserted.
    /// CEP:WHY: abi's ASSUMES names machir numbering; this test keeps that honest.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn gpr_class_round_trips_through_machir_naming() {
        let class = gpr_class();
        assert_eq!(class.capacity(), 15);
        assert_eq!(class.id, RegClassId::Gpr);
        for (i, &p) in ALLOCATABLE_GPRS.iter().enumerate() {
            let pr = &class.registers[i];
            assert_eq!(pr.id, RegId(i as u32));
            assert_eq!(pr.name, reg_name(Isa::X86_64, p));
            assert_eq!(gpr_phys(pr.id), Some(p));
            assert!(is_valid_gpr(p));
        }
        // RSP is not allocatable; RBP is (no frame pointer in this backend).
        assert!(!ALLOCATABLE_GPRS.contains(&RSP));
        assert!(ALLOCATABLE_GPRS.contains(&RBP));
        assert!(is_callee_saved(RBX) && !is_caller_saved(RBX));
        assert!(is_caller_saved(RAX) && !is_callee_saved(RAX));
    }

    /// CEP:WHAT: FPR table round-trips through machir naming for xmm0..xmm15.
    /// CEP:WHY: Class symmetry check for the floating-point allocator path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn fpr_class_round_trips_through_machir_naming() {
        let class = fpr_class();
        assert_eq!(class.capacity(), 16);
        for (i, &p) in ALLOCATABLE_FPRS.iter().enumerate() {
            assert_eq!(class.registers[i].name, reg_name(Isa::X86_64, p));
            assert_eq!(fpr_phys(RegId(i as u32)), Some(p));
        }
        assert_eq!(ALLOCATABLE_FPRS[0], PhysReg(16));
    }

    /// CEP:WHAT: The static name table matches machir::reg_name for every real x86-64
    /// register number (0..=31), so the &'static str builders can never drift from the
    /// frozen machir naming contract.
    /// CEP:WHY: abi.rs ASSUMES machir numbering; this is the mechanical check.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn static_names_match_machir_reg_name() {
        for n in 0..=31u16 {
            let r = PhysReg(n);
            assert_eq!(static_reg_name(r), reg_name(Isa::X86_64, r), "reg {n}");
        }
    }
}
