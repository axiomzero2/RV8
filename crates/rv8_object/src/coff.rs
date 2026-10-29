// CEP:FILE: crates/rv8_object/src/coff.rs
// CEP:WHAT: COFF / PE object emission (honest stub). Design intent for the implementing
//   agent: a deterministic COFF serializer for Windows x64 - sections grouped RX then RW
//   as with ELF, .text/.data/.bss mapped from Progbits/NoBits, the symbol table in a
//   fixed canonical order (section symbols, then locals, then globals; aux records
//   zero-filled), the COFF string table built with suffix sharing, and both the PE
//   TimeDateStamp and the object placeholder timestamp forced to 0 so identical input
//   yields identical bytes. Relocations map Abs64 -> R_X86_64_64-equivalent
//   (IMAGE_REL_AMD64_ADDR64) and PcRel32 -> IMAGE_REL_AMD64_REL32 with addend -4
//   absorbed by the writer.
// CEP:WHY: The universal model is format-agnostic by contract (STANDARDS 5.5); COFF/PE is
//   the Windows target and must uphold the same byte-determinism bar as ELF.
// CEP:CLASS: HPC-1
// CEP:STATUS: stub
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every entry point returns Err(ObjectError::Unsupported) until implemented.
// CEP:ASSUMES: PE/COFF x64 numbering; string-table offsets must stay under the COFF
//   32-bit boundary (guaranteed by MAX_STRING_TABLE_BYTES).
// CEP:COST: none while stubbed.
// CEP:EVIDENCE: none yet (stub).
use crate::error::ObjectError;
use crate::model::ObjectFile;

/// CEP:WHAT: Emits a COFF relocatable object (stub).
/// CEP:WHY: Reserved entry point for the Windows serializer; callers get a named
///   Unsupported error, never a panic.
/// CEP:STATUS: stub
/// CEP:FAILURE: always ObjectError::Unsupported("coff emission").
/// CEP:ASSUMES: see CEP:FILE design intent above.
/// CEP:COST: O(1) rejection.
/// CEP:EVIDENCE: none yet (stub).
pub fn write_coff(_obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    Err(ObjectError::Unsupported("coff emission"))
}
