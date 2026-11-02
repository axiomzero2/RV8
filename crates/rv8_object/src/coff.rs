// CEP:FILE: crates/rv8_object/src/coff.rs
// CEP:WHAT: Deterministic PE/COFF x86-64 relocatable-object serializer (Machine 0x8664):
//   COFF header with a zero TimeDateStamp, a section table ordered by permission group
//   then name (8-byte inline names, "/offset" long names in the COFF string table),
//   section characteristics derived from model permissions, a symbol table with one
//   auxiliary record per section symbol, relocation entries with the explicit model
//   addend materialized into the patched field (REL style), and a suffix-shared COFF
//   string table.
// CEP:WHY: STANDARDS 5.5 makes deterministic object emission the product on every target;
//   COFF is the Windows target and must meet the same byte-determinism bar as ELF.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: model validation runs first; duplicate section names, empty global symbol
//   names, non-external undefined symbols, alignment above COFF_MAX_ALIGN, relocation
//   counts above COFF_MAX_RELOCS_PER_SECTION, symbol values or file cursors above the u32
//   fields, and unknown relocation symbols all return ObjectError; never panics.
// CEP:ASSUMES: PE/COFF x64 numbering; the model carries no COMDAT data, so auxiliary
//   Selection is always IMAGE_COMDAT_SELECT_UNKNOWN (0); relocations are REL-style, so
//   the model addend is written into the patched field at emission; raw section data is
//   file-aligned to COFF_FILE_ALIGN (4).
// CEP:COST: O(S log S + R + Y + total bytes); one output allocation; reloc-bearing
//   sections are cloned once for addend patching; string table adds are bounded by
//   MAX_STRING_TABLE_BYTES.
// CEP:EVIDENCE: unit tests in crates/rv8_object/tests/serializers.rs (golden bytes, header
//   and section-table walk, long-name string table, aux records, relocation encoding,
//   NOBITS mapping, negatives, byte determinism); acceptance determinism test in
//   tests/acceptance.rs.

use std::collections::BTreeMap;

use crate::elf::{write_bytes_at, StringTable};
use crate::error::ObjectError;
use crate::model::{align_up, ObjectFile, RelocKind, Section, SectionType, FLAG_ALLOC, FLAG_WRITE};

// --- COFF constants (named; PE/COFF x64 numbering) ---

pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
/// CEP:WHAT: COFF file header size in bytes.
/// CEP:WHY: Named layout constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: fixed COFF header layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: shared by the writer and the golden test.
pub const COFF_HEADER_BYTES: usize = 20;
/// CEP:WHAT: Section table entry size in bytes.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: fixed COFF section layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: shared by the writer and the golden test.
pub const COFF_SECTION_BYTES: usize = 40;
/// CEP:WHAT: Symbol record and auxiliary record size in bytes.
/// CEP:WHY: Named layout constant; both are fixed 18-byte records.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: fixed COFF symbol layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: symbol-table emission and its tests.
pub const COFF_SYMBOL_BYTES: usize = 18;
/// CEP:WHAT: Relocation entry size in bytes.
/// CEP:WHY: Named layout constant.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: fixed COFF relocation layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation emission.
pub const COFF_RELOC_BYTES: usize = 10;
/// CEP:WHAT: Size of the COFF string-table length prefix.
/// CEP:WHY: The four-byte total-size field precedes the strings; "/offset" names point
///   relative to the field's start.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: long-name test.
pub const COFF_STRTAB_HEADER_BYTES: usize = 4;
/// CEP:WHAT: Adjustment from StringTable offsets to COFF string-table offsets: the shared
///   StringTable starts with one leading NUL that the COFF table does not carry.
/// CEP:WHY: COFF offsets are relative to the size field; StringTable offset 1 (the first
///   real string) maps to COFF offset 4.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: StringTable::new() seeds exactly one NUL byte.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: long-name test.
pub const COFF_STRTAB_LEADING_NUL: usize = 1;
/// CEP:WHAT: Maximum inline section/symbol name length (8-byte field).
/// CEP:WHY: Names longer than this go to the string table as "/offset".
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: long-name test.
pub const COFF_SHORT_NAME_BYTES: usize = 8;
/// CEP:WHAT: The deterministic TimeDateStamp written into the COFF header.
/// CEP:WHY: Wall-clock stamps would break reproducible builds; zero is the convention for
///   reproducible objects.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: golden test asserts the zero stamp.
pub const COFF_TIMESTAMP_ZERO: u32 = 0;
/// CEP:WHAT: File characteristics for emitted objects (no flags set).
/// CEP:WHY: Object files need no IMAGE_FILE_* flags; fixed zero keeps bytes stable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: golden test.
pub const COFF_OBJECT_CHARACTERISTICS: u16 = 0;
/// CEP:WHAT: IMAGE_SCN_CNT_CODE section characteristic.
/// CEP:WHY: Executable sections map here.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
/// CEP:WHAT: IMAGE_SCN_CNT_INITIALIZED_DATA section characteristic.
/// CEP:WHY: Initialized non-executable data maps here.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;
/// CEP:WHAT: IMAGE_SCN_CNT_UNINITIALIZED_DATA section characteristic.
/// CEP:WHY: NOBITS sections map here.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: nobits test.
pub const IMAGE_SCN_CNT_UNINITIALIZED_DATA: u32 = 0x0000_0080;
/// CEP:WHAT: IMAGE_SCN_MEM_EXECUTE section characteristic.
/// CEP:WHY: Permission mapping from FLAG_EXEC.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
/// CEP:WHAT: IMAGE_SCN_MEM_READ section characteristic.
/// CEP:WHY: Every model section is at least readable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
/// CEP:WHAT: IMAGE_SCN_MEM_WRITE section characteristic.
/// CEP:WHY: Permission mapping from FLAG_WRITE.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;
/// CEP:WHAT: IMAGE_SCN_ALIGN_1BYTES base value; the alignment exponent is OR-ed in at
///   IMAGE_SCN_ALIGN_SHIFT (0x00100000 | (log2 << 20), so 16 bytes is 0x00500000).
/// CEP:WHY: COFF encodes alignment as an exponent over this base, not a shift of it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test asserts ALIGN_16BYTES for align 16.
pub const IMAGE_SCN_ALIGN_1BYTES: u32 = 0x0010_0000;
/// CEP:WHAT: Bit position of the alignment exponent inside the characteristics field.
/// CEP:WHY: One named constant keeps the encoding in one place.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: section-table test.
pub const IMAGE_SCN_ALIGN_SHIFT: u32 = 20;
/// CEP:WHAT: Largest section alignment COFF can encode (8192 bytes).
/// CEP:WHY: Named budget; larger alignments cannot round-trip the 4-bit exponent field.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_ALIGN).
/// CEP:ASSUMES: align is a power of two (model-validated).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test coff_rejects_align_too_large.
pub const COFF_MAX_ALIGN: u64 = 8192;
/// CEP:WHAT: IMAGE_SYM_CLASS_EXTERNAL storage class.
/// CEP:WHY: Global symbols (defined and undefined) use it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: symbol-table test.
pub const IMAGE_SYM_CLASS_EXTERNAL: u8 = 2;
/// CEP:WHAT: IMAGE_SYM_CLASS_STATIC storage class.
/// CEP:WHY: Section symbols and local symbols use it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: symbol-table test.
pub const IMAGE_SYM_CLASS_STATIC: u8 = 3;
/// CEP:WHAT: Section number used for undefined symbols (IMAGE_SYM_UNDEFINED).
/// CEP:WHY: Undefined references carry section number zero.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: symbol-table test.
pub const IMAGE_SYM_UNDEFINED: u16 = 0;
/// CEP:WHAT: IMAGE_REL_AMD64_ADDR64 relocation type.
/// CEP:WHY: Model RelocKind::Abs64 maps here; part of the pinned contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF x64 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation encoding test.
pub const IMAGE_REL_AMD64_ADDR64: u16 = 0x0001;
/// CEP:WHAT: IMAGE_REL_AMD64_REL32 relocation type.
/// CEP:WHY: Model RelocKind::PcRel32 maps here; part of the pinned contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: PE/COFF x64 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: relocation encoding test.
pub const IMAGE_REL_AMD64_REL32: u16 = 0x0014;
/// CEP:WHAT: File-alignment applied to raw data, relocation tables, and the symbol table.
/// CEP:WHY: A fixed 4-byte alignment keeps every offset deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: golden test.
pub const COFF_FILE_ALIGN: u64 = 4;
/// CEP:WHAT: Largest file cursor encodable in COFF u32 pointer fields.
/// CEP:WHY: Named budget; PointerToRawData/PointerToSymbolTable are u32.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_FILE_BYTES).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test coff_rejects_section_offset_overflow.
pub const COFF_MAX_FILE_BYTES: u64 = 0xFFFF_FFFF;
/// CEP:WHAT: Largest section count encodable in the 16-bit NumberOfSections field.
/// CEP:WHY: Named budget; unreachable under MAX_SECTIONS but checked explicitly.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_SECTIONS).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: write_coff section-count check.
pub const COFF_MAX_SECTIONS: u64 = 0xFFFF;
/// CEP:WHAT: Largest relocation count encodable in the 16-bit NumberOfRelocations field.
/// CEP:WHY: Named budget; the COFF count-escape record is deliberately not emitted, so
///   oversized tables are rejected instead of mis-encoded.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_RELOCS_PER_SECTION).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: negative test coff_rejects_reloc_count_overflow.
pub const COFF_MAX_RELOCS_PER_SECTION: u64 = 0xFFFF;

/// CEP:WHAT: Deterministic permission-group rank for section ordering: executable (0),
///   allocated non-executable (1), non-allocated (2).
/// CEP:WHY: Mirrors the ELF RX/RW/non-allocated grouping so all serializers agree.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: FLAG_* semantics.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: section-ordering test.
fn group_rank(sec: &Section) -> u8 {
    if sec.is_exec() {
        0
    } else if sec.flags & FLAG_ALLOC != 0 {
        1
    } else {
        2
    }
}

/// CEP:WHAT: Section characteristics derived from the model: content class from the
///   section type, permission bits from the flags, alignment exponent from align.
/// CEP:WHY: The ".text/.data/.bss/.rdata mapping from permissions" is the value field of
///   the COFF section table; one function owns it.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_ALIGN) when align exceeds 8192.
/// CEP:ASSUMES: align is 0 (treated as 1) or a power of two.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: section-table and nobits tests.
fn section_characteristics(sec: &Section) -> Result<u32, ObjectError> {
    let align = sec.align.max(1);
    if align > COFF_MAX_ALIGN {
        return Err(ObjectError::BudgetExceeded {
            what: "COFF_MAX_ALIGN",
            limit: COFF_MAX_ALIGN,
            actual: align,
        });
    }
    let mut c = if sec.ty == SectionType::NoBits {
        IMAGE_SCN_CNT_UNINITIALIZED_DATA
    } else if sec.is_exec() {
        IMAGE_SCN_CNT_CODE
    } else {
        IMAGE_SCN_CNT_INITIALIZED_DATA
    };
    c |= IMAGE_SCN_MEM_READ;
    if sec.flags & FLAG_WRITE != 0 {
        c |= IMAGE_SCN_MEM_WRITE;
    }
    if sec.is_exec() {
        c |= IMAGE_SCN_MEM_EXECUTE;
    }
    c |= IMAGE_SCN_ALIGN_1BYTES | (align.ilog2() << IMAGE_SCN_ALIGN_SHIFT);
    Ok(c)
}

/// CEP:WHAT: Builds the section emission order: permission group, then section name;
///   duplicate model section names are rejected first.
/// CEP:WHY: COFF identifies sections by name in symbol references, so duplicates are
///   ambiguous; the (group, name) sort is a pure function of the model.
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidName ("duplicate section name").
/// CEP:ASSUMES: none
/// CEP:COST: O(S log S).
/// CEP:EVIDENCE: duplicate-section negative test.
fn coff_order(obj: &ObjectFile) -> Result<Vec<usize>, ObjectError> {
    let mut names: Vec<&str> = obj.sections.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    for pair in names.windows(2) {
        if pair[0] == pair[1] {
            return Err(ObjectError::InvalidName {
                name: pair[0].to_string(),
                reason: "duplicate section name",
            });
        }
    }
    let mut order: Vec<usize> = (0..obj.sections.len()).collect();
    order.sort_by(|&a, &b| {
        group_rank(&obj.sections[a])
            .cmp(&group_rank(&obj.sections[b]))
            .then_with(|| obj.sections[a].name.cmp(&obj.sections[b].name))
    });
    Ok(order)
}

/// CEP:WHAT: Emits a COFF relocatable object (Machine 0x8664) from the universal model:
///   zero timestamp, sections ordered by permission group then name, section symbols with
///   auxiliary records first, then model symbols in model order (locals as STATIC,
///   globals as EXTERNAL), relocations per section with the addend written into the
///   patched field, and a suffix-shared COFF string table for long names.
/// CEP:WHY: The Windows serializer of the universal model; byte-determinism is the bar.
/// CEP:STATUS: complete
/// CEP:FAILURE: validate() rejections, InvalidName (duplicate section names, empty global
///   names, non-external undefined symbols), BudgetExceeded (COFF_MAX_ALIGN,
///   COFF_MAX_FILE_BYTES, COFF_MAX_SECTIONS, COFF_MAX_RELOCS_PER_SECTION,
///   MAX_STRING_TABLE_BYTES), UnknownRelocSymbol, ArithmeticOverflow, WriteOutOfBounds;
///   never panics.
/// CEP:ASSUMES: see CEP:FILE header assumptions.
/// CEP:COST: O(S log S + R + Y + total bytes).
/// CEP:EVIDENCE: golden-byte and structural tests in tests/serializers.rs; determinism in
///   tests/acceptance.rs.
pub fn write_coff(obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    obj.validate()?;
    if obj.sections.len() as u64 > COFF_MAX_SECTIONS {
        return Err(ObjectError::BudgetExceeded {
            what: "COFF_MAX_SECTIONS",
            limit: COFF_MAX_SECTIONS,
            actual: obj.sections.len() as u64,
        });
    }
    for sym in &obj.symbols {
        if sym.global && sym.name.is_empty() {
            return Err(ObjectError::InvalidName {
                name: sym.name.clone(),
                reason: "global symbol requires a non-empty name",
            });
        }
        if !sym.defined && !sym.global {
            return Err(ObjectError::InvalidName {
                name: sym.name.clone(),
                reason: "undefined COFF symbols must be external",
            });
        }
        if sym.defined && sym.value > u32::MAX as u64 {
            return Err(ObjectError::BudgetExceeded {
                what: "COFF_SYMBOL_VALUE_SPACE",
                limit: u32::MAX as u64,
                actual: sym.value,
            });
        }
    }
    let order = coff_order(obj)?;
    let n = obj.sections.len();
    for sec in &obj.sections {
        section_characteristics(sec)?;
    }

    // Section symbol records come first (one per section, emission order); model symbols
    // follow in model order. First occurrence of each name maps to its record index for
    // relocation resolution.
    let records_per_section = 2usize; // section symbol + its auxiliary record
    let total_records = n * records_per_section + obj.symbols.len();
    let mut first_sym: BTreeMap<&str, u32> = BTreeMap::new();
    for (pos, sym) in obj.symbols.iter().enumerate() {
        let idx = (n * records_per_section + pos) as u32;
        first_sym.entry(sym.name.as_str()).or_insert(idx);
    }

    // COFF string table (suffix-shared) for long names: long section names first (in
    // emission order), then long symbol names (model order). Offsets are relative to the
    // table start (StringTable offset + strtab_adjust).
    let mut strtab = StringTable::new();
    let strtab_adjust = (COFF_STRTAB_HEADER_BYTES - COFF_STRTAB_LEADING_NUL) as u32;
    let mut sec_name_off = vec![0u32; n];
    for &i in &order {
        if obj.sections[i].name.len() > COFF_SHORT_NAME_BYTES {
            let off = strtab.add(&obj.sections[i].name)?;
            sec_name_off[i] = off + strtab_adjust;
        }
    }
    let mut sym_name_off = vec![0u32; obj.symbols.len()];
    for (m, sym) in obj.symbols.iter().enumerate() {
        if sym.name.len() > COFF_SHORT_NAME_BYTES {
            let off = strtab.add(&sym.name)?;
            sym_name_off[m] = off + strtab_adjust;
        }
    }

    // Cursor plan, every step checked against the u32 pointer budget.
    let mut cursor = COFF_HEADER_BYTES as u64 + (n as u64) * COFF_SECTION_BYTES as u64;
    check_file_cursor(cursor)?;
    let mut sec_raw_off = vec![0u64; n];
    for &i in &order {
        let sec = &obj.sections[i];
        if sec.ty != SectionType::Progbits {
            continue;
        }
        cursor = align_up(cursor, COFF_FILE_ALIGN)?;
        sec_raw_off[i] = cursor;
        cursor = cursor
            .checked_add(sec.bytes.len() as u64)
            .ok_or(ObjectError::ArithmeticOverflow { context: "coff data cursor" })?;
        check_file_cursor(cursor)?;
    }
    let mut reloc_off = vec![0u64; n];
    for &i in &order {
        if obj.sections[i].relocs.is_empty() {
            continue;
        }
        if obj.sections[i].relocs.len() as u64 > COFF_MAX_RELOCS_PER_SECTION {
            return Err(ObjectError::BudgetExceeded {
                what: "COFF_MAX_RELOCS_PER_SECTION",
                limit: COFF_MAX_RELOCS_PER_SECTION,
                actual: obj.sections[i].relocs.len() as u64,
            });
        }
        cursor = align_up(cursor, COFF_FILE_ALIGN)?;
        reloc_off[i] = cursor;
        cursor = cursor
            .checked_add((obj.sections[i].relocs.len() as u64) * COFF_RELOC_BYTES as u64)
            .ok_or(ObjectError::ArithmeticOverflow { context: "coff reloc cursor" })?;
        check_file_cursor(cursor)?;
    }
    cursor = align_up(cursor, COFF_FILE_ALIGN)?;
    let symtab_off = cursor;
    cursor = cursor
        .checked_add((total_records as u64) * COFF_SYMBOL_BYTES as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "coff symtab cursor" })?;
    check_file_cursor(cursor)?;
    let strtab_off = cursor;
    let strtab_body = &strtab.as_bytes()[COFF_STRTAB_LEADING_NUL..];
    cursor = cursor
        .checked_add((COFF_STRTAB_HEADER_BYTES + strtab_body.len()) as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "coff strtab cursor" })?;
    check_file_cursor(cursor)?;
    let total = cursor;

    let mut image = vec![0u8; usize::try_from(total)
        .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff image size" })?];

    // File header: Machine, NumberOfSections, TimeDateStamp (zero), PointerToSymbolTable,
    // NumberOfSymbols, SizeOfOptionalHeader (0), Characteristics.
    let header: Vec<u8> = IMAGE_FILE_MACHINE_AMD64
        .to_le_bytes()
        .into_iter()
        .chain((n as u16).to_le_bytes())
        .chain(COFF_TIMESTAMP_ZERO.to_le_bytes())
        .chain((symtab_off as u32).to_le_bytes())
        .chain((total_records as u32).to_le_bytes())
        .chain(0u16.to_le_bytes())
        .chain(COFF_OBJECT_CHARACTERISTICS.to_le_bytes())
        .collect();
    write_bytes_at(&mut image, 0, &header)?;

    // Section table.
    let mut at = COFF_HEADER_BYTES;
    for &i in &order {
        let sec = &obj.sections[i];
        let chars = section_characteristics(sec)?;
        let (n_relocs, reloc_ptr) = if sec.relocs.is_empty() {
            (0u16, 0u32)
        } else {
            (sec.relocs.len() as u16, reloc_off[i] as u32)
        };
        let raw_size = sec.bytes.len() as u32;
        let raw_ptr = match sec.ty {
            SectionType::Progbits => sec_raw_off[i] as u32,
            SectionType::NoBits => 0,
        };
        let mut rec: Vec<u8> = Vec::with_capacity(COFF_SECTION_BYTES);
        rec.extend_from_slice(&coff_name_bytes(
            &sec.name,
            sec_name_off[i],
        )?);
        rec.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
        rec.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
        rec.extend_from_slice(&raw_size.to_le_bytes());
        rec.extend_from_slice(&raw_ptr.to_le_bytes());
        rec.extend_from_slice(&reloc_ptr.to_le_bytes());
        rec.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
        rec.extend_from_slice(&n_relocs.to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
        rec.extend_from_slice(&chars.to_le_bytes());
        write_bytes_at(&mut image, at, &rec)?;
        at += COFF_SECTION_BYTES;
    }

    // Section contents with addends materialized into the patched fields.
    for &i in &order {
        let sec = &obj.sections[i];
        if sec.ty != SectionType::Progbits {
            continue;
        }
        if sec.relocs.is_empty() {
            write_bytes_at(
                &mut image,
                usize::try_from(sec_raw_off[i])
                    .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff data offset" })?,
                &sec.bytes,
            )?;
            continue;
        }
        let mut data = sec.bytes.clone();
        for r in &sec.relocs {
            let patch_at = usize::try_from(r.offset)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff reloc patch" })?;
            match r.kind {
                RelocKind::PcRel32 => {
                    write_bytes_at(&mut data, patch_at, &(r.addend as i32).to_le_bytes())?;
                }
                RelocKind::Abs64 => {
                    write_bytes_at(&mut data, patch_at, &(r.addend as u64).to_le_bytes())?;
                }
            }
        }
        write_bytes_at(
            &mut image,
            usize::try_from(sec_raw_off[i])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff data offset" })?,
            &data,
        )?;
    }

    // Relocation entries: VirtualAddress (offset in section), SymbolTableIndex, Type.
    for &i in &order {
        if reloc_off[i] == 0 {
            continue;
        }
        let mut table: Vec<u8> =
            Vec::with_capacity(obj.sections[i].relocs.len() * COFF_RELOC_BYTES);
        for r in &obj.sections[i].relocs {
            let idx = first_sym
                .get(r.symbol.as_str())
                .copied()
                .ok_or_else(|| ObjectError::UnknownRelocSymbol { name: r.symbol.clone() })?;
            let rtype = match r.kind {
                RelocKind::Abs64 => IMAGE_REL_AMD64_ADDR64,
                RelocKind::PcRel32 => IMAGE_REL_AMD64_REL32,
            };
            table.extend_from_slice(&(r.offset as u32).to_le_bytes());
            table.extend_from_slice(&idx.to_le_bytes());
            table.extend_from_slice(&rtype.to_le_bytes());
        }
        write_bytes_at(
            &mut image,
            usize::try_from(reloc_off[i])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff reloc offset" })?,
            &table,
        )?;
    }

    // Symbol table: section symbols (+ aux) in emission order, then model symbols.
    let mut symtab: Vec<u8> = Vec::with_capacity(total_records * COFF_SYMBOL_BYTES);
    for (pos, &i) in order.iter().enumerate() {
        let sec = &obj.sections[i];
        let section_number = u16::try_from(pos + 1)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff section number" })?;
        let mut rec: Vec<u8> = Vec::with_capacity(COFF_SYMBOL_BYTES);
        rec.extend_from_slice(&coff_name_bytes(&sec.name, sec_name_off[i])?);
        rec.extend_from_slice(&0u32.to_le_bytes()); // Value
        rec.extend_from_slice(&section_number.to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes()); // Type
        rec.push(IMAGE_SYM_CLASS_STATIC);
        rec.push(1); // NumberOfAuxSymbols
        // Auxiliary record for a section definition.
        rec.extend_from_slice(&(sec.bytes.len() as u32).to_le_bytes()); // Length
        rec.extend_from_slice(&(sec.relocs.len() as u16).to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers
        rec.extend_from_slice(&0u32.to_le_bytes()); // CheckSum
        rec.extend_from_slice(&section_number.to_le_bytes()); // Number
        rec.push(0); // Selection: IMAGE_COMDAT_SELECT_UNKNOWN (no COMDAT in the model)
        rec.extend_from_slice(&[0u8; 3]); // Reserved
        symtab.extend_from_slice(&rec);
    }
    for (m, sym) in obj.symbols.iter().enumerate() {
        let section_number = match (sym.defined, sym.section) {
            (true, Some(si)) => u16::try_from(
                order
                    .iter()
                    .position(|&i| i == si)
                    .map_or(0, |p| p + 1),
            )
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff symbol section" })?,
            _ => IMAGE_SYM_UNDEFINED,
        };
        let class = if sym.global {
            IMAGE_SYM_CLASS_EXTERNAL
        } else {
            IMAGE_SYM_CLASS_STATIC
        };
        let value = if sym.defined { sym.value as u32 } else { 0 };
        let mut rec: Vec<u8> = Vec::with_capacity(COFF_SYMBOL_BYTES);
        rec.extend_from_slice(&coff_name_bytes(&sym.name, sym_name_off[m])?);
        rec.extend_from_slice(&value.to_le_bytes());
        rec.extend_from_slice(&section_number.to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes()); // Type
        rec.push(class);
        rec.push(0); // NumberOfAuxSymbols
        symtab.extend_from_slice(&rec);
    }
    write_bytes_at(
        &mut image,
        usize::try_from(symtab_off)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff symtab offset" })?,
        &symtab,
    )?;

    // COFF string table: 4-byte total size (including the field) + strings. The shared
    // StringTable's leading NUL is not part of the COFF table.
    let mut table: Vec<u8> = Vec::with_capacity(COFF_STRTAB_HEADER_BYTES + strtab_body.len());
    table.extend_from_slice(&((COFF_STRTAB_HEADER_BYTES + strtab_body.len()) as u32).to_le_bytes());
    table.extend_from_slice(strtab_body);
    write_bytes_at(
        &mut image,
        usize::try_from(strtab_off)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "coff strtab offset" })?,
        &table,
    )?;

    Ok(image)
}

/// CEP:WHAT: Builds the 8-byte COFF name field: names up to 8 bytes are inline and
///   NUL-padded, longer names become "/offset" pointing into the string table.
/// CEP:WHY: The two-form name encoding is part of the COFF format contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidName when the "/offset" decimal does not fit the 8-byte field
///   (unreachable under MAX_STRING_TABLE_BYTES) or the name contains NUL.
/// CEP:ASSUMES: caller has validated names (check_name ran in model validation).
/// CEP:COST: O(name).
/// CEP:EVIDENCE: long-name test.
fn coff_name_bytes(name: &str, strtab_off: u32) -> Result<[u8; COFF_SHORT_NAME_BYTES], ObjectError> {
    let bytes = name.as_bytes();
    let mut out = [0u8; COFF_SHORT_NAME_BYTES];
    if bytes.len() <= COFF_SHORT_NAME_BYTES {
        out[..bytes.len()].copy_from_slice(bytes);
        return Ok(out);
    }
    let text = format!("/{strtab_off}");
    let text_bytes = text.as_bytes();
    if text_bytes.len() > COFF_SHORT_NAME_BYTES {
        return Err(ObjectError::InvalidName {
            name: name.to_string(),
            reason: "COFF /offset name does not fit the 8-byte field",
        });
    }
    out[..text_bytes.len()].copy_from_slice(text_bytes);
    Ok(out)
}

/// CEP:WHAT: Rejects a file cursor above the u32 pointer budget.
/// CEP:WHY: COFF pointer fields are u32; overflow must be an error, not a wrap.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (COFF_MAX_FILE_BYTES).
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative test coff_rejects_section_offset_overflow.
fn check_file_cursor(cursor: u64) -> Result<(), ObjectError> {
    if cursor > COFF_MAX_FILE_BYTES {
        return Err(ObjectError::BudgetExceeded {
            what: "COFF_MAX_FILE_BYTES",
            limit: COFF_MAX_FILE_BYTES,
            actual: cursor,
        });
    }
    Ok(())
}
