// CEP:FILE: crates/rv8_object/tests/serializers.rs
// CEP:WHAT: Structural, golden-byte, and negative tests for the Mach-O 64 and COFF x86-64
//   serializers and for the ELF hot/cold split pass: header and load-command/section-table
//   walks, permission-then-name ordering, symbol partitions and aux records, relocation
//   encoding with materialized addends, content-derived UUID, NOBITS mapping, hand-computed
//   minimal-file goldens, and one structured ObjectError per validation rule.
// CEP:WHY: The serializers must be proven byte-exact (goldens), deterministic, and
//   fail-closed (one negative test per rule) without relying on external parsers.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: little-endian x86-64 host-independent byte expectations (hand-computed
//   fixtures); rv8_core SHA-256 is the content hash used by the Mach-O UUID.
// CEP:COST: test-only; all fixtures are small.
// CEP:EVIDENCE: this file is the evidence; every rule has one test below.

use rv8_object::coff::{
    write_coff, COFF_HEADER_BYTES, COFF_STRTAB_HEADER_BYTES,
    COFF_SYMBOL_BYTES, COFF_TIMESTAMP_ZERO, IMAGE_FILE_MACHINE_AMD64, IMAGE_REL_AMD64_ADDR64,
    IMAGE_REL_AMD64_REL32, IMAGE_SCN_ALIGN_1BYTES, IMAGE_SCN_ALIGN_SHIFT, IMAGE_SCN_CNT_CODE,
    IMAGE_SCN_CNT_UNINITIALIZED_DATA, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ,
    IMAGE_SCN_MEM_WRITE, IMAGE_SYM_CLASS_EXTERNAL, IMAGE_SYM_CLASS_STATIC,
};
use rv8_object::elf::write_elf64_relocatable;
use rv8_object::elf::{SHDR_BYTES, SHT_PROGBITS};
use rv8_object::macho::{
    write_macho64, CPU_SUBTYPE_X86_64_ALL, CPU_TYPE_X86_64, LC_BUILD_VERSION, LC_DYSYMTAB,
    LC_SEGMENT_64, LC_SYMTAB, LC_UUID, MH_MAGIC_64, MH_OBJECT, MACHO_CMD24_BYTES,
    MACHO_DYSYMTAB_CMD_BYTES, MACHO_HEADER_BYTES, MACHO_NLIST_BYTES, MACHO_SECTION_BYTES,
    MACHO_SEGMENT_HEADER_BYTES,
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

/// One-section, one-local-symbol object: the minimal Mach-O golden fixture.
fn macho_minimal() -> ObjectFile {
    let mut o = ObjectFile::new("m.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).unwrap();
    o.add_symbol("f", true, false, Some(t), 0, 1).unwrap();
    o
}

/// One-section, one-global-symbol object: the minimal COFF golden fixture.
fn coff_minimal() -> ObjectFile {
    let mut o = ObjectFile::new("c.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 1).unwrap();
    o
}

/// CEP:WHAT: Walks the Mach-O load commands and checks the fixed order (build version,
///   uuid, segment, symtab, dysymtab), sizes, header identity fields, and the command
///   byte budget.
/// CEP:WHY: Load-command order is part of the determinism contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: one exec section with one global symbol (dysymtab present).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_header_and_load_commands() {
    let mut o = ObjectFile::new("m.o");
    let t = o.add_text_section(".text", 16, vec![0xc3]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 1).unwrap();
    let img = write_macho64(&o).unwrap();
    assert_eq!(rd_u32(&img, 0), MH_MAGIC_64);
    assert_eq!(rd_u32(&img, 4), CPU_TYPE_X86_64);
    assert_eq!(rd_u32(&img, 8), CPU_SUBTYPE_X86_64_ALL);
    assert_eq!(rd_u32(&img, 12), MH_OBJECT);
    let ncmds = rd_u32(&img, 16);
    let sizeofcmds = rd_u32(&img, 20) as usize;
    assert_eq!(ncmds, 5);
    let expected_cmds = 2 * MACHO_CMD24_BYTES
        + MACHO_SEGMENT_HEADER_BYTES
        + MACHO_SECTION_BYTES
        + MACHO_CMD24_BYTES
        + MACHO_DYSYMTAB_CMD_BYTES;
    assert_eq!(sizeofcmds, expected_cmds);

    let mut at = MACHO_HEADER_BYTES;
    let mut seen: Vec<u32> = Vec::new();
    while at < MACHO_HEADER_BYTES + sizeofcmds {
        let cmd = rd_u32(&img, at);
        let size = rd_u32(&img, at + 4) as usize;
        seen.push(cmd);
        at += size;
    }
    assert_eq!(
        seen,
        vec![LC_BUILD_VERSION, LC_UUID, LC_SEGMENT_64, LC_SYMTAB, LC_DYSYMTAB]
    );
    assert_eq!(at, MACHO_HEADER_BYTES + sizeofcmds);
    // Fixed build version payload: macOS platform, minos and sdk 11.0.0.
    assert_eq!(rd_u32(&img, MACHO_HEADER_BYTES + 8), 1); // PLATFORM_MACOS
    assert_eq!(rd_u32(&img, MACHO_HEADER_BYTES + 12), 0x000B_0000);
    assert_eq!(rd_u32(&img, MACHO_HEADER_BYTES + 16), 0x000B_0000);
}

/// CEP:WHAT: Sections are grouped into __TEXT, __DATA, then __DWARF segments, and sorted
///   by name inside each segment.
/// CEP:WHY: "Permission then name" is the documented Mach-O ordering rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: segment mapping from the writer contract (NoBits always in __DATA).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_sections_grouped_by_permission_then_name() {
    let mut o = ObjectFile::new("order.o");
    o.add_text_section(".text", 1, vec![0x90]).unwrap();
    o.add_data_section(".data", 1, vec![0u8; 4]).unwrap();
    o.add_data_section(".rodata", 1, vec![0u8; 4]).unwrap();
    o.add_nobits_section(".bss", 1, 16).unwrap();
    o.add_section(".debug_info", SectionType::Progbits, 0, 1, vec![0u8; 4]).unwrap();
    let img = write_macho64(&o).unwrap();
    // Walk to the three segment commands (third, fourth, fifth load commands).
    let mut at = MACHO_HEADER_BYTES + 2 * MACHO_CMD24_BYTES;
    let mut order: Vec<(String, String)> = Vec::new();
    for cmd_idx in 0..3u32 {
        assert_eq!(rd_u32(&img, at), LC_SEGMENT_64, "segment command {cmd_idx}");
        let nsects = rd_u32(&img, at + 64);
        let segname = String::from_utf8_lossy(&img[at + 8..at + 24])
            .trim_end_matches('\0')
            .to_string();
        for s in 0..nsects {
            let rec = at + 72 + (s as usize) * MACHO_SECTION_BYTES;
            let sectname = String::from_utf8_lossy(&img[rec..rec + 16])
                .trim_end_matches('\0')
                .to_string();
            order.push((segname.clone(), sectname));
        }
        at += rd_u32(&img, at + 4) as usize;
    }
    assert_eq!(
        order,
        vec![
            ("__TEXT".to_string(), "__text".to_string()),
            ("__DATA".to_string(), "__bss".to_string()),
            ("__DATA".to_string(), "__data".to_string()),
            ("__DATA".to_string(), "__rodata".to_string()),
            ("__DWARF".to_string(), "__debug_info".to_string()),
        ]
    );
}

/// CEP:WHAT: The nlist_64 table is partitioned locals, defined externals, undefined
///   externals (model order inside each class), with correct n_strx, n_type, n_sect, and
///   n_value, and LC_DYSYMTAB counts match.
/// CEP:WHY: Mach-O symbol conventions and the determinism contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_symbol_table_partition() {
    let mut o = ObjectFile::new("syms.o");
    let t = o.add_text_section(".text", 1, vec![0x90; 8]).unwrap();
    o.add_symbol("l0", true, false, Some(t), 0, 2).unwrap();
    o.add_symbol("g0", true, true, Some(t), 4, 4).unwrap();
    o.add_symbol("ext", false, true, None, 0, 0).unwrap();
    o.add_symbol("l1", true, false, Some(t), 6, 2).unwrap();
    o.add_symbol("", true, false, Some(t), 0, 0).unwrap(); // anonymous local
    let img = write_macho64(&o).unwrap();

    // Find LC_SYMTAB in the command walk.
    let mut at = MACHO_HEADER_BYTES;
    let (symoff, nsyms, stroff, strsize);
    let (dysym_at, symtab_at);
    loop {
        let cmd = rd_u32(&img, at);
        let size = rd_u32(&img, at + 4) as usize;
        if cmd == LC_SYMTAB {
            symtab_at = at;
            symoff = rd_u32(&img, at + 8) as usize;
            nsyms = rd_u32(&img, at + 12);
            stroff = rd_u32(&img, at + 16) as usize;
            strsize = rd_u32(&img, at + 20);
            break;
        }
        at += size;
    }
    let mut cursor = symtab_at + MACHO_CMD24_BYTES;
    loop {
        let cmd = rd_u32(&img, cursor);
        let size = rd_u32(&img, cursor + 4) as usize;
        if cmd == LC_DYSYMTAB {
            dysym_at = cursor;
            break;
        }
        cursor += size;
    }
    assert_eq!(rd_u32(&img, dysym_at + 12), 3); // nlocalsym
    assert_eq!(rd_u32(&img, dysym_at + 16), 3); // iextdefsym
    assert_eq!(rd_u32(&img, dysym_at + 20), 1); // nextdefsym
    assert_eq!(rd_u32(&img, dysym_at + 24), 4); // iundefsym
    assert_eq!(rd_u32(&img, dysym_at + 28), 1); // nundefsym

    assert_eq!(nsyms, 5);
    assert_eq!(strsize as usize, 14); // \0 l0\0 l1\0 g0\0 ext\0 ("" adds nothing)
    let nlist = |i: usize| &img[symoff + i * MACHO_NLIST_BYTES..symoff + (i + 1) * MACHO_NLIST_BYTES];
    // Order: l0, l1, "" (locals), g0 (extdef), ext (undef).
    let expect = [
        (1u32, 0x0Eu8, 1u8, 0u64),  // l0
        (4u32, 0x0Eu8, 1u8, 6u64),  // l1
        (0u32, 0x0Eu8, 1u8, 0u64),  // anonymous local
        (7u32, 0x0Fu8, 1u8, 4u64),  // g0
        (10u32, 0x01u8, 0u8, 0u64), // ext
    ];
    for (i, (strx, ty, sect, value)) in expect.iter().enumerate() {
        let rec = nlist(i);
        assert_eq!(rd_u32(rec, 0), *strx, "n_strx {i}");
        assert_eq!(rec[4], *ty, "n_type {i}");
        assert_eq!(rec[5], *sect, "n_sect {i}");
        assert_eq!(rd_u64(rec, 8), *value, "n_value {i}");
    }
    assert_eq!(&img[stroff..stroff + strsize as usize], b"\0l0\0l1\0g0\0ext\0");
}

/// CEP:WHAT: PcRel32 relocations become X86_64_RELOC_SIGNED (pcrel, length 4, extern)
///   and Abs64 becomes X86_64_RELOC_UNSIGNED (length 8, extern); the model addend is
///   materialized into the patched field bytes.
/// CEP:WHY: The REL-style addend conversion is the load-bearing encoding rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: g0 (extdef) gets emitted index 0, ext (undef) index 1.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_relocation_encoding() {
    let mut o = ObjectFile::new("reloc.o");
    let mut bytes = vec![0xe8, 0, 0, 0, 0]; // call rel32 (field at 1)
    bytes.extend_from_slice(&[0u8; 11]); // abs64 slot at 8 (section is 16 bytes)
    let t = o.add_text_section(".text", 1, bytes).unwrap();
    o.add_symbol("g0", true, true, Some(t), 0, 4).unwrap();
    o.add_symbol("ext", false, true, None, 0, 0).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "ext", -4).unwrap();
    o.add_reloc(t, 8, RelocKind::Abs64, "g0", 0x10).unwrap();
    let img = write_macho64(&o).unwrap();

    // Patched field bytes (the rel32 FIELD sits at section offset 1, after the e8 opcode).
    let mut at = MACHO_HEADER_BYTES;
    let (data_off, reloff, nreloc);
    loop {
        let cmd = rd_u32(&img, at);
        let size = rd_u32(&img, at + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            let rec = at + MACHO_SEGMENT_HEADER_BYTES;
            data_off = rd_u32(&img, rec + 48) as usize;
            reloff = rd_u32(&img, rec + 56) as usize;
            nreloc = rd_u32(&img, rec + 60);
            break;
        }
        at += size;
    }
    assert_eq!(&img[data_off + 1..data_off + 5], &0xFFFF_FFFCu32.to_le_bytes());
    assert_eq!(rd_u64(&img, data_off + 8), 0x10);
    assert_eq!(nreloc, 2);
    let entry = |i: usize| &img[reloff + i * 8..reloff + (i + 1) * 8];
    assert_eq!(rd_u32(entry(0), 0), 1); // r_address
    let info0 = rd_u32(entry(0), 4);
    assert_eq!(info0 & 0x00FF_FFFF, 1); // "ext"
    assert_eq!(info0 >> 24 & 1, 1); // pcrel
    assert_eq!(info0 >> 25 & 3, 2); // length 4
    assert_eq!(info0 >> 27 & 1, 1); // extern
    assert_eq!(info0 >> 28, 1); // X86_64_RELOC_SIGNED
    assert_eq!(rd_u32(entry(1), 0), 8);
    let info1 = rd_u32(entry(1), 4);
    assert_eq!(info1 & 0x00FF_FFFF, 0); // "g0"
    assert_eq!(info1 >> 24 & 1, 0); // not pcrel
    assert_eq!(info1 >> 25 & 3, 3); // length 8
    assert_eq!(info1 >> 27 & 1, 1); // extern
    assert_eq!(info1 >> 28, 0); // X86_64_RELOC_UNSIGNED
}

/// CEP:WHAT: The LC_UUID payload is the SHA-256-derived content id: identical content
///   yields identical UUIDs, changed content does not.
/// CEP:WHY: Reproducible builds need content-derived, wall-clock-free identifiers.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: UUID bytes sit at 64..80 (second load command payload).
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_uuid_is_content_derived() {
    let a = write_macho64(&macho_minimal()).unwrap();
    let b = write_macho64(&macho_minimal()).unwrap();
    assert_eq!(a[64..80], b[64..80]);
    let mut c = ObjectFile::new("m.o");
    let t = c.add_text_section(".text", 16, vec![0xc3]).unwrap();
    c.add_symbol("f", true, false, Some(t), 0, 2).unwrap();
    let other = write_macho64(&c).unwrap();
    assert_ne!(a[64..80], other[64..80], "different symbol size must change the UUID");
}

/// CEP:WHAT: NOBITS sections become S_ZEROFILL records in __DATA with a reserved size,
///   no file offset, and vmsize above filesize.
/// CEP:WHY: Zero-fill storage must not grow the file.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: single data section.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_nobits_zerofill() {
    let mut o = ObjectFile::new("bss.o");
    o.add_nobits_section(".bss", 8, 32).unwrap();
    let img = write_macho64(&o).unwrap();
    let mut at = MACHO_HEADER_BYTES;
    loop {
        let cmd = rd_u32(&img, at);
        let size = rd_u32(&img, at + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            let rec = at + MACHO_SEGMENT_HEADER_BYTES;
            assert_eq!(rd_u32(&img, rec + 64), 1); // S_ZEROFILL flags word
            assert_eq!(rd_u32(&img, rec + 48), 0); // file offset (u32 field)
            assert_eq!(rd_u64(&img, rec + 40), 32); // reserved size
            let vmsize = rd_u64(&img, at + 32);
            let filesize = rd_u64(&img, at + 48);
            assert_eq!(vmsize, 32);
            assert_eq!(filesize, 0);
            break;
        }
        at += size;
    }
}

/// CEP:WHAT: A minimal one-section one-symbol Mach-O object is byte-exact against the
///   hand-computed 283-byte fixture (UUID payload excluded; pinned by its own test).
/// CEP:WHY: Field-order regressions must break loudly.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: header 32 + commands 224, data at 256, symtab at 264, strtab at 280.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_minimal_golden_bytes() {
    let img = write_macho64(&macho_minimal()).unwrap();
    assert_eq!(img.len(), 283);
    let prefix: [u8; 64] = [
        0xcf, 0xfa, 0xed, 0xfe, // magic
        0x07, 0x00, 0x00, 0x01, // cputype x86_64
        0x03, 0x00, 0x00, 0x00, // cpusubtype
        0x01, 0x00, 0x00, 0x00, // MH_OBJECT
        0x04, 0x00, 0x00, 0x00, // ncmds
        0xe0, 0x00, 0x00, 0x00, // sizeofcmds 224
        0x00, 0x00, 0x00, 0x00, // flags
        0x00, 0x00, 0x00, 0x00, // reserved
        0x25, 0x00, 0x00, 0x00, // LC_BUILD_VERSION
        0x18, 0x00, 0x00, 0x00, // cmdsize 24
        0x01, 0x00, 0x00, 0x00, // PLATFORM_MACOS
        0x00, 0x00, 0x0b, 0x00, // minos 11.0.0
        0x00, 0x00, 0x0b, 0x00, // sdk 11.0.0
        0x00, 0x00, 0x00, 0x00, // (build cmd padding to 24)
        0x1b, 0x00, 0x00, 0x00, // LC_UUID
        0x18, 0x00, 0x00, 0x00, // cmdsize 24
    ];
    assert_eq!(&img[0..64], &prefix);
    let middle: [u8; 176] = [
        0x19, 0x00, 0x00, 0x00, // LC_SEGMENT_64
        0x98, 0x00, 0x00, 0x00, // cmdsize 152
        b'_', b'_', b'T', b'E', b'X', b'T', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // segname
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // vmaddr 256
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // vmsize 1
        0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // fileoff 256
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // filesize 1
        0x07, 0x00, 0x00, 0x00, // maxprot
        0x05, 0x00, 0x00, 0x00, // initprot RX
        0x01, 0x00, 0x00, 0x00, // nsects
        0x00, 0x00, 0x00, 0x00, // seg flags
        b'_', b'_', b't', b'e', b'x', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // sectname
        b'_', b'_', b'T', b'E', b'X', b'T', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // segname
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // addr 0 (MH_OBJECT)
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // size 1
        0x00, 0x01, 0x00, 0x00, // offset 256
        0x04, 0x00, 0x00, 0x00, // align log2 4
        0x00, 0x00, 0x00, 0x00, // reloff
        0x00, 0x00, 0x00, 0x00, // nreloc
        0x00, 0x04, 0x00, 0x80, // PURE|SOME instructions
        0x00, 0x00, 0x00, 0x00, // reserved1
        0x00, 0x00, 0x00, 0x00, // reserved2
        0x00, 0x00, 0x00, 0x00, // reserved3
        0x02, 0x00, 0x00, 0x00, // LC_SYMTAB
        0x18, 0x00, 0x00, 0x00, // cmdsize 24
        0x08, 0x01, 0x00, 0x00, // symoff 264
        0x01, 0x00, 0x00, 0x00, // nsyms 1
        0x18, 0x01, 0x00, 0x00, // stroff 280
        0x03, 0x00, 0x00, 0x00, // strsize 3
    ];
    assert_eq!(&img[80..256], &middle);
    assert_eq!(img[256], 0xc3);
    assert!(img[257..264].iter().all(|b| *b == 0));
    let nlist: [u8; 16] = [
        0x01, 0x00, 0x00, 0x00, // n_strx ("f")
        0x0e, // N_SECT local
        0x01, // n_sect
        0x00, 0x00, // n_desc
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // n_value 0
    ];
    assert_eq!(&img[264..280], &nlist);
    assert_eq!(&img[280..283], b"\0f\0");
}

/// CEP:WHAT: Duplicate section names are rejected with a structured error.
/// CEP:WHY: Required negative test; (sectname, segname) must stay unambiguous.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_duplicate_section_names() {
    let mut o = ObjectFile::new("dup.o");
    o.add_text_section(".text", 1, vec![0x90]).unwrap();
    o.add_text_section(".text", 1, vec![0x90]).unwrap();
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            name: ".text".into(),
            reason: "duplicate section name",
        })
    );
}

/// CEP:WHAT: Global symbols with empty names are rejected; undefined locals are rejected.
/// CEP:WHY: Required negative tests; both are unencodable in Mach-O/COFF symbol tables.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_unencodable_symbols() {
    let mut o = ObjectFile::new("anon.o");
    let t = o.add_text_section(".text", 1, vec![0x90]).unwrap();
    o.add_symbol("", true, true, Some(t), 0, 1).unwrap();
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::InvalidName {
            name: "".into(),
            reason: "global symbol requires a non-empty name",
        })
    );
    let mut o2 = ObjectFile::new("locundef.o");
    o2.add_text_section(".text", 1, vec![0x90]).unwrap();
    o2.add_symbol("quiet", false, false, None, 0, 0).unwrap();
    assert_eq!(
        write_macho64(&o2),
        Err(ObjectError::InvalidName {
            name: "quiet".into(),
            reason: "undefined Mach-O symbols must be external",
        })
    );
}

/// CEP:WHAT: A file cursor above the u32 offset budget is rejected before any allocation;
///   alignment alone drives the cursor past the budget.
/// CEP:WHY: Required negative test (section offset overflow); Mach-O offset fields are
///   u32 and must never wrap.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: two sections aligned 2^31 force a 2^32 cursor.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_section_offset_overflow() {
    let mut o = ObjectFile::new("far.o");
    o.add_text_section(".a", 1 << 31, vec![0x90]).unwrap();
    o.add_text_section(".b", 1 << 31, vec![0x90]).unwrap();
    match write_macho64(&o) {
        Err(ObjectError::BudgetExceeded { what, .. }) => assert_eq!(what, "MACHO_MAX_FILE_BYTES"),
        other => panic!("expected budget error, got {other:?}"),
    }
}

/// CEP:WHAT: Alignments above 2^31 and invalid (non-power-of-two) alignments are
///   rejected; oversize mapped section names are rejected.
/// CEP:WHY: Required negative tests for the Mach-O alignment and name budgets.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_rejects_align_and_name_budgets() {
    let mut o = ObjectFile::new("align.o");
    o.add_text_section(".text", (1 << 31) << 1, vec![0x90]).unwrap();
    assert!(matches!(
        write_macho64(&o),
        Err(ObjectError::BudgetExceeded { what: "MACHO_MAX_ALIGN", .. })
    ));

    let bad = Section {
        name: ".bad".into(),
        ty: SectionType::Progbits,
        flags: FLAG_ALLOC | FLAG_EXEC,
        align: 3,
        bytes: vec![0x90],
        relocs: Vec::new(),
    };
    let mut o2 = ObjectFile::new("bad.o");
    o2.sections.push(bad);
    assert_eq!(
        write_macho64(&o2),
        Err(ObjectError::InvalidAlignment { section: ".bad".into(), align: 3 })
    );

    let mut o3 = ObjectFile::new("name.o");
    o3.add_text_section(".this_section_name_is_far_too_long", 1, vec![0x90]).unwrap();
    assert!(matches!(
        write_macho64(&o3),
        Err(ObjectError::InvalidName { reason: "mapped Mach-O sectname exceeds 16 bytes", .. })
    ));
}

/// CEP:WHAT: Two emissions of the same multi-section, reloc-bearing object are
///   byte-identical.
/// CEP:WHY: Local determinism re-check next to the writer.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test; the cross-run matrix lives in tests/acceptance.rs.
#[test]
fn macho_determinism() {
    let mut o = ObjectFile::new("det.o");
    let mut bytes = vec![0xe8, 0, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 11]);
    let t = o.add_text_section(".text", 16, bytes).unwrap();
    let d = o.add_data_section(".data", 8, vec![7u8; 8]).unwrap();
    o.add_nobits_section(".bss", 8, 64).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 5).unwrap();
    o.add_symbol("v", true, true, Some(d), 0, 8).unwrap();
    o.add_symbol("ext", false, true, None, 0, 0).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "ext", -4).unwrap();
    o.add_reloc(t, 8, RelocKind::Abs64, "v", 0).unwrap();
    let a = write_macho64(&o).unwrap();
    let b = write_macho64(&o).unwrap();
    assert_eq!(a, b);
}

/// CEP:WHAT: Walks the COFF header and section table: machine, zero timestamp, symbol
///   pointers, and characteristics derived from permissions.
/// CEP:WHY: Header field order and permission mapping are the COFF contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: one exec section aligned 16.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_header_and_section_table() {
    let img = write_coff(&coff_minimal()).unwrap();
    assert_eq!(rd_u16(&img, 0), IMAGE_FILE_MACHINE_AMD64);
    assert_eq!(rd_u16(&img, 2), 1); // NumberOfSections
    assert_eq!(rd_u32(&img, 4), COFF_TIMESTAMP_ZERO);
    assert_eq!(rd_u16(&img, 18), 0); // SizeOfOptionalHeader
    let sym_ptr = rd_u32(&img, 8) as usize;
    let n_syms = rd_u32(&img, 12);
    assert_eq!(n_syms, 3); // section symbol + aux + "f"
    let at = COFF_HEADER_BYTES;
    assert_eq!(&img[at..at + 5], b".text");
    assert_eq!(rd_u32(&img, at + 16), 1); // SizeOfRawData
    assert_eq!(rd_u32(&img, at + 20), 60); // PointerToRawData: data follows the section table
    assert_eq!(rd_u16(&img, at + 32), 0); // NumberOfRelocations
    let chars = rd_u32(&img, at + 36);
    assert_eq!(
        chars,
        IMAGE_SCN_CNT_CODE
            | IMAGE_SCN_MEM_READ
            | IMAGE_SCN_MEM_EXECUTE
            | IMAGE_SCN_ALIGN_1BYTES
            | (4 << IMAGE_SCN_ALIGN_SHIFT)
    );
    // Symbol table walks: section symbol (STATIC + aux), aux record, then "f" (EXTERNAL).
    let rec = |i: usize| &img[sym_ptr + i * COFF_SYMBOL_BYTES..sym_ptr + (i + 1) * COFF_SYMBOL_BYTES];
    assert_eq!(&rec(0)[..5], b".text");
    assert_eq!(rd_u16(rec(0), 12), 1); // SectionNumber
    assert_eq!(rec(0)[16], IMAGE_SYM_CLASS_STATIC);
    assert_eq!(rec(0)[17], 1); // NumberOfAuxSymbols
    assert_eq!(rd_u32(rec(1), 0), 1); // aux Length
    assert_eq!(rd_u16(rec(1), 12), 1); // aux Number (offset 12 per the PE spec)
    assert_eq!(rec(1)[14], 0); // aux Selection (no COMDAT in the model)
    assert_eq!(rec(2)[0], b'f');
    assert_eq!(rd_u16(rec(2), 12), 1); // SectionNumber
    assert_eq!(rec(2)[16], IMAGE_SYM_CLASS_EXTERNAL);
    assert_eq!(&img[sym_ptr + (n_syms as usize) * COFF_SYMBOL_BYTES..][..4], &[4, 0, 0, 0]);
}

/// CEP:WHAT: Long section and symbol names become "/offset" fields into the COFF string
///   table, offsets are relative to the size field, and suffix sharing applies.
/// CEP:WHY: The long-name mechanism must round-trip exactly.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: ".long_shared_name" shares its suffix with the symbol "long_shared_name".
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_long_name_string_table() {
    let mut o = ObjectFile::new("long.o");
    let s = o
        .add_section(".long_shared_name", SectionType::Progbits, FLAG_ALLOC, 1, vec![0u8; 4])
        .unwrap();
    o.add_symbol("long_shared_name", true, true, Some(s), 0, 4).unwrap();
    let img = write_coff(&o).unwrap();
    let at = COFF_HEADER_BYTES;
    let name_field = &img[at..at + 8];
    assert_eq!(name_field[0], b'/');
    let sec_off: usize = std::str::from_utf8(&name_field[1..8])
        .unwrap()
        .trim_end_matches('\0')
        .parse()
        .unwrap();
    let sym_ptr = rd_u32(&img, 8) as usize;
    let n_syms = rd_u32(&img, 12) as usize;
    let strtab_at = sym_ptr + n_syms * COFF_SYMBOL_BYTES;
    let strtab_size = rd_u32(&img, strtab_at) as usize;
    // String table: size field + ".long_shared_name\0" (the symbol name suffix-shares).
    assert_eq!(strtab_size, COFF_STRTAB_HEADER_BYTES + ".long_shared_name".len() + 1);
    assert_eq!(
        &img[strtab_at + sec_off..strtab_at + sec_off + ".long_shared_name".len()],
        b".long_shared_name"
    );
    let sym_name = &img[sym_ptr + 2 * COFF_SYMBOL_BYTES..sym_ptr + 2 * COFF_SYMBOL_BYTES + 8];
    assert_eq!(sym_name[0], b'/');
    let sym_off: usize = std::str::from_utf8(&sym_name[1..8])
        .unwrap()
        .trim_end_matches('\0')
        .parse()
        .unwrap();
    // Suffix sharing: the symbol name points one byte into the section name's bytes.
    assert_eq!(sym_off, sec_off + 1);
    assert_eq!(
        &img[strtab_at + sym_off..strtab_at + sym_off + "long_shared_name".len()],
        b"long_shared_name"
    );
}

/// CEP:WHAT: PcRel32 becomes IMAGE_REL_AMD64_REL32 and Abs64 becomes
///   IMAGE_REL_AMD64_ADDR64 with symbol-table indices resolved by first occurrence, and
///   the model addend is materialized into the patched field bytes.
/// CEP:WHY: The REL-style conversion and index resolution are the load-bearing rules.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: symbol order: section symbol, aux, g0, ext.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_relocation_encoding() {
    let mut o = ObjectFile::new("reloc.o");
    let mut bytes = vec![0xe8, 0, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 11]);
    let t = o.add_text_section(".text", 1, bytes).unwrap();
    o.add_symbol("g0", true, true, Some(t), 0, 4).unwrap();
    o.add_symbol("ext", false, true, None, 0, 0).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "ext", -4).unwrap();
    o.add_reloc(t, 8, RelocKind::Abs64, "g0", 0x10).unwrap();
    let img = write_coff(&o).unwrap();
    let at = COFF_HEADER_BYTES;
    let data_off = rd_u32(&img, at + 20) as usize;
    let reloc_off = rd_u32(&img, at + 24) as usize;
    assert_eq!(&img[data_off + 1..data_off + 5], &0xFFFF_FFFCu32.to_le_bytes());
    assert_eq!(rd_u64(&img, data_off + 8), 0x10);
    let entry = |i: usize| &img[reloc_off + i * 10..reloc_off + (i + 1) * 10];
    assert_eq!(rd_u32(entry(0), 0), 1); // VirtualAddress
    assert_eq!(rd_u32(entry(0), 4), 3); // "ext" record index
    assert_eq!(rd_u16(entry(0), 8), IMAGE_REL_AMD64_REL32);
    assert_eq!(rd_u32(entry(1), 0), 8);
    assert_eq!(rd_u32(entry(1), 4), 2); // "g0" record index
    assert_eq!(rd_u16(entry(1), 8), IMAGE_REL_AMD64_ADDR64);
}

/// CEP:WHAT: NOBITS sections map to CNT_UNINITIALIZED_DATA with a zero raw pointer, a
///   reserved SizeOfRawData, and MEM_READ|MEM_WRITE permissions.
/// CEP:WHY: .bss-style reservations must not grow the file.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_nobits_uninitialized() {
    let mut o = ObjectFile::new("bss.o");
    o.add_nobits_section(".bss", 8, 128).unwrap();
    let img = write_coff(&o).unwrap();
    let at = COFF_HEADER_BYTES;
    assert_eq!(&img[at..at + 4], b".bss");
    assert_eq!(rd_u32(&img, at + 16), 128); // SizeOfRawData (reserved)
    assert_eq!(rd_u32(&img, at + 20), 0); // PointerToRawData
    let chars = rd_u32(&img, at + 36);
    assert_eq!(
        chars,
        IMAGE_SCN_CNT_UNINITIALIZED_DATA
            | IMAGE_SCN_MEM_READ
            | IMAGE_SCN_MEM_WRITE
            | IMAGE_SCN_ALIGN_1BYTES
            | (3 << IMAGE_SCN_ALIGN_SHIFT)
    );
}

/// CEP:WHAT: A minimal one-section one-symbol COFF object is byte-exact against the
///   hand-computed 122-byte fixture.
/// CEP:WHY: Field-order regressions must break loudly.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: header 20, section table 20..60, data 60, symtab 64..118, strtab 118.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_minimal_golden_bytes() {
    let img = write_coff(&coff_minimal()).unwrap();
    assert_eq!(img.len(), 122);
    let expected: [u8; 122] = [
        0x64, 0x86, // Machine AMD64
        0x01, 0x00, // NumberOfSections
        0x00, 0x00, 0x00, 0x00, // TimeDateStamp
        0x40, 0x00, 0x00, 0x00, // PointerToSymbolTable 64
        0x03, 0x00, 0x00, 0x00, // NumberOfSymbols 3
        0x00, 0x00, // SizeOfOptionalHeader
        0x00, 0x00, // Characteristics
        // Section table (20..60).
        b'.', b't', b'e', b'x', b't', 0, 0, 0, // Name
        0x00, 0x00, 0x00, 0x00, // VirtualSize
        0x00, 0x00, 0x00, 0x00, // VirtualAddress
        0x01, 0x00, 0x00, 0x00, // SizeOfRawData
        0x3c, 0x00, 0x00, 0x00, // PointerToRawData 60
        0x00, 0x00, 0x00, 0x00, // PointerToRelocations
        0x00, 0x00, 0x00, 0x00, // PointerToLinenumbers
        0x00, 0x00, // NumberOfRelocations
        0x00, 0x00, // NumberOfLinenumbers
        0x20, 0x00, 0x50, 0x60, // Characteristics CNT_CODE|ALIGN_16|MEM_X|MEM_R
        // Raw data (60) + padding to 64.
        0xc3, 0x00, 0x00, 0x00,
        // Section symbol (64).
        b'.', b't', b'e', b'x', b't', 0, 0, 0,
        0x00, 0x00, 0x00, 0x00, // Value
        0x01, 0x00, // SectionNumber
        0x00, 0x00, // Type
        0x03, // STATIC
        0x01, // NumberOfAuxSymbols
        // Auxiliary record (82).
        0x01, 0x00, 0x00, 0x00, // Length
        0x00, 0x00, // NumberOfRelocations
        0x00, 0x00, // NumberOfLinenumbers
        0x00, 0x00, 0x00, 0x00, // CheckSum
        0x01, 0x00, // Number
        0x00, // Selection
        0x00, 0x00, 0x00, // Reserved
        // Symbol "f" (100).
        b'f', 0, 0, 0, 0, 0, 0, 0,
        0x00, 0x00, 0x00, 0x00, // Value 0
        0x01, 0x00, // SectionNumber 1
        0x00, 0x00, // Type
        0x02, // EXTERNAL
        0x00, // NumberOfAuxSymbols
        // String table (118): size field only.
        0x04, 0x00, 0x00, 0x00,
    ];
    assert_eq!(img, expected);
}

/// CEP:WHAT: Alignments above 8192 and non-power-of-two alignments are rejected.
/// CEP:WHY: Required negative tests for the COFF alignment budget.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_rejects_align_budgets() {
    let mut o = ObjectFile::new("align.o");
    o.add_text_section(".text", 16384, vec![0x90]).unwrap();
    assert!(matches!(
        write_coff(&o),
        Err(ObjectError::BudgetExceeded { what: "COFF_MAX_ALIGN", .. })
    ));
    let bad = Section {
        name: ".bad".into(),
        ty: SectionType::Progbits,
        flags: FLAG_ALLOC | FLAG_EXEC,
        align: 6,
        bytes: vec![0x90],
        relocs: Vec::new(),
    };
    let mut o2 = ObjectFile::new("bad.o");
    o2.sections.push(bad);
    assert_eq!(
        write_coff(&o2),
        Err(ObjectError::InvalidAlignment { section: ".bad".into(), align: 6 })
    );
}

/// CEP:WHAT: A relocation count above the 16-bit NumberOfRelocations field is rejected.
/// CEP:WHY: Required negative test; the count-escape record is deliberately not emitted.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: overlapping relocation offsets are legal in the model, so a 4-byte
///   section can carry 65536 entries.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_rejects_reloc_count_overflow() {
    let mut o = ObjectFile::new("many.o");
    let t = o.add_text_section(".text", 1, vec![0u8; 8]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 8).unwrap();
    for _ in 0..65536u32 {
        o.add_reloc(t, 0, RelocKind::Abs64, "f", 0).unwrap();
    }
    assert!(matches!(
        write_coff(&o),
        Err(ObjectError::BudgetExceeded { what: "COFF_MAX_RELOCS_PER_SECTION", .. })
    ));
}

/// CEP:WHAT: Duplicate section names, empty global names, and undefined locals are
///   rejected by the COFF writer.
/// CEP:WHY: Required negative tests for the name rules.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn coff_rejects_name_rules() {
    let mut o = ObjectFile::new("dup.o");
    o.add_text_section(".text", 1, vec![0x90]).unwrap();
    o.add_text_section(".text", 1, vec![0x90]).unwrap();
    assert_eq!(
        write_coff(&o),
        Err(ObjectError::InvalidName {
            name: ".text".into(),
            reason: "duplicate section name",
        })
    );
    let mut o2 = ObjectFile::new("anon.o");
    let t2 = o2.add_text_section(".text", 1, vec![0x90]).unwrap();
    o2.add_symbol("", true, true, Some(t2), 0, 1).unwrap();
    assert_eq!(
        write_coff(&o2),
        Err(ObjectError::InvalidName {
            name: "".into(),
            reason: "global symbol requires a non-empty name",
        })
    );
    let mut o3 = ObjectFile::new("locundef.o");
    o3.add_text_section(".text", 1, vec![0x90]).unwrap();
    o3.add_symbol("quiet", false, false, None, 0, 0).unwrap();
    assert_eq!(
        write_coff(&o3),
        Err(ObjectError::InvalidName {
            name: "quiet".into(),
            reason: "undefined COFF symbols must be external",
        })
    );
}

/// CEP:WHAT: A relocation naming a symbol absent from the object fails closed in both
///   new serializers.
/// CEP:WHY: Required negative test (unknown relocation symbol).
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn macho_coff_reject_unknown_reloc_symbol() {
    let mut o = ObjectFile::new("ghost.o");
    let t = o.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 5).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "ghost", -4).unwrap();
    assert_eq!(
        write_macho64(&o),
        Err(ObjectError::UnknownRelocSymbol { name: "ghost".into() })
    );
    assert_eq!(
        write_coff(&o),
        Err(ObjectError::UnknownRelocSymbol { name: "ghost".into() })
    );
}

/// CEP:WHAT: Two emissions of the same multi-section COFF object are byte-identical.
/// CEP:WHY: Local determinism re-check next to the writer.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test; the cross-run matrix lives in tests/acceptance.rs.
#[test]
fn coff_determinism() {
    let mut o = ObjectFile::new("det.o");
    let t = o.add_text_section(".text", 16, vec![0xe8, 0, 0, 0, 0]).unwrap();
    let d = o.add_data_section(".data", 8, vec![7u8; 8]).unwrap();
    o.add_nobits_section(".bss", 8, 64).unwrap();
    o.add_symbol("f", true, true, Some(t), 0, 5).unwrap();
    o.add_symbol("v", true, true, Some(d), 0, 8).unwrap();
    o.add_symbol("ext", false, true, None, 0, 0).unwrap();
    o.add_reloc(t, 1, RelocKind::PcRel32, "ext", -4).unwrap();
    let a = write_coff(&o).unwrap();
    let b = write_coff(&o).unwrap();
    assert_eq!(a, b);
}

/// CEP:WHAT: The hot/cold split places hot executable sections before cold ones (names
///   ending in ".cold") inside the ELF RX group, with cold sections last.
/// CEP:WHY: design.md 1618-1622 ordering rule; HOT_COLD_SPLIT is now active.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: emitted section order is readable from the section header table and
///   .shstrtab.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn elf_hot_cold_split_orders_hot_first() {
    let mut o = ObjectFile::new("hc.o");
    o.add_text_section(".text.cold", 64, vec![0x90]).unwrap();
    o.add_text_section(".text", 16, vec![0x90]).unwrap();
    o.add_text_section(".text.helper", 8, vec![0x90]).unwrap();
    o.add_symbol("f", true, true, Some(1), 0, 1).unwrap();
    let img = write_elf64_relocatable(&o).unwrap();
    let shoff = rd_u64(&img, 0x28) as usize;
    let shnum = rd_u16(&img, 0x3c) as usize;
    let sh = |i: usize| &img[shoff + i * SHDR_BYTES..shoff + (i + 1) * SHDR_BYTES];
    let str_hdr = sh(shnum - 1);
    let strtab_at = rd_u64(str_hdr, 24) as usize;
    let strtab_len = rd_u64(str_hdr, 32) as usize;
    let shstr = &img[strtab_at..strtab_at + strtab_len];
    let name_of = |rec: &[u8]| -> String {
        let off = rd_u32(rec, 0) as usize;
        let end = shstr[off..].iter().position(|b| *b == 0).unwrap() + off;
        String::from_utf8_lossy(&shstr[off..end]).into_owned()
    };
    // Slots: 0 null, 1..4 input sections, then generated tables.
    let names: Vec<String> = (1..4).map(|i| name_of(sh(i))).collect();
    assert_eq!(names, vec![".text", ".text.helper", ".text.cold"]);
    // All three input sections remain executable PROGBITS.
    for i in 1..4 {
        assert_eq!(rd_u32(sh(i), 4), SHT_PROGBITS);
    }
}
