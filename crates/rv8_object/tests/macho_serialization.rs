// CEP:FILE: crates/rv8_object/tests/macho_serialization.rs
// CEP:WHAT: Mach-O 64 serializer contract tests: byte-determinism, a hand-computed minimal
//   golden layout (header, load-command, section-record, nlist, and strtab offsets), fixed
//   field-offset checks with zeroed reserved words, string-table suffix dedup, relocation
//   entry encoding with materialized addends, LC_DYSYMTAB presence/partition rules, and one
//   structured ObjectError per rejection rule (alignment, duplicate section/symbol names,
//   oversize names, bad symbol shapes, unknown reloc symbols, file-offset overflow).
// CEP:WHY: STANDARDS 5.5 requires byte-deterministic Mach-O emission and fail-closed
//   validation; these tests pin those properties at the byte level without external parsers.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: little-endian Mach-O 64 layout; hand-computed offsets for the one-section
//   fixture (32-byte header, 200-byte command block, section data at 240, symtab at 248,
//   strtab at 264).
// CEP:COST: test-only; all fixtures are small.
// CEP:EVIDENCE: this file is the evidence; every required behavior has one test below.

use rv8_object::macho::{
    write_macho64, CPU_SUBTYPE_X86_64_ALL, CPU_TYPE_X86_64, LC_BUILD_VERSION, LC_DYSYMTAB,
    LC_SEGMENT_64, LC_SYMTAB, LC_UUID, MACHO_CMD24_BYTES, MACHO_DYSYMTAB_CMD_BYTES,
    MACHO_HEADER_BYTES, MACHO_NLIST_BYTES, MACHO_SECTION_BYTES, MACHO_SEGMENT_HEADER_BYTES,
    MH_MAGIC_64, MH_OBJECT, S_ATTR_PURE_INSTRUCTIONS, S_ATTR_SOME_INSTRUCTIONS, SEGNAME_DATA,
    SEGNAME_NONALLOC, SEGNAME_TEXT, S_ZEROFILL, X86_64_RELOC_SIGNED, X86_64_RELOC_UNSIGNED,
};
use rv8_object::model::{ObjectFile, RelocKind, Section, SectionType, FLAG_ALLOC, FLAG_EXEC};
use rv8_object::ObjectError;

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn rd_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// Reads a NUL-padded fixed 16-byte Mach-O name field as a trimmed String.
fn rd_name(b: &[u8], off: usize) -> String {
    let raw = &b[off..off + 16];
    let end = raw.iter().position(|&c| c == 0).unwrap_or(16);
    String::from_utf8(raw[..end].to_vec()).unwrap()
}

/// Returns (byte offset, raw record) for every section_64 record in the load commands,
/// in file order.
fn section_records(img: &[u8]) -> Vec<(usize, [u8; MACHO_SECTION_BYTES])> {
    let ncmds = rd_u32(img, 16);
    let mut at = MACHO_HEADER_BYTES;
    let mut out = Vec::new();
    for _ in 0..ncmds {
        let cmd = rd_u32(img, at);
        let size = rd_u32(img, at + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            let nsects = rd_u32(img, at + 64) as usize;
            for s in 0..nsects {
                let rec = at + MACHO_SEGMENT_HEADER_BYTES + s * MACHO_SECTION_BYTES;
                let mut raw = [0u8; MACHO_SECTION_BYTES];
                raw.copy_from_slice(&img[rec..rec + MACHO_SECTION_BYTES]);
                out.push((rec, raw));
            }
        }
        at += size;
    }
    out
}

/// Returns the byte offset of the first load command with the given cmd id, if present.
fn find_cmd(img: &[u8], want: u32) -> Option<usize> {
    let ncmds = rd_u32(img, 16);
    let mut at = MACHO_HEADER_BYTES;
    for _ in 0..ncmds {
        let cmd = rd_u32(img, at);
        let size = rd_u32(img, at + 4) as usize;
        if cmd == want {
            return Some(at);
        }
        at += size;
    }
    None
}

/// Reads the (symoff, nsyms, stroff, strsize) quadruple of the LC_SYMTAB command.
fn symtab_cmd(img: &[u8]) -> (usize, usize, usize, usize) {
    let at = find_cmd(img, LC_SYMTAB).expect("LC_SYMTAB present");
    (
        rd_u32(img, at + 8) as usize,
        rd_u32(img, at + 12) as usize,
        rd_u32(img, at + 16) as usize,
        rd_u32(img, at + 20) as usize,
    )
}

/// One-section, one-local-symbol object: the hand-computed golden fixture.
fn golden_object() -> ObjectFile {
    let mut o = ObjectFile::new("golden.o");
    let t = o
        .add_text_section(".text", 16, vec![0x8d, 0x04, 0x37, 0xc3])
        .expect("add .text");
    o.add_symbol("f", true, false, Some(t), 0, 4).expect("add symbol");
    o
}

/// CEP:WHAT: Two emissions of the same model are byte-identical, including the UUID.
/// CEP:WHY: Required determinism gate (STANDARDS 5.5): same input, identical bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_two_runs_byte_identical() {
    let obj = golden_object();
    let a = write_macho64(&obj).expect("first emission");
    let b = write_macho64(&obj).expect("second emission");
    assert_eq!(a, b, "Mach-O writer must be byte-deterministic");
    assert!(!a.is_empty());
}

/// CEP:WHAT: Hand-computed golden layout for one 16-aligned 4-byte section plus one local
///   symbol: ncmds=4, sizeofcmds=224, section data at 256, symtab at 264, strtab at 280,
///   total 283 bytes, and the exact nlist/strtab bytes.
/// CEP:WHY: Offsets must be provable, not just self-consistent: every cursor here was
///   computed by hand from the documented layout law.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: LC_SYMTAB is always emitted; locals-only objects omit LC_DYSYMTAB only;
///   command block = 24 + 24 + 152 + 24.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_minimal_golden_offsets() {
    let img = write_macho64(&golden_object()).expect("emission");
    assert_eq!(img.len(), 283);
    assert_eq!(rd_u32(&img, 16), 4, "ncmds: build_version, uuid, segment, symtab");
    assert_eq!(rd_u32(&img, 20), 224, "sizeofcmds = 24 + 24 + 152 + 24");

    // Load commands at their hand-computed offsets.
    assert_eq!(rd_u32(&img, 32), LC_BUILD_VERSION);
    assert_eq!(rd_u32(&img, 36), MACHO_CMD24_BYTES as u32);
    assert_eq!(rd_u32(&img, 56), LC_UUID);
    assert_eq!(rd_u32(&img, 60), MACHO_CMD24_BYTES as u32);
    assert_eq!(rd_u32(&img, 80), LC_SEGMENT_64);
    assert_eq!(
        rd_u32(&img, 84),
        (MACHO_SEGMENT_HEADER_BYTES + MACHO_SECTION_BYTES) as u32
    );
    assert_eq!(rd_name(&img, 88), SEGNAME_TEXT);
    // Segment covers command-block-end .. end of section data (vmaddr == fileoff).
    assert_eq!(rd_u64(&img, 104), 256, "vmaddr = segment start");
    assert_eq!(rd_u64(&img, 112), 4, "vmsize");
    assert_eq!(rd_u64(&img, 120), 256, "fileoff = segment start");
    assert_eq!(rd_u64(&img, 128), 4, "filesize");
    assert_eq!(rd_u32(&img, 232), LC_SYMTAB, "symtab command follows segments");

    // Section record at 80 + 72 = 152 (80 bytes, all field offsets hand-computed).
    assert_eq!(rd_name(&img, 152), "__text");
    assert_eq!(rd_name(&img, 168), SEGNAME_TEXT);
    assert_eq!(rd_u64(&img, 184), 0, "section addr zero (MH_OBJECT convention)");
    assert_eq!(rd_u64(&img, 192), 4, "section size");
    assert_eq!(rd_u32(&img, 200), 256, "section file offset (u32 field)");
    assert_eq!(rd_u32(&img, 204), 4, "align 16 -> exponent 4");
    assert_eq!(rd_u32(&img, 208), 0, "reloff empty");
    assert_eq!(rd_u32(&img, 212), 0, "nreloc zero");
    assert_eq!(
        rd_u32(&img, 216),
        S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS,
        "code section flags"
    );

    // nlist_64 at 264: one local defined symbol, n_strx 1, N_SECT, section 1.
    assert_eq!(rd_u32(&img, 264), 1, "n_strx into strtab");
    assert_eq!(img[268], 0x0E, "n_type N_SECT for a local definition");
    assert_eq!(img[269], 1, "n_sect = first section");
    assert_eq!(rd_u16(&img, 270), 0, "n_desc zero");
    assert_eq!(rd_u64(&img, 272), 0, "n_value = section-relative offset");

    // String table at 280: the mandatory NUL plus the symbol name.
    assert_eq!(&img[280..283], b"\0f\0");
    let (symoff, nsyms, stroff, strsize) = symtab_cmd(&img);
    assert_eq!((symoff, nsyms, stroff, strsize), (264, 1, 280, 3));
}

/// CEP:WHAT: Header identity fields and every reserved/zero word: magic, cpu type/subtype,
///   filetype, zeroed flags and reserved words (Mach-O carries no timestamps), and reserved
///   section-record words.
/// CEP:WHY: Fixed-offset field checks and the zero-padding law (no wall-clock data).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: one global symbol fixture for the n_type/n_desc checks.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_header_fields_zeroed_at_fixed_offsets() {
    let mut o = ObjectFile::new("fields.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("g", true, true, Some(t), 0, 1).expect("symbol");
    let img = write_macho64(&o).expect("emission");
    assert_eq!(rd_u32(&img, 0), MH_MAGIC_64);
    assert_eq!(rd_u32(&img, 4), CPU_TYPE_X86_64);
    assert_eq!(rd_u32(&img, 8), CPU_SUBTYPE_X86_64_ALL);
    assert_eq!(rd_u32(&img, 12), MH_OBJECT);
    assert_eq!(rd_u32(&img, 24), 0, "flags word zero");
    assert_eq!(rd_u32(&img, 28), 0, "reserved header word zero");
    // UUID payload is present and deterministic-looking (non-zero digest prefix).
    assert!(img[64..80].iter().any(|&b| b != 0), "UUID not all-zero");
    // Global definition: N_SECT | N_EXT, n_desc still zero.
    let (symoff, _, _, _) = symtab_cmd(&img);
    assert_eq!(img[symoff + 4], 0x0F, "N_SECT|N_EXT for a global definition");
    assert_eq!(rd_u16(&img, symoff + 6), 0, "n_desc zero");
    // Reserved words of the section record stay zero.
    let (base, _) = section_records(&img)[0];
    assert_eq!(rd_u32(&img, base + 68), 0, "reserved1");
    assert_eq!(rd_u32(&img, base + 72), 0, "reserved2");
    assert_eq!(rd_u32(&img, base + 76), 0, "reserved3");
}

/// CEP:WHAT: String-table suffix sharing across the Mach-O strtab: adding "main" after
///   "domain" reuses the embedded "main\0" suffix, so the table stays 8 bytes and the second
///   nlist n_strx points into the middle of the first name.
/// CEP:WHY: STANDARDS 5.5 names string dedup with suffix sharing; the Mach-O writer must
///   inherit it from the shared StringTable.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: emitted symbol order keeps model order within the local class.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_strtab_suffix_dedup() {
    let mut o = ObjectFile::new("dedup.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("domain", true, false, Some(t), 0, 1).expect("sym0");
    o.add_symbol("main", true, false, Some(t), 1, 1).expect("sym1");
    let img = write_macho64(&o).expect("emission");
    let (symoff, nsyms, stroff, strsize) = symtab_cmd(&img);
    assert_eq!(nsyms, 2);
    assert_eq!(strsize, 8, "table is NUL + domain + NUL only");
    assert_eq!(&img[stroff..stroff + strsize], b"\0domain\0");
    assert_eq!(rd_u32(&img, symoff), 1, "domain at offset 1");
    assert_eq!(rd_u32(&img, symoff + MACHO_NLIST_BYTES), 3, "main reuses suffix at 3");
}

/// CEP:WHAT: Relocation entries encode (r_address, symbol index, pcrel, length, extern,
///   type) and the model addends are materialized into the patched fields of the section
///   data (REL-style).
/// CEP:WHY: The relocation contract of the writer: x86_64 r_type numbering, 8-byte entries,
///   and explicit-addend materialization must all be byte-pinned.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: emitted symbol order locals/extdefs/undefs gives "f" index 0, "v" index 1,
///   and "ext" index 2; hand-computed section data offset 496 and reloc table offset 520.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_reloc_encoding_and_materialized_addends() {
    let mut o = ObjectFile::new("reloc.o");
    let bytes = vec![0xe8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let t = o.add_text_section(".text", 16, bytes).expect(".text");
    let d = o.add_data_section(".data", 8, vec![0xaa; 8]).expect(".data");
    o.add_symbol("f", true, false, Some(t), 0, 1).expect("local");
    o.add_symbol("v", true, true, Some(d), 0, 8).expect("extdef");
    o.add_symbol("ext", false, true, None, 0, 0).expect("undef");
    o.add_reloc(t, 1, RelocKind::PcRel32, "ext", -4).expect("reloc 1");
    o.add_reloc(t, 8, RelocKind::Abs64, "v", 7).expect("reloc 2");
    let img = write_macho64(&o).expect("emission");

    // __TEXT section record: data at 496, reloc table at 520 with 2 entries.
    let records = section_records(&img);
    assert_eq!(records.len(), 2);
    let (text_at, _) = &records[0];
    assert_eq!(*text_at, 152, "__TEXT section record offset");
    assert_eq!(rd_u32(&img, 152 + 48), 496, "hand-computed .text data offset");
    let reloff = rd_u32(&img, 152 + 56) as usize;
    let nreloc = rd_u32(&img, 152 + 60) as usize;
    assert_eq!((reloff, nreloc), (520, 2));

    // Entry 0: PcRel32 against "ext" (emitted index 2): pcrel=1, length=2, extern=1,
    // r_type=X86_64_RELOC_SIGNED.
    let info0 = rd_u32(&img, reloff + 4);
    assert_eq!(rd_u32(&img, reloff), 1, "r_address = patch offset in section");
    assert_eq!(info0 & 0x00FF_FFFF, 2, "r_symbolnum = emitted index of ext");
    assert_eq!((info0 >> 24) & 1, 1, "r_pcrel");
    assert_eq!((info0 >> 25) & 0x3, 2, "r_length = 2 (4 bytes)");
    assert_eq!((info0 >> 27) & 1, 1, "r_extern");
    assert_eq!((info0 >> 28) & 0xF, X86_64_RELOC_SIGNED);

    // Entry 1: Abs64 against "v" (emitted index 1): pcrel=0, length=3, extern=1,
    // r_type=X86_64_RELOC_UNSIGNED.
    let info1 = rd_u32(&img, reloff + 12);
    assert_eq!(rd_u32(&img, reloff + 8), 8, "r_address of the Abs64 patch");
    assert_eq!(info1 & 0x00FF_FFFF, 1, "r_symbolnum = emitted index of v");
    assert_eq!((info1 >> 24) & 1, 0, "r_pcrel clear");
    assert_eq!((info1 >> 25) & 0x3, 3, "r_length = 3 (8 bytes)");
    assert_eq!((info1 >> 27) & 1, 1, "r_extern");
    assert_eq!((info1 >> 28) & 0xF, X86_64_RELOC_UNSIGNED);

    // Addends materialized into the section bytes: rel32 field = -4, abs64 field = 7.
    assert_eq!(&img[496 + 1..496 + 5], &[0xfc, 0xff, 0xff, 0xff]);
    assert_eq!(&img[496 + 8..496 + 16], &7u64.to_le_bytes());
}

/// CEP:WHAT: A locals-only object emits no LC_DYSYMTAB (documented omission: the indirect
///   symbol machinery only exists when externals exist) and ncmds shrinks accordingly.
/// CEP:WHY: LC_DYSYMTAB omission must be deliberate and testable, not accidental.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_dysymtab_omitted_without_externals() {
    let img = write_macho64(&golden_object()).expect("emission");
    assert_eq!(rd_u32(&img, 16), 4, "build_version + uuid + segment + symtab");
    assert!(find_cmd(&img, LC_DYSYMTAB).is_none(), "no dysymtab");
    assert!(find_cmd(&img, LC_SYMTAB).is_some(), "symtab still present");
}

/// CEP:WHAT: With externals present the LC_DYSYMTAB partition fields match the emitted
///   nlist order: locals, then defined externals, then undefined externals, with correct
///   n_type bytes and zero n_value for undefined symbols.
/// CEP:WHY: The dysymtab indices are the machine-readable form of the symbol partition.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_dysymtab_partition_indices() {
    let mut o = ObjectFile::new("parts.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("l1", true, false, Some(t), 0, 1).expect("l1");
    o.add_symbol("l2", true, false, Some(t), 1, 1).expect("l2");
    o.add_symbol("g1", true, true, Some(t), 2, 1).expect("g1");
    o.add_symbol("u1", false, true, None, 0, 0).expect("u1");
    let img = write_macho64(&o).expect("emission");

    let dysym = find_cmd(&img, LC_DYSYMTAB).expect("dysymtab present");
    assert_eq!(rd_u32(&img, dysym + 4), MACHO_DYSYMTAB_CMD_BYTES as u32);
    assert_eq!(rd_u32(&img, dysym + 8), 0, "ilocalsym");
    assert_eq!(rd_u32(&img, dysym + 12), 2, "nlocalsym");
    assert_eq!(rd_u32(&img, dysym + 16), 2, "iextdefsym");
    assert_eq!(rd_u32(&img, dysym + 20), 1, "nextdefsym");
    assert_eq!(rd_u32(&img, dysym + 24), 3, "iundefsym");
    assert_eq!(rd_u32(&img, dysym + 28), 1, "nundefsym");

    let (symoff, nsyms, stroff, strsize) = symtab_cmd(&img);
    assert_eq!(nsyms, 4);
    let names: Vec<&str> = ["l1\0", "l2\0", "g1\0", "u1\0"].to_vec();
    let types = [0x0Eu8, 0x0Eu8, 0x0Fu8, 0x01u8];
    let sects = [1u8, 1, 1, 0]; // undefined externals carry n_sect 0
    for (i, ((want_name, want_type), want_sect)) in
        names.iter().zip(types.iter()).zip(sects.iter()).enumerate()
    {
        let rec = symoff + i * MACHO_NLIST_BYTES;
        let strx = rd_u32(&img, rec) as usize;
        let got = &img[stroff + strx..stroff + strsize];
        assert_eq!(&got[..want_name.len()], want_name.as_bytes());
        assert_eq!(img[rec + 4], *want_type, "n_type of symbol {i}");
        assert_eq!(img[rec + 5], *want_sect, "n_sect of symbol {i}");
    }
    // Undefined externals carry n_value 0.
    assert_eq!(rd_u64(&img, symoff + 3 * MACHO_NLIST_BYTES + 8), 0);
}

/// CEP:WHAT: Sections are grouped into __TEXT, __DATA, then __DWARF segments, sorted by
///   name inside each group, and n_sect numbers follow the emission order.
/// CEP:WHY: Permission-then-name ordering is the documented layout law.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: NoBits zero-fill maps to __DATA even when allocated; flagless sections map
///   to the __DWARF debug group.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_sections_grouped_and_numbered() {
    let mut o = ObjectFile::new("groups.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    let b = o.add_nobits_section(".bss", 8, 8).expect(".bss");
    let d = o.add_data_section(".data", 8, vec![1; 8]).expect(".data");
    o.sections.push(Section {
        name: ".rodata".into(),
        ty: SectionType::Progbits,
        flags: 0,
        align: 1,
        bytes: vec![2; 4],
        relocs: Vec::new(),
    });
    o.add_symbol("st", true, false, Some(t), 0, 1).expect("st");
    o.add_symbol("sb", true, false, Some(b), 0, 8).expect("sb");
    o.add_symbol("sd", true, false, Some(d), 0, 8).expect("sd");
    o.add_symbol("sr", true, false, Some(o.sections.len() - 1), 0, 4).expect("sr");
    let img = write_macho64(&o).expect("emission");

    let got: Vec<(String, String)> = section_records(&img)
        .iter()
        .map(|(_, r)| (rd_name(r, 0), rd_name(r, 16)))
        .collect();
    assert_eq!(
        got,
        vec![
            ("__text".into(), SEGNAME_TEXT.into()),
            ("__bss".into(), SEGNAME_DATA.into()),
            ("__data".into(), SEGNAME_DATA.into()),
            ("__rodata".into(), SEGNAME_NONALLOC.into()),
        ]
    );
    // Zero-fill record: S_ZEROFILL type and no file bytes.
    let (_, bss) = &section_records(&img)[1];
    assert_eq!(rd_u32(bss, 64) & 0xFF, S_ZEROFILL, "flags carry S_ZEROFILL");
    assert_eq!(rd_u32(bss, 48), 0, "NoBits occupies no file bytes");
    // n_sect of each symbol follows emission order 1..4.
    let (symoff, _, _, _) = symtab_cmd(&img);
    for (i, want) in [1u8, 2, 3, 4].iter().enumerate() {
        assert_eq!(img[symoff + i * MACHO_NLIST_BYTES + 5], *want, "n_sect {i}");
    }
}

/// CEP:WHAT: The UUID payload is a pure function of model content: equal models give equal
///   UUIDs and a one-byte content change changes the UUID.
/// CEP:WHY: Content-derived identity is the reproducible-build requirement.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: UUID bytes live at header_end + 24 (the LC_UUID payload).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_uuid_tracks_content() {
    let uuid_at = MACHO_HEADER_BYTES + MACHO_CMD24_BYTES + 8;
    let a = write_macho64(&golden_object()).expect("emission a");
    let b = write_macho64(&golden_object()).expect("emission b");
    assert_eq!(a[uuid_at..uuid_at + 16], b[uuid_at..uuid_at + 16]);

    let mut changed = golden_object();
    changed.sections[0].bytes[0] ^= 0xFF;
    let c = write_macho64(&changed).expect("emission c");
    assert_ne!(a[uuid_at..uuid_at + 16], c[uuid_at..uuid_at + 16]);
}

/// CEP:WHAT: Two symbols with the same non-empty name are rejected (ambiguous nlist table);
///   anonymous locals ("") may repeat.
/// CEP:WHY: Required negative test (duplicate symbol names).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_duplicate_symbol_names() {
    let mut o = ObjectFile::new("dup.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("dup", true, false, Some(t), 0, 1).expect("first");
    o.add_symbol("dup", true, false, Some(t), 1, 1).expect("second");
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            name: "dup".into(),
            reason: "duplicate symbol name",
        })
    );

    // Anonymous locals are exempt.
    let mut anon = ObjectFile::new("anon.o");
    let ta = anon.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    anon.add_symbol("", true, false, Some(ta), 0, 1).expect("anon 1");
    anon.add_symbol("", true, false, Some(ta), 1, 1).expect("anon 2");
    assert!(write_macho64(&anon).is_ok());
}

/// CEP:WHAT: Two model sections with the same name are rejected.
/// CEP:WHY: Required negative test: (sectname, segname) pairs must stay unambiguous.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_duplicate_section_names() {
    let mut o = ObjectFile::new("dupsec.o");
    o.add_text_section(".text", 16, vec![0xc3]).expect("first");
    o.add_text_section(".text", 16, vec![0xc3]).expect("second");
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            name: ".text".into(),
            reason: "duplicate section name",
        })
    );
}

/// CEP:WHAT: Alignments that are not powers of two and alignments above 2^31 are rejected
///   with structured errors, never wrapped or truncated.
/// CEP:WHY: Required negative tests (alignment law and the MACHO_MAX_ALIGN budget).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: model validation reports non-power-of-two; the writer reports the budget.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_invalid_alignments() {
    let mut o = ObjectFile::new("align.o");
    o.sections.push(Section {
        name: ".misaligned".into(),
        ty: SectionType::Progbits,
        flags: FLAG_ALLOC | FLAG_EXEC,
        align: 3,
        bytes: vec![0x90],
        relocs: Vec::new(),
    });
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::InvalidAlignment {
            section: ".misaligned".into(),
            align: 3,
        })
    );

    let mut big = ObjectFile::new("bigalign.o");
    big.add_text_section(".text", (1u64 << 31) << 1, vec![0x90]).expect("section");
    assert!(matches!(
        write_macho64(&big),
        Err(ObjectError::BudgetExceeded { what: "MACHO_MAX_ALIGN", .. })
    ));
}

/// CEP:WHAT: A section name whose mapped sectname exceeds the 16-byte Mach-O field is
///   rejected (documented choice: reject rather than emit a long-name table).
/// CEP:WHY: Required negative test (oversize names).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: mapping prepends "__" and maps '.'/'-' to '_' before the length check.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_oversize_section_name() {
    let mut o = ObjectFile::new("name.o");
    o.add_text_section(".this_section_name_is_far_too_long", 1, vec![0x90])
        .expect("section stored");
    assert!(matches!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            reason: "mapped Mach-O sectname exceeds 16 bytes",
            ..
        })
    ));
}

/// CEP:WHAT: Global symbols must carry non-empty names.
/// CEP:WHY: Required negative test (empty names are illegal on externals).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_empty_global_symbol_name() {
    let mut o = ObjectFile::new("empty.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("", true, true, Some(t), 0, 1).expect("stored");
    assert!(matches!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            reason: "global symbol requires a non-empty name",
            ..
        })
    ));
}

/// CEP:WHAT: An undefined symbol that is not external is rejected (Mach-O undefined means
///   N_UNDF|N_EXT).
/// CEP:WHY: Required negative test (undefined locals cannot be encoded).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_undefined_local_symbol() {
    let mut o = ObjectFile::new("quiet.o");
    o.add_text_section(".text", 16, vec![0xc3]).expect(".text");
    o.add_symbol("quiet", false, false, None, 0, 0).expect("stored");
    assert!(matches!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            reason: "undefined Mach-O symbols must be external",
            ..
        })
    ));
}

/// CEP:WHAT: A relocation referencing a symbol absent from the object is rejected.
/// CEP:WHY: Required negative test (UnknownRelocSymbol).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_unknown_reloc_symbol() {
    let mut o = ObjectFile::new("missing.o");
    let t = o
        .add_text_section(".text", 16, vec![0xe8, 0, 0, 0, 0, 0xc3])
        .expect(".text");
    o.add_symbol("main", true, true, Some(t), 0, 6).expect("main");
    o.add_reloc(t, 1, RelocKind::PcRel32, "missing_fn", -4).expect("reloc stored");
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::UnknownRelocSymbol { name: "missing_fn".into() })
    );
}

/// CEP:WHAT: A layout whose file cursor crosses the u32 offset budget is rejected before
///   any image allocation.
/// CEP:WHY: Required negative test (checked offsets/sizes; Mach-O offsets are u32).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: two sections aligned 2^31 force a 2^32 cursor without large allocations.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_file_offset_overflow() {
    let mut o = ObjectFile::new("far.o");
    o.add_text_section(".a", 1 << 31, vec![0x90]).expect(".a");
    o.add_text_section(".b", 1 << 31, vec![0x90]).expect(".b");
    match write_macho64(&o) {
        Err(ObjectError::BudgetExceeded { what, .. }) => assert_eq!(what, "MACHO_MAX_FILE_BYTES"),
        other => panic!("expected budget error, got {other:?}"),
    }
}
