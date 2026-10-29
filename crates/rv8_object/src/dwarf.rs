// CEP:FILE: crates/rv8_object/src/dwarf.rs
// CEP:WHAT: DWARF debug-info emission (honest stub). Design intent for the implementing
//   agent: emit the five core sections .debug_info, .debug_abbrev, .debug_line,
//   .debug_str, and .debug_ranges from the universal object model plus machine-IR line
//   provenance. The five sections are independent once header offsets are fixed, so they
//   are built CONCURRENTLY (independent byte builders joined deterministically by
//   section order) and then compressed with a deterministic LZ backend (fixed dictionary,
//   no timestamps, no dictionary randomization) into .zdebug_* or SHF_COMPRESSED form.
// CEP:WHY: Debuggability is a first-class product property, but debug info must never
//   threaten the determinism contract: every DIE offset, line-row, and range entry is a
//   pure function of the input objects and the same layout plan the linker already uses.
// CEP:CLASS: HPC-1
// CEP:STATUS: stub
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every entry point returns Err(ObjectError::Unsupported) until implemented.
// CEP:ASSUMES: DWARF 5 for x86-64 Linux; the model grows provenance fields before this
//   module can emit meaningful line tables (function/file/line spans from Machine IR).
// CEP:COST: none while stubbed; target is O(debug records) emission.
// CEP:EVIDENCE: none yet (stub); acceptance will pin byte-determinism of .debug_* output
//   the same way the relocatable writer does.

use crate::error::ObjectError;
use crate::model::ObjectFile;

/// CEP:WHAT: Emits DWARF debug sections for an object (stub).
/// CEP:WHY: Reserved entry point so the driver can wire debug emission before the
///   implementation lands; callers get a named Unsupported error, never a panic.
/// CEP:STATUS: stub
/// CEP:FAILURE: always ObjectError::Unsupported("dwarf emission").
/// CEP:ASSUMES: see CEP:FILE design intent above.
/// CEP:COST: O(1) rejection.
/// CEP:EVIDENCE: none yet (stub).
pub fn write_dwarf64(_obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    Err(ObjectError::Unsupported("dwarf emission"))
}
