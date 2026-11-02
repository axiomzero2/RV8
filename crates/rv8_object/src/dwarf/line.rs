// CEP:FILE: crates/rv8_object/src/dwarf/line.rs
// CEP:WHAT: The DWARF 5 line-number section (.debug_line) emitter: a version-5 header
//   with directory/file-name entry formats, and a real state-machine line program built
//   from model LineTable rows (DW_LNS_advance_pc, DW_LNS_advance_line, DW_LNS_set_column,
//   DW_LNS_negate_stmt, DW_LNS_copy, DW_LNE_end_sequence), plus the ULEB128/SLEB128
//   primitives shared with the abbrev and info encoders.
// CEP:WHY: design.md 1701-1745 requires a real line-number program; the state machine is
//   emitted row by row in stable address order so identical input yields identical bytes.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: unit_length overflow above the DWARF32 u32 field returns ArithmeticOverflow;
//   negative row deltas cannot occur after the stable sort but still map to
//   ArithmeticOverflow instead of wrapping; never panics.
// CEP:ASSUMES: DWARF 5 numbering; min_inst_length 1 and max_ops_per_inst 1 so an
//   operation advance equals an address advance; rows carry file index 0 (validated by
//   LineTable::add_row); a table with no rows still emits one end_sequence so the program
//   is empty but structurally valid.
// CEP:COST: O(rows log rows) stable sort plus O(rows) program bytes; one output buffer.
// CEP:EVIDENCE: line-header and state-machine tests in crates/rv8_object/tests/
//   serializers.rs, including the exact opcode byte sequence for a two-row table and the
//   empty-table program.

use crate::error::ObjectError;
use crate::model::LineTable;

/// CEP:WHAT: DWARF line-table version emitted (DWARF 5).
/// CEP:WHY: The stub contract pins DWARF 5 for x86-64 Linux.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: consumers understand version 5 headers.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const LINE_VERSION: u16 = 5;
/// CEP:WHAT: minimum_instruction_length field.
/// CEP:WHY: 1 makes an operation advance equal an address advance.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: x86-64 variable-length instructions are addressed bytewise.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const MIN_INST_LENGTH: u8 = 1;
/// CEP:WHAT: maximum_operations_per_instruction field.
/// CEP:WHY: 1 keeps the address arithmetic trivial and deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: VLIW is not a target.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const MAX_OPS_PER_INST: u8 = 1;
/// CEP:WHAT: default_is_stmt field.
/// CEP:WHY: Statement-prologue default per DWARF convention.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const DEFAULT_IS_STMT: u8 = 1;
/// CEP:WHAT: line_base field (special-opcode bias).
/// CEP:WHY: Fixed value pins the special-opcode encoding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF convention -5.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const LINE_BASE: i8 = -5;
/// CEP:WHAT: line_range field (special-opcode span).
/// CEP:WHY: Fixed value pins the special-opcode encoding.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF convention 14.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const LINE_RANGE: u8 = 14;
/// CEP:WHAT: opcode_base field (special opcodes start after this).
/// CEP:WHY: 12 standard opcodes means opcode_base is 13.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 standard opcode set.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const OPCODE_BASE: u8 = 13;
/// CEP:WHAT: standard_opcode_lengths table (operand counts of DW_LNS_copy..DW_LNS_set_isa).
/// CEP:WHY: The header must declare them; consumers read the table, the writer never
///   emits special opcodes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 standard opcode operand counts.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const STANDARD_OPCODE_LENGTHS: [u8; 12] = [0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1];
/// CEP:WHAT: DW_LNCT_path content type for directory/file entries.
/// CEP:WHY: Version-5 entry formats declare (content type, form) pairs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const DW_LNCT_PATH: u64 = 1;
/// CEP:WHAT: DW_FORM_string form code used for inline directory/file entries.
/// CEP:WHY: Inline strings avoid a .debug_line_str section while staying valid DWARF 5.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: line-header test.
pub(crate) const DW_FORM_STRING: u64 = 8;
/// CEP:WHAT: Standard line opcodes used by the emitter.
/// CEP:WHY: Named encodings keep the state machine auditable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constants.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_COPY: u8 = 1;
/// CEP:WHAT: DW_LNS_advance_pc opcode.
/// CEP:WHY: See DW_LNS_COPY.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_ADVANCE_PC: u8 = 2;
/// CEP:WHAT: DW_LNS_advance_line opcode.
/// CEP:WHY: See DW_LNS_COPY.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_ADVANCE_LINE: u8 = 3;
/// CEP:WHAT: DW_LNS_set_file opcode.
/// CEP:WHY: See DW_LNS_COPY.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_SET_FILE: u8 = 4;
/// CEP:WHAT: DW_LNS_set_column opcode.
/// CEP:WHY: See DW_LNS_COPY.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_SET_COLUMN: u8 = 5;
/// CEP:WHAT: DW_LNS_negate_stmt opcode.
/// CEP:WHY: See DW_LNS_COPY.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNS_NEGATE_STMT: u8 = 6;
/// CEP:WHAT: Extended-opcode escape byte (0) starting every DW_LNE_* instruction.
/// CEP:WHY: Extended opcodes are 0, ULEB length, sub-opcode.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNE_ESCAPE: u8 = 0;
/// CEP:WHAT: DW_LNE_end_sequence sub-opcode.
/// CEP:WHY: Ends one line-number sequence and appends its row.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: DWARF 5 numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: state-machine test.
pub(crate) const DW_LNE_END_SEQUENCE: u8 = 1;

/// CEP:WHAT: Encodes `value` as ULEB128 into `out` (at most 10 bytes for u64).
/// CEP:WHY: Shared primitive for abbrev, info, and line encoders; shifts are bounded so
///   encoding is total over any u64 and cannot overflow.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(ceil(bits/7)) pushes, at most 10.
/// CEP:EVIDENCE: uleb/sleb encoding test in tests/serializers.rs.
pub(crate) fn uleb128(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// CEP:WHAT: Encodes `value` as SLEB128 into `out` (at most 10 bytes for i64).
/// CEP:WHY: Shared primitive for signed operands (DW_LNS_advance_line); total over i64.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(ceil(bits/7)) pushes, at most 10.
/// CEP:EVIDENCE: uleb/sleb encoding test in tests/serializers.rs.
pub(crate) fn sleb128(mut value: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let sign_bit_set = byte & 0x40 != 0;
        if (value == 0 && !sign_bit_set) || (value == -1 && sign_bit_set) {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// CEP:WHAT: Emits a complete .debug_line section: version-5 header (single directory
///   entry holding the compilation directory, one file entry from the LineTable when
///   present, none otherwise) followed by the state-machine program over the rows sorted
///   stably by address, closed with DW_LNE_end_sequence.
/// CEP:WHY: The design.md line-number program; empty tables still produce a structurally
///   valid program.
/// CEP:STATUS: complete
/// CEP:FAILURE: ArithmeticOverflow when the section would exceed the DWARF32 u32
///   unit_length; never panics.
/// CEP:ASSUMES: rows reference file index 0 (validated at LineTable::add_row).
/// CEP:COST: O(rows log rows + rows).
/// CEP:EVIDENCE: line tests in tests/serializers.rs.
pub(crate) fn emit_line_section(
    table: Option<&LineTable>,
    comp_dir: &str,
) -> Result<Vec<u8>, ObjectError> {
    // The program is built first: header_length spans everything between itself and the
    // program, so the program size must be known before the header is assembled.
    let program = emit_line_program(table)?;

    let mut header_tail: Vec<u8> = vec![
        MIN_INST_LENGTH,
        MAX_OPS_PER_INST,
        DEFAULT_IS_STMT,
        LINE_BASE as u8,
        LINE_RANGE,
        OPCODE_BASE,
    ];
    header_tail.extend_from_slice(&STANDARD_OPCODE_LENGTHS);
    // Directory entry format: one (DW_LNCT_path, DW_FORM_string) pair.
    header_tail.push(1);
    uleb128(DW_LNCT_PATH, &mut header_tail);
    uleb128(DW_FORM_STRING, &mut header_tail);
    uleb128(1, &mut header_tail); // directories_count
    header_tail.extend_from_slice(comp_dir.as_bytes());
    header_tail.push(0);
    // File-name entry format: one (DW_LNCT_path, DW_FORM_string) pair.
    header_tail.push(1);
    uleb128(DW_LNCT_PATH, &mut header_tail);
    uleb128(DW_FORM_STRING, &mut header_tail);
    match table {
        Some(t) => {
            uleb128(1, &mut header_tail); // file_names_count
            header_tail.extend_from_slice(t.file_name().as_bytes());
            header_tail.push(0);
        }
        None => {
            uleb128(0, &mut header_tail); // file_names_count
        }
    }

    // unit_length counts the bytes AFTER its own 4-byte field (DWARF 5 spec, section 7.4):
    // version (2) + address_size (1) + segment_selector_size (1) + header_length field (4),
    // then the header body and the program. This matches the .debug_info writer in dwarf.rs.
    let after_length: usize = 2 + 1 + 1 + 4;
    let unit_length = after_length + header_tail.len() + program.len();
    let unit_length = u32::try_from(unit_length)
        .map_err(|_| ObjectError::ArithmeticOverflow { context: "dwarf line unit_length" })?;

    let mut out: Vec<u8> = Vec::with_capacity(unit_length as usize);
    out.extend_from_slice(&unit_length.to_le_bytes());
    out.extend_from_slice(&LINE_VERSION.to_le_bytes());
    out.push(crate::dwarf::DWARF_ADDRESS_SIZE);
    out.push(0); // segment_selector_size
    let header_length = header_tail.len() as u32;
    out.extend_from_slice(&header_length.to_le_bytes());
    out.extend_from_slice(&header_tail);
    out.extend_from_slice(&program);
    Ok(out)
}

/// CEP:WHAT: Emits the line-program byte stream: rows sorted stably by address, each row
///   advancing the state with the smallest opcode set (advance_pc, advance_line,
///   set_column, set_file, negate_stmt) and closing rows with DW_LNS_copy or
///   DW_LNE_end_sequence; the program always ends with an end_sequence.
/// CEP:WHY: The state machine is the only consumer-visible mapping of addresses to source
///   lines; minimal opcodes keep it small and deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: ArithmeticOverflow if a row address precedes the state address after
///   sorting (unreachable) or a line number exceeds i64 (blocked by MAX_LINE_NUMBER);
///   never panics.
/// CEP:ASSUMES: initial state is address 0, file 0, line 1, column 0, is_stmt true.
/// CEP:COST: O(rows log rows + emitted bytes).
/// CEP:EVIDENCE: state-machine and empty-table tests.
fn emit_line_program(table: Option<&LineTable>) -> Result<Vec<u8>, ObjectError> {
    // (address, file, line, column, is_stmt, end_sequence) in stable address order.
    let mut rows: Vec<(u64, u64, u64, u64, bool, bool)> = Vec::new();
    if let Some(t) = table {
        let mut idx: Vec<usize> = (0..t.rows().len()).collect();
        idx.sort_by_key(|&i| t.rows()[i].address);
        for &i in &idx {
            let r = &t.rows()[i];
            rows.push((r.address, r.file, r.line, r.column, r.is_stmt, r.end_sequence));
        }
    }
    let mut program: Vec<u8> = Vec::new();
    let mut address: u64 = 0;
    let mut file: u64 = 0;
    let mut line: u64 = 1;
    let mut column: u64 = 0;
    let mut is_stmt: bool = DEFAULT_IS_STMT == 1;
    // An empty table still owes one end_sequence: the program must terminate its (empty)
    // sequence, so the state starts unclosed and the tail emits the extended opcode.
    let mut closed = false;
    for &(r_addr, r_file, r_line, r_col, r_stmt, r_end) in &rows {
        closed = false;
        if r_addr < address {
            return Err(ObjectError::ArithmeticOverflow {
                context: "dwarf line address delta",
            });
        }
        let delta = r_addr - address;
        if delta != 0 {
            program.push(DW_LNS_ADVANCE_PC);
            uleb128(delta, &mut program);
        }
        if r_line != line {
            program.push(DW_LNS_ADVANCE_LINE);
            let state_line = i64::try_from(line)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "dwarf line state" })?;
            let row_line = i64::try_from(r_line)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "dwarf line row" })?;
            sleb128(row_line - state_line, &mut program);
        }
        if r_col != column {
            program.push(DW_LNS_SET_COLUMN);
            uleb128(r_col, &mut program);
        }
        if r_file != file {
            program.push(DW_LNS_SET_FILE);
            uleb128(r_file, &mut program);
        }
        if r_stmt != is_stmt {
            program.push(DW_LNS_NEGATE_STMT);
        }
        if r_end {
            program.push(DW_LNE_ESCAPE);
            uleb128(1, &mut program);
            program.push(DW_LNE_END_SEQUENCE);
            address = 0;
            file = 0;
            line = 1;
            column = 0;
            is_stmt = true;
            closed = true;
        } else {
            program.push(DW_LNS_COPY);
            address = r_addr;
            file = r_file;
            line = r_line;
            column = r_col;
            is_stmt = r_stmt;
        }
    }
    if !closed {
        program.push(DW_LNE_ESCAPE);
        uleb128(1, &mut program);
        program.push(DW_LNE_END_SEQUENCE);
    }
    Ok(program)
}
