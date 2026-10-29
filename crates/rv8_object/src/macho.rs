// CEP:FILE: crates/rv8_object/src/macho.rs
// CEP:WHAT: Mach-O 64-bit emission (honest stub). Design intent for the implementing
//   agent: a deterministic MH_OBJECT/MH_EXECUTE serializer - one LC_SEGMENT_64 per
//   permission group (RX then RW, mirroring the ELF segment rule), fileoff/vmaddr pairs
//   advanced in lockstep for p_vaddr = p_offset-style congruence, symbol and relocation
//   tables in input order (stable), and n_mtime/n_pptime forced to 0 so identical input
//   yields identical Mach-O bytes. String table reuses the suffix-sharing StringTable.
// CEP:WHY: The universal model is format-agnostic by contract (STANDARDS 5.5); Mach-O is
//   the Darwin target and must uphold the same byte-determinism bar as ELF.
// CEP:CLASS: HPC-1
// CEP:STATUS: stub
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every entry point returns Err(ObjectError::Unsupported) until implemented.
// CEP:ASSUMES: little-endian arm64/x86_64 Mach-O; load-command ordering is part of the
//   determinism contract and must be fixed before emission.
// CEP:COST: none while stubbed.
// CEP:EVIDENCE: none yet (stub).
use crate::error::ObjectError;
use crate::model::ObjectFile;

/// CEP:WHAT: Emits a Mach-O 64-bit relocatable object (stub).
/// CEP:WHY: Reserved entry point for the Darwin serializer; callers get a named
///   Unsupported error, never a panic.
/// CEP:STATUS: stub
/// CEP:FAILURE: always ObjectError::Unsupported("macho emission").
/// CEP:ASSUMES: see CEP:FILE design intent above.
/// CEP:COST: O(1) rejection.
/// CEP:EVIDENCE: none yet (stub).
pub fn write_macho64(_obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    Err(ObjectError::Unsupported("macho emission"))
}
