// CEP:FILE: crates/rv8_object/tests/coff_dwarf_serialization/main.rs
// CEP:WHAT: Byte-level tests for the COFF x86-64 serializer, the DWARF 5 debug emitter, and
//   the ELF hot/cold section-splitting pass, split into the `coff` and `dwarf_elf` modules
//   over the shared little-endian read helpers defined here: a hand-computed 122-byte COFF
//   golden at fixed offsets, header/section/symbol/relocation/NOBITS walks, one negative
//   test per COFF validation rule, ULEB128/SLEB128 round-trip properties with exact edge
//   encodings, the fixed .debug_abbrev table, the .debug_info DIE layout with strp
//   resolution, the .debug_line header and state-machine opcodes (end_sequence presence,
//   empty table), DWARF determinism and negatives, and hot-before-cold ELF ordering with
//   no-op cases and determinism.
// CEP:WHY: STANDARDS 5.5 requires byte-identical emission per format and fail-closed
//   validation; these tests are the per-rule evidence the writers' CEP:EVIDENCE lines cite.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: little-endian byte expectations are hand-computed from PE/COFF x64 and
//   DWARF 5 numbering; numeric literals are written in hex to match the byte domain.
// CEP:COST: test-only; all fixtures are small (the relocation-count negative test pushes
//   0x10000 relocs, bounded by MAX_RELOCS_PER_SECTION).
// CEP:EVIDENCE: the coff and dwarf_elf modules are the evidence; each writer rule has one
//   test there.

mod coff;
mod dwarf_elf;

pub(crate) const U16_BYTES: usize = 0x2;
pub(crate) const U32_BYTES: usize = 0x4;
pub(crate) const U64_BYTES: usize = 0x8;
pub(crate) const ULEB_MAX_BYTES: usize = 0xA;

/// CEP:WHAT: Reads a little-endian u16 at `off` from an emitted image.
/// CEP:WHY: Every byte-level assertion in this test target goes through these readers, so
///   endianness is pinned in exactly one place.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on out-of-range offsets (test-only).
/// CEP:ASSUMES: the slice is long enough for the read.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by every test in the coff and dwarf_elf modules.
pub(crate) fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + U16_BYTES].try_into().unwrap())
}

/// CEP:WHAT: Reads a little-endian u32 at `off` from an emitted image.
/// CEP:WHY: See rd_u16.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on out-of-range offsets (test-only).
/// CEP:ASSUMES: the slice is long enough for the read.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by every test in the coff and dwarf_elf modules.
pub(crate) fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + U32_BYTES].try_into().unwrap())
}

/// CEP:WHAT: Reads a little-endian u64 at `off` from an emitted image.
/// CEP:WHY: See rd_u16.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on out-of-range offsets (test-only).
/// CEP:ASSUMES: the slice is long enough for the read.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by every test in the coff and dwarf_elf modules.
pub(crate) fn rd_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + U64_BYTES].try_into().unwrap())
}
