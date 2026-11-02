// CEP:FILE: crates/rv8_object/tests/coff_dwarf_serialization/dwarf_elf.rs
// CEP:WHAT: Byte-level tests for the DWARF 5 debug emitter and the ELF hot/cold split pass:
//   ULEB128/SLEB128 round-trip properties with exact edge encodings, the fixed .debug_abbrev
//   table golden, the .debug_info DIE layout with strp resolution into .debug_str, the
//   .debug_line header and state-machine opcode stream (end_sequence presence, empty-table
//   validity), DWARF determinism and negative tests, and hot-before-cold ELF section
//   ordering (with all-hot/all-cold no-op cases and emission determinism).
// CEP:WHY: design.md 1701-1745 makes debug emission part of the product and design.md
//   1618-1622 pins the split ordering; both must be byte-exact and deterministic.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: test-only; assertions panic on mismatch by design.
// CEP:ASSUMES: little-endian expectations hand-computed from DWARF 5 numbering; the
//   independent S/LEB128 decoders in this module implement the DWARF grammar.
// CEP:COST: test-only; fixtures are small.
// CEP:EVIDENCE: this module is the evidence; each dwarf/elf-split rule has one test below.

use rv8_object::dwarf::{
    encode_sleb128, encode_uleb128, write_dwarf64, DwarfUnit, DW_UT_COMPILE, DWARF_ADDRESS_SIZE,
    DWARF_VERSION, DW_ATE_FLOAT, DW_ATE_SIGNED, DW_LANG_RUST,
};
use rv8_object::elf::{write_elf64_relocatable, SHDR_BYTES, SHT_PROGBITS};
use rv8_object::model::{LineTable, ObjectFile, PrimitiveType, RelocKind, MAX_NAME_BYTES};
use rv8_object::ObjectError;

use crate::{rd_u16, rd_u32, rd_u64, U32_BYTES, ULEB_MAX_BYTES};

/// Independent ULEB128 decoder for round-trip checks; returns (value, bytes consumed).
fn decode_uleb(bytes: &[u8]) -> (u64, usize) {
    let mut value: u64 = 0x0;
    let mut shift: u32 = 0x0;
    let mut used: usize = 0x0;
    for &byte in bytes {
        value |= ((byte & 0x7F) as u64) << shift;
        shift += 0x7;
        used += 0x1;
        if byte & 0x80 == 0x0 {
            break;
        }
    }
    (value, used)
}

/// Independent SLEB128 decoder for round-trip checks; returns (value, bytes consumed).
fn decode_sleb(bytes: &[u8]) -> (i64, usize) {
    let mut value: i64 = 0x0;
    let mut shift: u32 = 0x0;
    let mut used: usize = 0x0;
    for &byte in bytes {
        value |= ((byte & 0x7F) as i64) << shift;
        shift += 0x7;
        used += 0x1;
        if byte & 0x80 == 0x0 {
            if shift < 0x40 && byte & 0x40 != 0x0 {
                value |= -0x1i64 << shift;
            }
            break;
        }
    }
    (value, used)
}

/// Asserts that `off` in `.debug_str` holds exactly `s` followed by a NUL terminator.
fn assert_strp(debug_str: &[u8], off: u32, s: &str) {
    let at = off as usize;
    assert_eq!(&debug_str[at..at + s.len()], s.as_bytes());
    assert_eq!(debug_str[at + s.len()], 0x0);
}

/// Resolves the emitted names of the ELF input-section slots by walking the section header
/// table and .shstrtab (slot 0 is the null section).
fn elf_input_section_names(img: &[u8], input_count: usize) -> Vec<String> {
    let shoff = rd_u64(img, 0x28) as usize;
    let shstrndx = rd_u16(img, 0x3E) as usize;
    let str_hdr = &img[shoff + shstrndx * SHDR_BYTES..shoff + (shstrndx + 0x1) * SHDR_BYTES];
    let str_at = rd_u64(str_hdr, 0x18) as usize;
    let str_len = rd_u64(str_hdr, 0x20) as usize;
    let shstr = &img[str_at..str_at + str_len];
    (0x1..=input_count)
        .map(|i| {
            let rec = &img[shoff + i * SHDR_BYTES..shoff + (i + 0x1) * SHDR_BYTES];
            let off = rd_u32(rec, 0x0) as usize;
            let end = shstr[off..].iter().position(|b| *b == 0x0).unwrap() + off;
            String::from_utf8_lossy(&shstr[off..end]).into_owned()
        })
        .collect()
}

/// CEP:WHAT: ULEB128 round-trip property: every sampled value (including 0x7F/0x80
///   boundaries, 32-bit and 64-bit extremes) encodes to at most 10 bytes, decodes back
///   exactly, and consumes the whole encoding; the canonical boundary byte shapes are
///   pinned exactly.
/// CEP:WHY: ULEB128 is the substrate of every DWARF table; its encoding is normative.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: decode_uleb implements the DWARF ULEB128 grammar.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_uleb128_round_trip_property() {
    let cases = [
        0x0,
        0x1,
        0x7F,
        0x80,
        0x3FFF,
        0x4000,
        0x1FFF_FFFF,
        0xFFFF_FFFF,
        0x7FFF_FFFF_FFFF_FFFF,
        u64::MAX,
        0x1234_5678_9ABC_DEF0,
    ];
    for v in cases {
        let enc = encode_uleb128(v);
        assert!(enc.len() <= ULEB_MAX_BYTES);
        let (dec, used) = decode_uleb(&enc);
        assert_eq!(used, enc.len());
        assert_eq!(dec, v);
    }
    assert_eq!(encode_uleb128(0x0), vec![0x00]);
    assert_eq!(encode_uleb128(0x7F), vec![0x7F]);
    assert_eq!(encode_uleb128(0x80), vec![0x80, 0x01]);
    let mut max = vec![0xFF; 0x9];
    max.push(0x01);
    assert_eq!(encode_uleb128(u64::MAX), max);
}

/// CEP:WHAT: SLEB128 round-trip property: signed boundary values (-1, 63/64, -64/-65,
///   i64::MIN, i64::MAX) encode to at most 10 bytes and decode back exactly, and the
///   canonical boundary byte shapes are pinned exactly.
/// CEP:WHY: DW_LNS_advance_line operands are SLEB128; sign extension must be exact.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: decode_sleb implements the DWARF SLEB128 grammar.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_sleb128_round_trip_property() {
    let cases = [
        0x0,
        0x1,
        -0x1,
        0x3F,
        0x40,
        -0x40,
        -0x41,
        0x1FFF,
        -0x2000,
        i64::MAX,
        i64::MIN,
        -0x12D687,
    ];
    for v in cases {
        let enc = encode_sleb128(v);
        assert!(enc.len() <= ULEB_MAX_BYTES);
        let (dec, used) = decode_sleb(&enc);
        assert_eq!(used, enc.len());
        assert_eq!(dec, v);
    }
    assert_eq!(encode_sleb128(-0x1), vec![0x7F]);
    assert_eq!(encode_sleb128(0x3F), vec![0x3F]);
    assert_eq!(encode_sleb128(0x40), vec![0xC0, 0x00]);
    assert_eq!(encode_sleb128(-0x40), vec![0x40]);
    assert_eq!(encode_sleb128(-0x41), vec![0xBF, 0x7F]);
    let mut min = vec![0x80; 0x9];
    min.push(0x7F);
    assert_eq!(encode_sleb128(i64::MIN), min);
}

/// CEP:WHAT: The fixed .debug_abbrev table is byte-exact: abbreviation 1 (compile_unit,
///   has_children: name/strp, language/data2, producer/strp, comp_dir/strp, low_pc/addr,
///   high_pc/data8, stmt_list/sec_offset), 2 (subprogram, leaf: name/low_pc/high_pc),
///   3 (base_type, leaf: name/byte_size/encoding), then the table terminator.
/// CEP:WHY: .debug_info offsets are a pure function of this static table.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: DWARF 5 tag/attr/form numbering.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_abbrev_table_is_fixed_golden() {
    let mut o = ObjectFile::new("a.o");
    let t = o.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x1).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[],
        line_table: None,
    };
    let secs = write_dwarf64(&o, &unit).unwrap();
    let expected: [u8; 0x2A] = [
        0x01, 0x11, 0x01, 0x03, 0x0E, 0x13, 0x05, 0x25, 0x0E, 0x1B, 0x0E, 0x11, 0x01, 0x12,
        0x07, 0x10, 0x17, 0x00, 0x00, 0x02, 0x2E, 0x00, 0x03, 0x0E, 0x11, 0x01, 0x12, 0x07,
        0x00, 0x00, 0x03, 0x24, 0x00, 0x03, 0x0E, 0x0B, 0x0B, 0x3E, 0x0B, 0x00, 0x00, 0x00,
    ];
    assert_eq!(secs.debug_abbrev, expected);
}

/// CEP:WHAT: The .debug_info layout: DWARF 5 header (unit_length excluding its own field,
///   version, DW_UT_compile, address_size, abbrev_offset 0), the CU DIE with strp-resolved
///   name/producer/comp_dir and low_pc/high_pc spanning the subprograms, one subprogram
///   DIE per defined executable symbol in model order, and deduplicated base-type DIEs.
/// CEP:WHY: The DIE tree is the debugger's primary view; offsets must be exact.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: subprograms "f" (0x0..0x1) and "g" (0x1..0x2) make low_pc 0x0 and
///   high_pc (offset form) 0x2; base types dedup to I32 then F64.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_info_layout_with_strp_resolution() {
    let mut o = ObjectFile::new("dwarf.o");
    let t = o.add_text_section(".text", 0x10, vec![0x90; 0x4]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x1).unwrap();
    o.add_symbol("g", true, false, Some(t), 0x1, 0x1).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[PrimitiveType::I32, PrimitiveType::F64, PrimitiveType::I32],
        line_table: None,
    };
    let secs = write_dwarf64(&o, &unit).unwrap();
    let info = &secs.debug_info;
    assert_eq!(rd_u32(info, 0x0) as usize, info.len() - U32_BYTES);
    assert_eq!(rd_u16(info, 0x4), DWARF_VERSION);
    assert_eq!(info[0x6], DW_UT_COMPILE);
    assert_eq!(info[0x7], DWARF_ADDRESS_SIZE);
    assert_eq!(rd_u32(info, 0x8), 0x0); // debug_abbrev_offset
    assert_eq!(info[0xC], 0x1); // CU abbrev code
    assert_strp(&secs.debug_str, rd_u32(info, 0xD), "u.rs");
    assert_eq!(rd_u16(info, 0x11), DW_LANG_RUST as u16);
    assert_strp(&secs.debug_str, rd_u32(info, 0x13), "rv8-test");
    assert_strp(&secs.debug_str, rd_u32(info, 0x17), "/tmp");
    assert_eq!(rd_u64(info, 0x1B), 0x0); // DW_AT_low_pc
    assert_eq!(rd_u64(info, 0x23), 0x2); // DW_AT_high_pc (offset form)
    assert_eq!(rd_u32(info, 0x2B), 0x0); // DW_AT_stmt_list
    // Subprogram DIEs in model order (abbrev code 2 is the subprogram abbreviation).
    let mut at = 0x2F;
    for (name, low, size) in [("f", 0x0u64, 0x1u64), ("g", 0x1, 0x1)] {
        assert_eq!(info[at], 0x2);
        assert_strp(&secs.debug_str, rd_u32(info, at + 0x1), name);
        assert_eq!(rd_u64(info, at + 0x5), low);
        assert_eq!(rd_u64(info, at + 0xD), size);
        at += 0x15;
    }
    // Base-type DIEs (abbrev code 3): name, byte_size, ATE encoding.
    for (name, size, encoding) in [("i32", 0x4u8, DW_ATE_SIGNED), ("f64", 0x8, DW_ATE_FLOAT)] {
        assert_eq!(info[at], 0x3);
        assert_strp(&secs.debug_str, rd_u32(info, at + 0x1), name);
        assert_eq!(info[at + 0x5], size);
        assert_eq!(info[at + 0x6], encoding as u8);
        at += 0x7;
    }
    assert_eq!(info[at], 0x0); // end of CU children
    assert_eq!(at + 0x1, info.len());
}

/// CEP:WHAT: The .debug_line layout for a two-row table: version-5 header fields
///   (unit_length excluding its own field, header_length, min_inst_length 1,
///   max_ops_per_inst 1, default_is_stmt 1, line_base -5, line_range 14, opcode_base 13,
///   standard_opcode_lengths), one directory entry (comp_dir) and one file entry, then the
///   exact state-machine opcode stream (advance_pc/advance_line/set_column/copy) closed by
///   DW_LNE_end_sequence.
/// CEP:WHY: The line program is the address-to-source mapping; its opcodes are normative.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
/// CEP:ASSUMES: initial state address 0, file 0, line 1, column 0, is_stmt true.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_line_header_and_state_machine() {
    let mut o = ObjectFile::new("line.o");
    let t = o.add_text_section(".text", 0x1, vec![0x90; 0x20]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x20).unwrap();
    let mut table = LineTable::new("u.rs").unwrap();
    table.add_row(0x10, 0x0, 0x5, 0x0, true, false).unwrap();
    table.add_row(0x20, 0x0, 0x8, 0x2, true, false).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[],
        line_table: Some(&table),
    };
    let secs = write_dwarf64(&o, &unit).unwrap();
    let line = &secs.debug_line;
    assert_eq!(rd_u32(line, 0x0) as usize, line.len() - U32_BYTES);
    assert_eq!(rd_u16(line, 0x4), DWARF_VERSION);
    assert_eq!(line[0x6], DWARF_ADDRESS_SIZE);
    assert_eq!(line[0x7], 0x0); // segment_selector_size
    assert_eq!(rd_u32(line, 0x8), 0x24); // header_length
    assert_eq!(line[0xC], 0x1); // minimum_instruction_length
    assert_eq!(line[0xD], 0x1); // maximum_operations_per_instruction
    assert_eq!(line[0xE], 0x1); // default_is_stmt
    assert_eq!(line[0xF] as i8, -0x5); // line_base
    assert_eq!(line[0x10], 0xE); // line_range
    assert_eq!(line[0x11], 0xD); // opcode_base
    assert_eq!(
        &line[0x12..0x1E],
        &[0x0, 0x1, 0x1, 0x1, 0x1, 0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x1]
    );
    // Directory entry format (1 pair), directories_count 1, "/tmp\0".
    assert_eq!(line[0x1E], 0x1);
    assert_eq!(line[0x1F], 0x1);
    assert_eq!(line[0x20], 0x8);
    assert_eq!(line[0x21], 0x1);
    assert_eq!(&line[0x22..0x27], b"/tmp\0");
    // File entry format (1 pair), file_names_count 1, "u.rs\0".
    assert_eq!(line[0x27], 0x1);
    assert_eq!(line[0x28], 0x1);
    assert_eq!(line[0x29], 0x8);
    assert_eq!(line[0x2A], 0x1);
    assert_eq!(&line[0x2B..0x30], b"u.rs\0");
    // Program: advance_pc 0x10, advance_line +4, copy; advance_pc 0x10, advance_line +3,
    // set_column 2, copy; DW_LNE_end_sequence.
    let program = &line[0x30..];
    assert_eq!(
        program,
        &[
            0x02, 0x10, 0x03, 0x04, 0x01, 0x02, 0x10, 0x03, 0x03, 0x05, 0x02, 0x01, 0x00,
            0x01, 0x01,
        ]
    );
    assert_eq!(line.len(), 0x3F);
}

/// CEP:WHAT: A LineTable-less unit still emits a structurally valid .debug_line: the file
///   entry count is 0 and the program is exactly DW_LNE_end_sequence, so the (empty)
///   sequence is terminated.
/// CEP:WHY: design.md requires a valid line program even without line data.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_line_empty_table_still_terminates() {
    let mut o = ObjectFile::new("noline.o");
    let t = o.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x1).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[],
        line_table: None,
    };
    let secs = write_dwarf64(&o, &unit).unwrap();
    let line = &secs.debug_line;
    let header_length = rd_u32(line, 0x8) as usize;
    let program = &line[0xC + header_length..];
    assert_eq!(program, &[0x00, 0x01, 0x01]);
    // file_names_count is 0 when no table is supplied.
    assert_eq!(line[0x2A], 0x0);
}

/// CEP:WHAT: Two write_dwarf64 emissions of the same object plus unit are byte-identical
///   across all four sections, including a populated line table.
/// CEP:WHY: Debug output must meet the same determinism bar as the object writers.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_emission_is_deterministic() {
    let mut o = ObjectFile::new("det.o");
    let t = o.add_text_section(".text", 0x10, vec![0x90; 0x10]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x8).unwrap();
    o.add_symbol("g", true, false, Some(t), 0x8, 0x8).unwrap();
    let mut table = LineTable::new("u.rs").unwrap();
    table.add_row(0x0, 0x0, 0x3, 0x0, true, false).unwrap();
    table.add_row(0x8, 0x0, 0x4, 0x1, true, false).unwrap();
    table.add_row(0x10, 0x0, 0x1, 0x0, true, true).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[PrimitiveType::U64, PrimitiveType::Bool, PrimitiveType::U64],
        line_table: Some(&table),
    };
    let a = write_dwarf64(&o, &unit).unwrap();
    let b = write_dwarf64(&o, &unit).unwrap();
    assert_eq!(a.debug_abbrev, b.debug_abbrev);
    assert_eq!(a.debug_info, b.debug_info);
    assert_eq!(a.debug_line, b.debug_line);
    assert_eq!(a.debug_str, b.debug_str);
}

/// CEP:WHAT: DWARF negative tests: a NUL-bearing CU name, an oversize producer, and a
///   subprogram whose value+size overflows u64 each return the named ObjectError.
/// CEP:WHY: Fail-closed construction extends to debug emission.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert/matches on mismatch (test-only).
/// CEP:ASSUMES: names are checked before any byte is emitted.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn dwarf_rejects_invalid_unit_and_symbol_data() {
    let mut o = ObjectFile::new("bad.o");
    let t = o.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    o.add_symbol("f", true, true, Some(t), 0x0, 0x1).unwrap();
    let nul = DwarfUnit {
        cu_name: "bad\0name",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[],
        line_table: None,
    };
    assert!(matches!(
        write_dwarf64(&o, &nul),
        Err(ObjectError::InvalidName { .. })
    ));
    let long_producer = "p".repeat(MAX_NAME_BYTES + 0x1);
    let oversize = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: &long_producer,
        base_types: &[],
        line_table: None,
    };
    assert!(matches!(
        write_dwarf64(&o, &oversize),
        Err(ObjectError::InvalidName { .. })
    ));
    let mut big = ObjectFile::new("big.o");
    let tb = big.add_text_section(".text", 0x1, vec![0x90]).unwrap();
    big.add_symbol("f", true, true, Some(tb), u64::MAX, 0x1).unwrap();
    let unit = DwarfUnit {
        cu_name: "u.rs",
        comp_dir: "/tmp",
        producer: "rv8-test",
        base_types: &[],
        line_table: None,
    };
    assert!(matches!(
        write_dwarf64(&big, &unit),
        Err(ObjectError::ArithmeticOverflow { context: "dwarf subprogram high_pc" })
    ));
}

/// CEP:WHAT: The hot/cold split places hot executable sections before cold ones even when
///   the cold section has the strictly larger alignment (which plain layout_order would
///   put first), and the executable slots stay SHT_PROGBITS.
/// CEP:WHY: design.md 1618-1622: hot code leads .text, cold error paths trail it.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: cold sections are named with the ".cold" suffix.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn elf_split_orders_hot_before_cold() {
    let mut o = ObjectFile::new("hc.o");
    o.add_text_section(".text.cold", 0x40, vec![0x90]).unwrap();
    o.add_text_section(".text", 0x10, vec![0x90]).unwrap();
    o.add_text_section(".text.helper", 0x8, vec![0x90]).unwrap();
    let img = write_elf64_relocatable(&o).unwrap();
    assert_eq!(
        elf_input_section_names(&img, 0x3),
        vec![".text", ".text.helper", ".text.cold"]
    );
    let shoff = rd_u64(&img, 0x28) as usize;
    for i in 0x1..=0x3 {
        let rec = &img[shoff + i * SHDR_BYTES..shoff + (i + 0x1) * SHDR_BYTES];
        assert_eq!(rd_u32(rec, 0x4), SHT_PROGBITS);
    }
}

/// CEP:WHAT: Without a ".cold" section the split is a no-op: the emitted executable order
///   is exactly the layout_order rule (descending alignment).
/// CEP:WHY: The pass must not reorder anything when there is nothing to split.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: all sections hot means no name ends in ".cold".
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn elf_split_is_noop_without_cold_sections() {
    let mut o = ObjectFile::new("hot.o");
    o.add_text_section(".text", 0x10, vec![0x90]).unwrap();
    o.add_text_section(".text.zzz", 0x40, vec![0x90]).unwrap();
    o.add_data_section(".data", 0x8, vec![0x0; 0x8]).unwrap();
    let img = write_elf64_relocatable(&o).unwrap();
    assert_eq!(
        elf_input_section_names(&img, 0x3),
        vec![".text.zzz", ".text", ".data"]
    );
}

/// CEP:WHAT: When every executable section is cold the stable partition keeps the
///   layout_order sequence intact (no reordering among cold sections).
/// CEP:WHY: The partition is stable on both sides, so equal classes never swap.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: both sections carry the ".cold" suffix.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn elf_split_preserves_order_when_all_cold() {
    let mut o = ObjectFile::new("cold.o");
    o.add_text_section(".text.a.cold", 0x20, vec![0x90]).unwrap();
    o.add_text_section(".text.b.cold", 0x10, vec![0x90]).unwrap();
    let img = write_elf64_relocatable(&o).unwrap();
    assert_eq!(
        elf_input_section_names(&img, 0x2),
        vec![".text.a.cold", ".text.b.cold"]
    );
}

/// CEP:WHAT: Two ELF emissions of a mixed hot/cold object (with data and NOBITS sections)
///   are byte-identical, and the hot section still leads the executable group.
/// CEP:WHY: The split feeds the deterministic writer; the pair must stay deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn elf_split_emission_is_deterministic() {
    let mut o = ObjectFile::new("det.o");
    o.add_text_section(".text.cold", 0x40, vec![0x90]).unwrap();
    o.add_text_section(".text", 0x10, vec![0x90; 0x8]).unwrap();
    o.add_data_section(".data", 0x8, vec![0x7; 0x8]).unwrap();
    o.add_nobits_section(".bss", 0x8, 0x40).unwrap();
    o.add_symbol("f", true, true, Some(0x1), 0x0, 0x1).unwrap();
    o.add_reloc(0x1, 0x0, RelocKind::Abs64, "f", 0x0).unwrap();
    let a = write_elf64_relocatable(&o).unwrap();
    let b = write_elf64_relocatable(&o).unwrap();
    assert_eq!(a, b);
    assert_eq!(
        elf_input_section_names(&a, 0x2),
        vec![".text", ".text.cold"]
    );
}
