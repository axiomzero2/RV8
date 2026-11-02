// CEP:FILE: crates/rv8_object/tests/coff_dwarf_serialization/coff.rs
// CEP:WHAT: Byte-level tests for the COFF x86-64 serializer: the hand-computed 122-byte
//   minimal golden asserted both field-by-field at fixed offsets and as a full image, the
//   multi-section permission walk with NOBITS mapping, long-name "/offset" string-table
//   encoding with suffix sharing, REL-style relocation encoding with materialized addends,
//   byte determinism, and one negative test per validation rule (alignment budget,
//   non-power-of-two alignment, duplicate section names, empty global names, undefined
//   non-external symbols, symbol-value overflow, relocation-count overflow, unknown
//   relocation symbols).
// CEP:WHY: STANDARDS 5.5 requires the Windows-target serializer to be byte-exact and
//   fail-closed; these tests are the evidence the coff.rs CEP:EVIDENCE lines cite.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: little-endian expectations hand-computed from PE/COFF x64 numbering.
// CEP:COST: test-only; fixtures are small.
// CEP:EVIDENCE: this module is the evidence; each COFF writer rule has one test below.

use rv8_object::coff::{
    write_coff, COFF_HEADER_BYTES, COFF_STRTAB_HEADER_BYTES, COFF_SYMBOL_BYTES,
    COFF_TIMESTAMP_ZERO, IMAGE_FILE_MACHINE_AMD64, IMAGE_REL_AMD64_ADDR64,
    IMAGE_REL_AMD64_REL32, IMAGE_SCN_ALIGN_1BYTES, IMAGE_SCN_ALIGN_SHIFT, IMAGE_SCN_CNT_CODE,
    IMAGE_SCN_CNT_UNINITIALIZED_DATA, IMAGE_SCN_MEM_READ, IMAGE_SCN_MEM_WRITE,
    IMAGE_SYM_CLASS_EXTERNAL, IMAGE_SYM_CLASS_STATIC,
};
use rv8_object::model::{ObjectFile, RelocKind, Section, SectionType, FLAG_ALLOC, FLAG_EXEC};
use rv8_object::ObjectError;

use crate::{rd_u16, rd_u32, rd_u64};

/// Size of one COFF section-table entry (named so walk arithmetic stays on constants).
const COFF_SECTION_ENTRY_BYTES: usize = 0x28;

/// One 16-byte-aligned .text section with one global function symbol: the minimal COFF
/// golden fixture (raw data at 0x3C, symbol table at 0x40, string table at 0x76).
fn coff_minimal() -> ObjectFile {
    let mut o = ObjectFile::new("c.o");
    let t = o.add_text_section(".text", 0x10, vec![0xC3]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x1).unwrap();
    o
}

/// CEP:WHAT: The minimal one-section one-symbol COFF object matches the hand-computed
///   122-byte fixture byte for byte, and every header/section/symbol field is asserted at
///   its fixed offset: header 0x00..0x14, section table 0x14..0x3C, raw data 0x3C,
///   symbol table 0x40..0x76, string table 0x76..0x7A.
/// CEP:WHY: Field-order regressions must break loudly against a stable golden.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: .text AX maps to CNT_CODE|MEM_READ|MEM_EXECUTE|ALIGN_16BYTES (0x60500020);
///   the timestamp is zero; the file alignment is 4.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_minimal_golden_at_fixed_offsets() {
    let img = write_coff(&coff_minimal()).unwrap();
    assert_eq!(img.len(), 0x7A);
    // File header.
    assert_eq!(rd_u16(&img, 0x00), IMAGE_FILE_MACHINE_AMD64);
    assert_eq!(rd_u16(&img, 0x02), 0x1);
    assert_eq!(rd_u32(&img, 0x04), COFF_TIMESTAMP_ZERO);
    assert_eq!(rd_u32(&img, 0x08), 0x40); // PointerToSymbolTable
    assert_eq!(rd_u32(&img, 0x0C), 0x3); // NumberOfSymbols
    assert_eq!(rd_u16(&img, 0x10), 0x0); // SizeOfOptionalHeader
    assert_eq!(rd_u16(&img, 0x12), 0x0); // Characteristics
    // Section table entry.
    let at = COFF_HEADER_BYTES;
    assert_eq!(&img[at..at + 0x5], b".text");
    assert_eq!(rd_u32(&img, at + 0x10), 0x1); // SizeOfRawData
    assert_eq!(rd_u32(&img, at + 0x14), 0x3C); // PointerToRawData
    assert_eq!(rd_u32(&img, at + 0x18), 0x0); // PointerToRelocations
    assert_eq!(rd_u16(&img, at + 0x20), 0x0); // NumberOfRelocations
    assert_eq!(rd_u32(&img, at + 0x24), 0x60500020);
    // Raw data and padding to the 4-byte file alignment.
    assert_eq!(&img[0x3C..0x40], &[0xC3, 0x00, 0x00, 0x00]);
    // Symbol records: section symbol, auxiliary, "f".
    let rec = |i: usize| &img[0x40 + i * COFF_SYMBOL_BYTES..0x40 + (i + 0x1) * COFF_SYMBOL_BYTES];
    assert_eq!(&rec(0x0)[..0x5], b".text");
    assert_eq!(rd_u32(rec(0x0), 0x8), 0x0); // Value
    assert_eq!(rd_u16(rec(0x0), 0xC), 0x1); // SectionNumber
    assert_eq!(rec(0x0)[0x10], IMAGE_SYM_CLASS_STATIC);
    assert_eq!(rec(0x0)[0x11], 0x1); // NumberOfAuxSymbols
    assert_eq!(rd_u32(rec(0x1), 0x0), 0x1); // aux Length
    assert_eq!(rd_u16(rec(0x1), 0x4), 0x0); // aux NumberOfRelocations
    assert_eq!(rd_u16(rec(0x1), 0xC), 0x1); // aux Number
    assert_eq!(rec(0x1)[0xE], 0x0); // aux Selection
    assert_eq!(rec(0x2)[0x0], b'f');
    assert_eq!(rd_u16(rec(0x2), 0xC), 0x1);
    assert_eq!(rec(0x2)[0x10], IMAGE_SYM_CLASS_EXTERNAL);
    assert_eq!(rec(0x2)[0x11], 0x0);
    // String table: the 4-byte size field only (all names fit inline).
    let strtab = 0x40 + 0x3 * COFF_SYMBOL_BYTES;
    assert_eq!(strtab, 0x76);
    assert_eq!(
        &img[strtab..strtab + COFF_STRTAB_HEADER_BYTES],
        &[0x04, 0x00, 0x00, 0x00]
    );
    // Full-image golden.
    let expected: [u8; 0x7A] = [
        // File header (20 bytes).
        0x64, 0x86, 0x01, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x40, 0x00, 0x00, 0x00,
        0x03, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        // Section table (40 bytes).
        b'.', b't', b'e', b'x', b't', 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00,
        0x3C, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x20, 0x00, 0x50, 0x60,
        // Raw data plus padding to the 4-byte file alignment.
        0xC3, 0x00, 0x00, 0x00,
        // Section symbol (18 bytes).
        b'.', b't', b'e', b'x', b't', 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01,
        // Auxiliary record (18 bytes).
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x00, 0x00,
        // Symbol "f" (18 bytes).
        b'f', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00,
        // String table (4 bytes): size field only.
        0x04, 0x00, 0x00, 0x00,
    ];
    assert_eq!(img, expected);
}

/// CEP:WHAT: A multi-section COFF emission walks correctly: the .bss NOBITS entry maps to
///   CNT_UNINITIALIZED_DATA with a zero raw pointer, a reserved raw size, and
///   MEM_READ|MEM_WRITE|ALIGN_8BYTES characteristics, while .text/.data keep their content
///   classes (sections are ordered by permission group, then name).
/// CEP:WHY: The permission-to-characteristics mapping is the ".text/.data/.bss" rule of
///   the COFF serializer contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: order is (group rank, name): .text, then .bss, then .data by name.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_nobits_uninitialized_mapping() {
    let mut o = ObjectFile::new("walk.o");
    let t = o.add_text_section(".text", 0x10, vec![0x90; 0x4]).unwrap();
    let d = o.add_data_section(".data", 0x8, vec![0x7; 0x8]).unwrap();
    o.add_nobits_section(".bss", 0x8, 0x80).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x4).unwrap();
    o.add_symbol("v", true, true, Some(d), 0x0, 0x8).unwrap();
    let img = write_coff(&o).unwrap();
    let text = COFF_HEADER_BYTES;
    let bss = text + COFF_SECTION_ENTRY_BYTES;
    let data = bss + COFF_SECTION_ENTRY_BYTES;
    assert_eq!(&img[text..text + 0x5], b".text");
    assert_eq!(&img[data..data + 0x5], b".data");
    assert_eq!(&img[bss..bss + 0x4], b".bss");
    // .bss: reserved raw size, no file bytes, RW content class.
    assert_eq!(rd_u32(&img, bss + 0x10), 0x80);
    assert_eq!(rd_u32(&img, bss + 0x14), 0x0);
    assert_eq!(
        rd_u32(&img, bss + 0x24),
        IMAGE_SCN_CNT_UNINITIALIZED_DATA
            | IMAGE_SCN_MEM_READ
            | IMAGE_SCN_MEM_WRITE
            | IMAGE_SCN_ALIGN_1BYTES
            | (0x3 << IMAGE_SCN_ALIGN_SHIFT)
    );
    // .text and .data keep their content classes.
    assert_eq!(
        rd_u32(&img, text + 0x24) & IMAGE_SCN_CNT_CODE,
        IMAGE_SCN_CNT_CODE
    );
    assert_eq!(
        rd_u32(&img, data + 0x24) & IMAGE_SCN_CNT_UNINITIALIZED_DATA,
        0x0
    );
}

/// CEP:WHAT: Long section and symbol names become "/offset" fields into the COFF string
///   table (offsets relative to the size field), and the symbol name suffix-shares the
///   section name's bytes one byte in.
/// CEP:WHY: The long-name mechanism must round-trip exactly and share suffixes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: ".long_shared_name" is emitted before the symbol name (section order).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_long_name_string_table_offsets() {
    let mut o = ObjectFile::new("long.o");
    let s = o
        .add_section(".long_shared_name", SectionType::Progbits, FLAG_ALLOC, 0x1, vec![0x0; 0x4])
        .unwrap();
    o.add_symbol("long_shared_name", true, true, Some(s), 0x0, 0x4).unwrap();
    let img = write_coff(&o).unwrap();
    let at = COFF_HEADER_BYTES;
    let name_field = &img[at..at + 0x8];
    assert_eq!(name_field[0x0], b'/');
    let sec_off: usize = std::str::from_utf8(&name_field[0x1..0x8])
        .unwrap()
        .trim_end_matches('\0')
        .parse()
        .unwrap();
    let sym_ptr = rd_u32(&img, 0x8) as usize;
    let n_syms = rd_u32(&img, 0xC) as usize;
    let strtab_at = sym_ptr + n_syms * COFF_SYMBOL_BYTES;
    let strtab_size = rd_u32(&img, strtab_at) as usize;
    // Size field + ".long_shared_name\0"; the symbol name shares the suffix.
    assert_eq!(strtab_size, COFF_STRTAB_HEADER_BYTES + 0x12);
    assert_eq!(
        &img[strtab_at + sec_off..strtab_at + sec_off + 0x11],
        b".long_shared_name"
    );
    let sym_name = &img[sym_ptr + 0x2 * COFF_SYMBOL_BYTES..sym_ptr + 0x2 * COFF_SYMBOL_BYTES + 0x8];
    assert_eq!(sym_name[0x0], b'/');
    let sym_off: usize = std::str::from_utf8(&sym_name[0x1..0x8])
        .unwrap()
        .trim_end_matches('\0')
        .parse()
        .unwrap();
    assert_eq!(sym_off, sec_off + 0x1);
    assert_eq!(
        &img[strtab_at + sym_off..strtab_at + sym_off + 0x10],
        b"long_shared_name"
    );
}

/// CEP:WHAT: COFF relocation encoding: PcRel32 becomes IMAGE_REL_AMD64_REL32, Abs64
///   becomes IMAGE_REL_AMD64_ADDR64, symbol indices resolve to first occurrence in the
///   record order (section symbol, aux, then model symbols), and the model addend is
///   materialized into the patched field (REL style).
/// CEP:WHY: REL-style conversion and index resolution are the load-bearing COFF rules.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: record order makes "ext" index 0x3 and "g0" index 0x2.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_relocation_encoding_rel_style() {
    let mut o = ObjectFile::new("reloc.o");
    let mut code = vec![0xE8, 0x00, 0x00, 0x00, 0x00];
    code.extend_from_slice(&[0x00; 0x0B]);
    let t = o.add_text_section(".text", 0x1, code).unwrap();
    o.add_symbol("g0", true, true, Some(t), 0x0, 0x4).unwrap();
    o.add_symbol("ext", false, true, None, 0x0, 0x0).unwrap();
    o.add_reloc(t, 0x1, RelocKind::PcRel32, "ext", -0x4).unwrap();
    o.add_reloc(t, 0x8, RelocKind::Abs64, "g0", 0x10).unwrap();
    let img = write_coff(&o).unwrap();
    let at = COFF_HEADER_BYTES;
    let data_off = rd_u32(&img, at + 0x14) as usize;
    let reloc_off = rd_u32(&img, at + 0x18) as usize;
    assert_eq!(&img[data_off + 0x1..data_off + 0x5], &0xFFFF_FFFCu32.to_le_bytes());
    assert_eq!(rd_u64(&img, data_off + 0x8), 0x10);
    let entry = |i: usize| &img[reloc_off + i * 0xA..reloc_off + (i + 0x1) * 0xA];
    assert_eq!(rd_u32(entry(0x0), 0x0), 0x1); // VirtualAddress
    assert_eq!(rd_u32(entry(0x0), 0x4), 0x3); // "ext" record index
    assert_eq!(rd_u16(entry(0x0), 0x8), IMAGE_REL_AMD64_REL32);
    assert_eq!(rd_u32(entry(0x1), 0x0), 0x8);
    assert_eq!(rd_u32(entry(0x1), 0x4), 0x2); // "g0" record index
    assert_eq!(rd_u16(entry(0x1), 0x8), IMAGE_REL_AMD64_ADDR64);
}

/// CEP:WHAT: Two emissions of the same multi-section COFF object are byte-identical.
/// CEP:WHY: Byte-determinism is the acceptance bar of STANDARDS 5.5.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_determinism() {
    let mut o = ObjectFile::new("det.o");
    let t = o.add_text_section(".text", 0x10, vec![0xE8, 0x00, 0x00, 0x00, 0x00]).unwrap();
    let d = o.add_data_section(".data", 0x8, vec![0x7; 0x8]).unwrap();
    o.add_nobits_section(".bss", 0x8, 0x40).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x5).unwrap();
    o.add_symbol("v", true, true, Some(d), 0x0, 0x8).unwrap();
    o.add_symbol("ext", false, true, None, 0x0, 0x0).unwrap();
    o.add_reloc(t, 0x1, RelocKind::PcRel32, "ext", -0x4).unwrap();
    let a = write_coff(&o).unwrap();
    let b = write_coff(&o).unwrap();
    assert_eq!(a, b);
}

/// CEP:WHAT: COFF alignment and name validation: alignment above COFF_MAX_ALIGN and
///   non-power-of-two alignment, duplicate section names, empty global symbol names, and
///   undefined non-external symbols each return the named ObjectError.
/// CEP:WHY: One negative test per fail-closed validation rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: the model rejects align 6 before the writer sees it.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_rejects_align_and_name_rules() {
    let mut o = ObjectFile::new("align.o");
    o.add_text_section(".text", 0x4000, vec![0x90]).unwrap();
    assert!(matches!(
        write_coff(&o),
        Err(ObjectError::BudgetExceeded { what: "COFF_MAX_ALIGN", .. })
    ));
    let mut bad = ObjectFile::new("nonpow2.o");
    bad.sections.push(Section {
        name: ".bad".into(),
        ty: SectionType::Progbits,
        flags: FLAG_ALLOC | FLAG_EXEC,
        align: 0x6,
        bytes: vec![0x90],
        relocs: Vec::new(),
    });
    assert_eq!(
        write_coff(&bad),
        Err(ObjectError::InvalidAlignment { section: ".bad".into(), align: 0x6 })
    );
    let mut dup = ObjectFile::new("dup.o");
    dup.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    dup.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    assert_eq!(
        write_coff(&dup),
        Err(ObjectError::InvalidName {
            name: ".text".into(),
            reason: "duplicate section name",
        })
    );
    let mut anon = ObjectFile::new("anon.o");
    let t = anon.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    anon.add_symbol("", true, true, Some(t), 0x0, 0x1).unwrap();
    assert_eq!(
        write_coff(&anon),
        Err(ObjectError::InvalidName {
            name: "".into(),
            reason: "global symbol requires a non-empty name",
        })
    );
    let mut quiet = ObjectFile::new("locundef.o");
    quiet.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    quiet.add_symbol("quiet", false, false, None, 0x0, 0x0).unwrap();
    assert_eq!(
        write_coff(&quiet),
        Err(ObjectError::InvalidName {
            name: "quiet".into(),
            reason: "undefined COFF symbols must be external",
        })
    );
}

/// CEP:WHAT: COFF field-overflow validation: a symbol value above u32::MAX, a relocation
///   count above the 16-bit NumberOfRelocations field, and a relocation naming an absent
///   symbol each fail closed.
/// CEP:WHY: u32/u16 fields must reject overflow, never wrap.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert/matches on mismatch (test-only).
/// CEP:ASSUMES: overlapping relocation offsets are legal in the model, so a 0x8-byte
///   section can carry 0x10000 entries.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_rejects_field_overflow() {
    let mut big = ObjectFile::new("big.o");
    let tb = big.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    big.add_symbol("big", true, true, Some(tb), u64::MAX, 0x1).unwrap();
    assert!(matches!(
        write_coff(&big),
        Err(ObjectError::BudgetExceeded { what: "COFF_SYMBOL_VALUE_SPACE", .. })
    ));
    let mut many = ObjectFile::new("many.o");
    let tm = many.add_text_section(".text", 0x1, vec![0x0; 0x8]).unwrap();
    many.add_symbol("f", true, true, Some(tm), 0x0, 0x8).unwrap();
    for _ in 0..0x10000u32 {
        many.add_reloc(tm, 0x0, RelocKind::Abs64, "f", 0x0).unwrap();
    }
    assert!(matches!(
        write_coff(&many),
        Err(ObjectError::BudgetExceeded { what: "COFF_MAX_RELOCS_PER_SECTION", .. })
    ));
    let mut ghost = ObjectFile::new("ghost.o");
    let tg = ghost.add_text_section(".text", 0x1, vec![0xE8, 0x00, 0x00, 0x00, 0x00]).unwrap();
    ghost.add_symbol("f", true, true, Some(tg), 0x0, 0x5).unwrap();
    ghost.add_reloc(tg, 0x1, RelocKind::PcRel32, "ghost", -0x4).unwrap();
    assert_eq!(
        write_coff(&ghost),
        Err(ObjectError::UnknownRelocSymbol { name: "ghost".into() })
    );
}
