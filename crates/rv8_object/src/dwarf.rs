// CEP:FILE: crates/rv8_object/src/dwarf.rs
// CEP:WHAT: DWARF 5 debug-info emission for the universal object model: a fixed
//   .debug_abbrev table (compile_unit, subprogram, base_type), a .debug_info compilation
//   unit (name/language/producer/comp_dir/low_pc/high_pc plus one subprogram DIE per
//   defined function symbol and one base-type DIE per deduplicated PrimitiveType), a
//   suffix-shared .debug_str, and the .debug_line line-number program from the model
//   LineTable (see the line submodule).
// CEP:WHY: design.md 1701-1745 makes debuggability part of emission; every DIE offset and
//   row is a pure function of the model so debug output meets the same byte-determinism
//   bar as the object writers.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: model validation and name checks run first; oversize or NUL-bearing names,
//   DWARF32 unit_length overflow, and symbol value/size sums that overflow u64 return
//   ObjectError; the library never panics.
// CEP:ASSUMES: DWARF 5 for x86-64 Linux (address size 8); debug addresses are
//   section-relative (relocatable objects carry no debug relocations in this revision);
//   subprograms are derived from defined symbols anchored in executable sections with
//   non-empty names; base types come from the caller via model PrimitiveType values.
// CEP:COST: O(Y log Y) for the string table adds (bounded by MAX_STRING_TABLE_BYTES) plus
//   O(subprograms + base types + rows); one buffer per emitted section.
// CEP:EVIDENCE: unit tests in crates/rv8_object/tests/serializers.rs (info/abbrev layout,
//   base types, line header and state machine, empty-table validity, determinism);
//   acceptance determinism test in tests/acceptance.rs.

mod line;

use crate::elf::StringTable;
use crate::error::ObjectError;
use crate::model::{check_name, ObjectFile, PrimitiveType};

pub(crate) use line::{emit_line_section, sleb128, uleb128};

// --- DWARF constants (named; DWARF 5 numbering) ---

pub const DW_TAG_COMPILE_UNIT: u64 = 0x11;
pub const DW_TAG_SUBPROGRAM: u64 = 0x2E;
pub const DW_TAG_BASE_TYPE: u64 = 0x24;
pub const DW_AT_NAME: u64 = 0x03;
pub const DW_AT_BYTE_SIZE: u64 = 0x0B;
pub const DW_AT_STMT_LIST: u64 = 0x10;
pub const DW_AT_LOW_PC: u64 = 0x11;
pub const DW_AT_HIGH_PC: u64 = 0x12;
pub const DW_AT_LANGUAGE: u64 = 0x13;
pub const DW_AT_COMP_DIR: u64 = 0x1B;
pub const DW_AT_PRODUCER: u64 = 0x25;
pub const DW_AT_ENCODING: u64 = 0x3E;
pub const DW_FORM_ADDR: u64 = 0x01;
pub const DW_FORM_DATA2: u64 = 0x05;
pub const DW_FORM_DATA8: u64 = 0x07;
pub const DW_FORM_DATA1: u64 = 0x0B;
pub const DW_FORM_STRP: u64 = 0x0E;
pub const DW_FORM_SEC_OFFSET: u64 = 0x17;
/// CEP:WHAT: DW_UT_compile unit type for the version-5 .debug_info header.
/// CEP:WHY: Version 5 headers carry the unit type explicitly.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: info-layout test.
pub const DW_UT_COMPILE: u8 = 0x01;
/// CEP:WHAT: DW_LANG_Rust language code.
/// CEP:WHY: The compiler's source language; fixed value keeps output deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering (0x001c).
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: info-layout test.
pub const DW_LANG_RUST: u64 = 0x001C;
/// CEP:WHAT: DW_ATE_boolean encoding for bool base types.
/// CEP:WHY: PrimitiveType mapping table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: base-type test.
pub const DW_ATE_BOOLEAN: u64 = 0x02;
/// CEP:WHAT: DW_ATE_float encoding for floating base types.
/// CEP:WHY: PrimitiveType mapping table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: base-type test.
pub const DW_ATE_FLOAT: u64 = 0x04;
/// CEP:WHAT: DW_ATE_signed encoding for signed base types.
/// CEP:WHY: PrimitiveType mapping table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: base-type test.
pub const DW_ATE_SIGNED: u64 = 0x05;
/// CEP:WHAT: DW_ATE_unsigned encoding for unsigned base types.
/// CEP:WHY: PrimitiveType mapping table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: base-type test.
pub const DW_ATE_UNSIGNED: u64 = 0x07;
/// CEP:WHAT: Emitted DWARF version for .debug_info and .debug_line.
/// CEP:WHY: The stub contract pins DWARF 5.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: consumers understand version 5.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: info and line layout tests.
pub const DWARF_VERSION: u16 = 5;
/// CEP:WHAT: Target address size in bytes.
/// CEP:WHY: x86-64 addresses are 8 bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: x86-64 Linux consumer.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: info and line layout tests.
pub const DWARF_ADDRESS_SIZE: u8 = 8;
/// CEP:WHAT: Largest section length encodable in the DWARF32 u32 length fields.
/// CEP:WHY: Named budget; DWARF64 is deliberately not emitted.
/// CEP:STATUS: complete
/// CEP:FAILURE: ArithmeticOverflow when exceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: checked in write_dwarf64.
pub const DWARF_MAX_SECTION_BYTES: u64 = 0xFFFF_FFFF;

/// CEP:WHAT: The inputs of one DWARF compilation unit: names, the primitive types to
///   describe, and the optional source line table.
/// CEP:WHY: The universal object model pins ObjectFile's shape, so compilation-unit
///   metadata travels beside it in this writer-owned record.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validation happens in write_dwarf64).
/// CEP:ASSUMES: names are valid Rust strings without NUL (checked at emission).
/// CEP:COST: borrowed references; no allocation.
/// CEP:EVIDENCE: exercised by every dwarf test in tests/serializers.rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DwarfUnit<'a> {
    /// Source file name of the compilation unit (DW_AT_name).
    pub cu_name: &'a str,
    /// Compilation directory (DW_AT_comp_dir and the line-header directory entry).
    pub comp_dir: &'a str,
    /// Producer string (DW_AT_producer).
    pub producer: &'a str,
    /// Primitive types to emit as DW_TAG_base_type DIEs (deduplicated stably).
    pub base_types: &'a [PrimitiveType],
    /// Source line table; None emits an empty-but-valid line program.
    pub line_table: Option<&'a crate::model::LineTable>,
}

/// CEP:WHAT: The four emitted DWARF sections, ready to be attached to an object.
/// CEP:WHY: Callers map these onto sections (.debug_abbrev/.debug_info/.debug_line/
///   .debug_str) without the writer depending on a container format.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (plain data).
/// CEP:ASSUMES: byte vectors are complete, self-contained sections.
/// CEP:COST: owned buffers.
/// CEP:EVIDENCE: exercised by every dwarf test in tests/serializers.rs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DwarfSections {
    /// Abbreviation table.
    pub debug_abbrev: Vec<u8>,
    /// Debug information (one compilation unit).
    pub debug_info: Vec<u8>,
    /// Line-number program.
    pub debug_line: Vec<u8>,
    /// String table for strp form values.
    pub debug_str: Vec<u8>,
}

/// CEP:WHAT: Builds the fixed .debug_abbrev table: abbreviation 1 (compile_unit with
///   name/language/producer/comp_dir/low_pc/high_pc/stmt_list, has_children), 2
///   (subprogram with name/low_pc/high_pc, leaf), 3 (base_type with name/byte_size/
///   encoding, leaf), then the table terminator.
/// CEP:WHY: The abbreviation layout is static, so the table is a constant byte sequence
///   and .debug_info offsets never depend on dynamic forms.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (total function).
/// CEP:ASSUMES: DWARF 5 form numbering.
/// CEP:COST: O(1) bytes.
/// CEP:EVIDENCE: abbrev test in tests/serializers.rs.
fn build_abbrev_table() -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    // Abbrev 1: compile_unit.
    uleb128(1, &mut out);
    uleb128(DW_TAG_COMPILE_UNIT, &mut out);
    out.push(1); // DW_CHILDREN_yes
    for (attr, form) in [
        (DW_AT_NAME, DW_FORM_STRP),
        (DW_AT_LANGUAGE, DW_FORM_DATA2),
        (DW_AT_PRODUCER, DW_FORM_STRP),
        (DW_AT_COMP_DIR, DW_FORM_STRP),
        (DW_AT_LOW_PC, DW_FORM_ADDR),
        (DW_AT_HIGH_PC, DW_FORM_DATA8),
        (DW_AT_STMT_LIST, DW_FORM_SEC_OFFSET),
    ] {
        uleb128(attr, &mut out);
        uleb128(form, &mut out);
    }
    out.push(0);
    out.push(0);
    // Abbrev 2: subprogram.
    uleb128(2, &mut out);
    uleb128(DW_TAG_SUBPROGRAM, &mut out);
    out.push(0); // DW_CHILDREN_no
    for (attr, form) in [
        (DW_AT_NAME, DW_FORM_STRP),
        (DW_AT_LOW_PC, DW_FORM_ADDR),
        (DW_AT_HIGH_PC, DW_FORM_DATA8),
    ] {
        uleb128(attr, &mut out);
        uleb128(form, &mut out);
    }
    out.push(0);
    out.push(0);
    // Abbrev 3: base_type.
    uleb128(3, &mut out);
    uleb128(DW_TAG_BASE_TYPE, &mut out);
    out.push(0); // DW_CHILDREN_no
    for (attr, form) in [
        (DW_AT_NAME, DW_FORM_STRP),
        (DW_AT_BYTE_SIZE, DW_FORM_DATA1),
        (DW_AT_ENCODING, DW_FORM_DATA1),
    ] {
        uleb128(attr, &mut out);
        uleb128(form, &mut out);
    }
    out.push(0);
    out.push(0);
    out.push(0); // end of abbrev table
    out
}

/// CEP:WHAT: DWARF ATE encoding for a model primitive type.
/// CEP:WHY: The model carries the scalar vocabulary; the DWARF numbering lives here.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (exhaustive match).
/// CEP:ASSUMES: PrimitiveType stays append-only with new variants added here.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: base-type test.
fn ate_encoding(ty: PrimitiveType) -> u64 {
    match ty {
        PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32 | PrimitiveType::I64 => {
            DW_ATE_SIGNED
        }
        PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 | PrimitiveType::U64 => {
            DW_ATE_UNSIGNED
        }
        PrimitiveType::F32 | PrimitiveType::F64 => DW_ATE_FLOAT,
        PrimitiveType::Bool => DW_ATE_BOOLEAN,
    }
}

/// CEP:WHAT: Emits the DWARF 5 debug sections for one object: .debug_abbrev (fixed),
///   .debug_info (compile unit plus one subprogram DIE per defined symbol in an
///   executable section with a non-empty name, in model order, plus one base-type DIE per
///   deduplicated PrimitiveType), .debug_str (suffix-shared), and .debug_line from the
///   optional LineTable (an empty-but-valid program when absent).
/// CEP:WHY: The debug-info product of the emission pipeline; determinism is the bar.
/// CEP:STATUS: complete
/// CEP:FAILURE: validate() rejections, InvalidName (oversize or NUL-bearing unit names),
///   ArithmeticOverflow (DWARF32 unit_length overflow, u64 address sums,
///   MAX_STRING_TABLE_BYTES), WriteOutOfBounds; never panics.
/// CEP:ASSUMES: see CEP:FILE header assumptions.
/// CEP:COST: O(Y log Y + subprograms + base types + rows).
/// CEP:EVIDENCE: unit tests in tests/serializers.rs; determinism in tests/acceptance.rs.
pub fn write_dwarf64(obj: &ObjectFile, unit: &DwarfUnit) -> Result<DwarfSections, ObjectError> {
    obj.validate()?;
    check_name(unit.cu_name)?;
    check_name(unit.comp_dir)?;
    check_name(unit.producer)?;

    // Subprograms: defined symbols anchored in executable sections, model order,
    // anonymous symbols skipped (a DIE needs a name).
    let mut subprograms: Vec<(&str, u64, u64)> = Vec::new();
    for sym in &obj.symbols {
        if !(sym.defined && !sym.name.is_empty()) {
            continue;
        }
        let si = match sym.section {
            Some(si) if si < obj.sections.len() => si,
            _ => continue,
        };
        if !obj.sections[si].is_exec() {
            continue;
        }
        let high = sym
            .value
            .checked_add(sym.size)
            .ok_or(ObjectError::ArithmeticOverflow { context: "dwarf subprogram high_pc" })?;
        subprograms.push((sym.name.as_str(), sym.value, high));
    }
    let cu_low = subprograms.iter().map(|&(_, low, _)| low).min().unwrap_or(0);
    let cu_high_end = subprograms.iter().map(|&(_, _, high)| high).max().unwrap_or(0);
    let cu_high_offset = cu_high_end
        .checked_sub(cu_low)
        .ok_or(ObjectError::ArithmeticOverflow { context: "dwarf cu high_pc" })?;

    // Base types: stable dedup by variant, first occurrence order.
    let mut base_types: Vec<PrimitiveType> = Vec::new();
    for ty in unit.base_types {
        if !base_types.contains(ty) {
            base_types.push(*ty);
        }
    }

    // .debug_str: producer, comp_dir, CU name, subprogram names, base-type names.
    let mut strtab = StringTable::new();
    let producer_off = strtab.add(unit.producer)?;
    let comp_dir_off = strtab.add(unit.comp_dir)?;
    let cu_name_off = strtab.add(unit.cu_name)?;
    let mut sub_name_off: Vec<u32> = Vec::with_capacity(subprograms.len());
    for (name, _, _) in &subprograms {
        sub_name_off.push(strtab.add(name)?);
    }
    let mut ty_name_off: Vec<u32> = Vec::with_capacity(base_types.len());
    for ty in &base_types {
        ty_name_off.push(strtab.add(ty.name())?);
    }

    let debug_abbrev = build_abbrev_table();

    // .debug_info: header + DIE tree.
    let mut info: Vec<u8> = Vec::new();
    info.extend_from_slice(&0u32.to_le_bytes()); // unit_length, back-patched below
    info.extend_from_slice(&DWARF_VERSION.to_le_bytes());
    info.push(DW_UT_COMPILE);
    info.push(DWARF_ADDRESS_SIZE);
    info.extend_from_slice(&0u32.to_le_bytes()); // debug_abbrev_offset
    // CU DIE.
    uleb128(1, &mut info);
    info.extend_from_slice(&cu_name_off.to_le_bytes());
    info.extend_from_slice(&(DW_LANG_RUST as u16).to_le_bytes());
    info.extend_from_slice(&producer_off.to_le_bytes());
    info.extend_from_slice(&comp_dir_off.to_le_bytes());
    info.extend_from_slice(&cu_low.to_le_bytes());
    info.extend_from_slice(&cu_high_offset.to_le_bytes());
    info.extend_from_slice(&0u32.to_le_bytes()); // DW_AT_stmt_list -> offset 0
    for (pos, &(_, low, high)) in subprograms.iter().enumerate() {
        uleb128(2, &mut info);
        info.extend_from_slice(&sub_name_off[pos].to_le_bytes());
        info.extend_from_slice(&low.to_le_bytes());
        info.extend_from_slice(&(high - low).to_le_bytes());
    }
    for (pos, ty) in base_types.iter().enumerate() {
        uleb128(3, &mut info);
        info.extend_from_slice(&ty_name_off[pos].to_le_bytes());
        info.push(ty.byte_size() as u8);
        info.push(ate_encoding(*ty) as u8);
    }
    info.push(0); // end of CU children
    let unit_length = u32::try_from(info.len() - 4)
        .map_err(|_| ObjectError::ArithmeticOverflow { context: "dwarf info unit_length" })?;
    if info.len() as u64 > DWARF_MAX_SECTION_BYTES {
        return Err(ObjectError::ArithmeticOverflow {
            context: "dwarf info section size",
        });
    }
    crate::elf::write_bytes_at(&mut info, 0, &unit_length.to_le_bytes())?;

    let debug_line = emit_line_section(unit.line_table, unit.comp_dir)?;

    Ok(DwarfSections {
        debug_abbrev,
        debug_info: info,
        debug_line,
        debug_str: strtab.as_bytes().to_vec(),
    })
}

/// CEP:WHAT: Public ULEB128 encoder: the canonical minimal encoding of `value`.
/// CEP:WHY: The exact primitive the .debug_abbrev/.debug_info/.debug_line encoders use,
///   exposed so integration tests can round-trip the byte format without reimplementing it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (total over u64; at most 10 bytes)
/// CEP:ASSUMES: none
/// CEP:COST: O(ceil(bits/7)) pushes, at most 10.
/// CEP:EVIDENCE: round-trip tests in crates/rv8_object/tests/coff_dwarf_serialization.rs.
pub fn encode_uleb128(value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    uleb128(value, &mut out);
    out
}

/// CEP:WHAT: Public SLEB128 encoder: the canonical minimal encoding of `value`.
/// CEP:WHY: Signed line-program operands (DW_LNS_advance_line) share the format; exposure
///   mirrors encode_uleb128 so tests pin the signed encoding too.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (total over i64; at most 10 bytes)
/// CEP:ASSUMES: none
/// CEP:COST: O(ceil(bits/7)) pushes, at most 10.
/// CEP:EVIDENCE: round-trip tests in crates/rv8_object/tests/coff_dwarf_serialization.rs.
pub fn encode_sleb128(value: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    sleb128(value, &mut out);
    out
}
