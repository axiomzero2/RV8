// CEP:FILE: crates/rv8_object/src/macho/consts.rs
// CEP:WHAT: The named Mach-O 64-bit constant set: header/record layout sizes, load-command
//   sizes, segment and protection names, section attribute bits, x86-64 relocation types,
//   the fixed LC_BUILD_VERSION payload, and the emission budgets.
// CEP:WHY: Standard section 2.6 (named budgets) and determinism: every byte the writer
//   emits traces to one named constant here, never to an inline literal.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: none; compile-time constants carry no failure paths.
// CEP:ASSUMES: Apple Mach-O 64-bit numbering for the values (magic, cpu type, load
//   commands, VM_PROT, S_ATTR, x86_64 reloc types); verified against the golden tests.
// CEP:COST: compile-time constants.
// CEP:EVIDENCE: golden-byte and field-offset tests in tests/serializers.rs and
//   tests/macho_serialization.rs.
// --- Mach-O constants (named; Apple Mach-O 64-bit numbering) ---

pub const MH_MAGIC_64: u32 = 0xFEED_FACF;
pub const CPU_TYPE_X86_64: u32 = 0x0100_0007;
pub const CPU_SUBTYPE_X86_64_ALL: u32 = 3;
pub const MH_OBJECT: u32 = 1;
pub const LC_SEGMENT_64: u32 = 0x19;
pub const LC_SYMTAB: u32 = 0x02;
pub const LC_DYSYMTAB: u32 = 0x0B;
pub const LC_BUILD_VERSION: u32 = 0x25;
pub const LC_UUID: u32 = 0x1B;
/// CEP:WHAT: Mach-O header (mach_header_64) size in bytes.
/// CEP:WHY: Named layout constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: shared by the writer and the golden test.
pub const MACHO_HEADER_BYTES: usize = 32;
/// CEP:WHAT: section_64 record size in bytes (sectname..reserved3).
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: shared by the writer and the golden test.
pub const MACHO_SECTION_BYTES: usize = 80;
/// CEP:WHAT: LC_SEGMENT_64 command size excluding section records.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: command sizing in write_macho64.
pub const MACHO_SEGMENT_HEADER_BYTES: usize = 72;
/// CEP:WHAT: LC_SYMTAB / LC_BUILD_VERSION / LC_UUID command size (24 bytes each).
/// CEP:WHY: Named layout constant for the three 6-word commands.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: command sizing in write_macho64.
pub const MACHO_CMD24_BYTES: usize = 24;
/// CEP:WHAT: LC_DYSYMTAB command size in bytes.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: command sizing in write_macho64.
pub const MACHO_DYSYMTAB_CMD_BYTES: usize = 80;
/// CEP:WHAT: nlist_64 record size in bytes.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: symbol table emission.
pub const MACHO_NLIST_BYTES: usize = 16;
/// CEP:WHAT: Relocation entry size in bytes (r_address word + info word).
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 64-bit Mach-O fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation area emission.
pub const MACHO_RELOC_BYTES: usize = 8;
/// CEP:WHAT: Maximum mapped sectname length (fixed 16-byte field).
/// CEP:WHY: Named budget; Mach-O section names cannot overflow the field.
/// CEP:STATUS: complete
/// CEP:FAILURE: macho_section_name reports InvalidName.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test macho_rejects_oversize_section_name.
pub const MACHO_SECTION_NAME_BYTES: usize = 16;
/// CEP:WHAT: UUID payload size of LC_UUID.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: SHA-256 digests carry at least 16 bytes.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: uuid_of_content.
pub const MACHO_UUID_BYTES: usize = 16;
/// CEP:WHAT: macOS platform code for LC_BUILD_VERSION.
/// CEP:WHY: Fixed platform keeps emission deterministic (no host probing).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Darwin consumer understands PLATFORM_MACOS.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: load-command walk test.
pub const PLATFORM_MACOS: u32 = 1;
/// CEP:WHAT: Fixed minimum OS version (11.0.0) packed for LC_BUILD_VERSION.
/// CEP:WHY: A fixed value, never a host or wall-clock read, keeps bytes identical.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: packed Apple version quadruple encoding (major << 16).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: load-command walk test.
pub const MACHO_MIN_OS_VERSION: u32 = 0x000B_0000;
/// CEP:WHAT: Fixed SDK version (11.0.0) packed for LC_BUILD_VERSION.
/// CEP:WHY: See MACHO_MIN_OS_VERSION.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: packed Apple version quadruple encoding.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: load-command walk test.
pub const MACHO_SDK_VERSION: u32 = 0x000B_0000;
/// CEP:WHAT: Segment name for executable sections.
/// CEP:WHY: Permission-group mapping is part of the determinism contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const SEGNAME_TEXT: &str = "__TEXT";
/// CEP:WHAT: Segment name for allocated writable and zero-fill sections.
/// CEP:WHY: See SEGNAME_TEXT.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const SEGNAME_DATA: &str = "__DATA";
/// CEP:WHAT: Segment name for non-allocated sections (debug-info convention).
/// CEP:WHY: See SEGNAME_TEXT.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: every non-allocated model section is debug-info-like (documented mapping).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const SEGNAME_NONALLOC: &str = "__DWARF";
/// CEP:WHAT: VM_PROT_READ bit.
/// CEP:WHY: Segment protection fields use VM_PROT numbering.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Darwin VM_PROT numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const VM_PROT_READ: u32 = 1;
/// CEP:WHAT: VM_PROT_WRITE bit.
/// CEP:WHY: See VM_PROT_READ.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Darwin VM_PROT numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const VM_PROT_WRITE: u32 = 2;
/// CEP:WHAT: VM_PROT_EXECUTE bit.
/// CEP:WHY: See VM_PROT_READ.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Darwin VM_PROT numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: segment_of.
pub const VM_PROT_EXECUTE: u32 = 4;
/// CEP:WHAT: maxprot value emitted for every segment (full protection).
/// CEP:WHY: Object files are never mapped; one fixed maxprot keeps bytes deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: write_macho64.
pub const VM_PROT_ALL: u32 = VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXECUTE;
/// CEP:WHAT: S_ZEROFILL section type for NOBITS sections.
/// CEP:WHY: Zero-fill memory needs no file bytes, mirroring SHT_NOBITS.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O section type numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test macho_nobits_zerofill.
pub const S_ZEROFILL: u32 = 1;
/// CEP:WHAT: S_ATTR_PURE_INSTRUCTIONS bit for executable sections.
/// CEP:WHY: Matches toolchain-emitted code sections.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O section attribute numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section record emission.
pub const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
/// CEP:WHAT: S_ATTR_SOME_INSTRUCTIONS bit for executable sections.
/// CEP:WHY: See S_ATTR_PURE_INSTRUCTIONS.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O section attribute numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section record emission.
pub const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x0000_0400;
/// CEP:WHAT: S_ATTR_DEBUG bit for non-allocated debug sections.
/// CEP:WHY: Matches toolchain-emitted debug sections.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O section attribute numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section record emission.
pub const S_ATTR_DEBUG: u32 = 0x0200_0000;
/// CEP:WHAT: x86_64 relocation type for absolute 64-bit patches.
/// CEP:WHY: Model RelocKind::Abs64 maps here; part of the pinned contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O x86_64 relocation numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation encoding test.
pub const X86_64_RELOC_UNSIGNED: u32 = 0;
/// CEP:WHAT: x86_64 relocation type for 32-bit PC-relative patches.
/// CEP:WHY: Model RelocKind::PcRel32 maps here; part of the pinned contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: Mach-O x86_64 relocation numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation encoding test.
pub const X86_64_RELOC_SIGNED: u32 = 1;
/// CEP:WHAT: Largest file cursor encodable in Mach-O u32 offset fields.
/// CEP:WHY: Named budget; symoff/stroff/reloff/offset are u32, so larger images are
///   rejected instead of truncated.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (MACHO_MAX_FILE_BYTES).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test macho_rejects_section_offset_overflow.
pub const MACHO_MAX_FILE_BYTES: u64 = 0xFFFF_FFFF;
/// CEP:WHAT: Largest section alignment encodable (2^31; align field is log2).
/// CEP:WHY: Named budget; larger alignments cannot round-trip the log2 field.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (MACHO_MAX_ALIGN).
/// CEP:ASSUMES: align is a power of two (model-validated).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test macho_rejects_align_overflow.
pub const MACHO_MAX_ALIGN: u64 = 1 << 31;
/// CEP:WHAT: Symbol-index space of the 24-bit r_symbolnum relocation field.
/// CEP:WHY: Named budget; MAX_SYMBOLS stays far below it but the check is explicit.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (MACHO_SYMBOL_INDEX_SPACE).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation encoding path.
pub const MACHO_SYMBOL_INDEX_SPACE: u64 = 0x00FF_FFFF;
/// CEP:WHAT: Alignment applied to the symtab and relocation area cursors.
/// CEP:WHY: 8-byte records on 8-byte boundaries; fixed value keeps offsets deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: write_macho64 cursor plan.
pub const MACHO_RECORD_ALIGN: u64 = 8;
