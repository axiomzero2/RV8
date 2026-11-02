// CEP:FILE: crates/rv8_object/src/macho/mod.rs
// CEP:WHAT: Deterministic Mach-O 64-bit relocatable-object serializer (MH_OBJECT,
//   CPU_TYPE_X86_64): fixed load-command order (LC_BUILD_VERSION, LC_UUID, one
//   LC_SEGMENT_64 per permission group, LC_SYMTAB, optional LC_DYSYMTAB), sections ordered
//   by permission group then name, an nlist_64 symbol table partitioned locals/externals,
//   x86-64 relocation entries with the explicit model addend materialized into the patched
//   field (REL style), and a content-derived UUID (SHA-256 via rv8_core).
// CEP:WHY: STANDARDS 5.5 makes deterministic object emission the product on every target;
//   Mach-O is the Darwin target and must meet the same byte-determinism bar as ELF.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: model validation runs first; duplicate section names, duplicate symbol
//   names, empty global symbol names, non-external undefined symbols, alignment above
//   MACHO_MAX_ALIGN, section names above 16 mapped bytes, unknown relocation symbols, and
//   file cursors above MACHO_MAX_FILE_BYTES all return ObjectError; never panics.
// CEP:ASSUMES: little-endian x86_64 Mach-O consumer (MH_OBJECT files are not executed, so
//   segment vmaddr/fileoff pairs are cosmetic but deterministic); symbol n_value is
//   section-relative (MH_OBJECT convention); relocations carry explicit addends in the
//   model and REL-style implicit addends in the file, so the addend is written into the
//   patched field at emission.
// CEP:COST: O(S log S + R + Y + total bytes) plus one SHA-256 over section/symbol content;
//   one output allocation; reloc-bearing sections are cloned once for addend patching.
// CEP:EVIDENCE: tests/serializers.rs (golden bytes, load-command walk, symbol partition,
//   relocation encoding, UUID derivation, negatives, byte determinism) and
//   tests/macho_serialization.rs (determinism, hand-computed golden offsets, strtab dedup,
//   relocation encoding, dysymtab partition, one negative per rule).

mod consts;

pub use consts::*;

use std::collections::BTreeMap;

use rv8_core::sha256::sha256;

use crate::elf::{write_bytes_at, StringTable};
use crate::error::ObjectError;
use crate::model::{align_up, ObjectFile, RelocKind, Section, SectionType, Symbol, FLAG_ALLOC};

/// CEP:WHAT: Maps a section to its Mach-O segment and initial protection: executable code
///   to __TEXT (RX), allocated non-executable and zero-fill data to __DATA (RW),
///   non-allocated sections to __DWARF (R).
/// CEP:WHY: The permission-group rule of the stub design; zero-fill cannot live in a
///   read-only segment, so the NoBits class wins over the exec flag.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: MH_OBJECT segments are not loaded, so the mapping is a deterministic
///   convention rather than a loader constraint.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: section-ordering test in tests/serializers.rs.
fn segment_of(sec: &Section) -> (&'static str, u32) {
    if sec.ty == SectionType::NoBits {
        return (SEGNAME_DATA, VM_PROT_READ | VM_PROT_WRITE);
    }
    if sec.is_exec() {
        return (SEGNAME_TEXT, VM_PROT_READ | VM_PROT_EXECUTE);
    }
    if sec.flags & FLAG_ALLOC != 0 {
        return (SEGNAME_DATA, VM_PROT_READ | VM_PROT_WRITE);
    }
    (SEGNAME_NONALLOC, VM_PROT_READ)
}

/// CEP:WHAT: Deterministic segment ordering rank (__TEXT first, then __DATA, __DWARF).
/// CEP:WHY: Permission groups order the file like the ELF RX/RW rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: segment_of values.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: section-ordering test in tests/serializers.rs.
fn segment_rank(sec: &Section) -> u8 {
    let (name, _) = segment_of(sec);
    if name == SEGNAME_TEXT {
        0
    } else if name == SEGNAME_DATA {
        1
    } else {
        2
    }
}

/// CEP:WHAT: Maps a model section name to a Mach-O sectname: leading dots stripped,
///   '.' and '-' become '_', and the platform double-underscore prefix is prepended
///   (".text" becomes "__text"); at most 16 bytes.
/// CEP:WHY: Matches the toolchain convention ("__text", "__data") while staying total and
///   deterministic over arbitrary model names.
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidName when the mapped name exceeds MACHO_SECTION_NAME_BYTES.
/// CEP:ASSUMES: caller has validated the input name (no NUL, within MAX_NAME_BYTES).
/// CEP:COST: O(name).
/// CEP:EVIDENCE: negative test macho_rejects_align_and_name_budgets and the golden tests.
fn macho_section_name(name: &str) -> Result<String, ObjectError> {
    let mapped: String = name
        .trim_start_matches('.')
        .chars()
        .map(|c| if c == '.' || c == '-' { '_' } else { c })
        .collect();
    let mapped = format!("__{mapped}");
    if mapped.len() > MACHO_SECTION_NAME_BYTES {
        return Err(ObjectError::InvalidName {
            name: name.to_string(),
            reason: "mapped Mach-O sectname exceeds 16 bytes",
        });
    }
    Ok(mapped)
}

/// CEP:WHAT: Builds the 16-byte LC_UUID payload: the first MACHO_UUID_BYTES bytes of the
///   SHA-256 over the canonical content serialization (sections in emission order with
///   name/type/flags/align/length/bytes, then symbols in model order with
///   name/defined/global/section/value/size).
/// CEP:WHY: Reproducible builds need a UUID that is a pure function of the input; rv8_core
///   SHA-256 is the crate's established content hash (driver cache interim).
/// CEP:STATUS: complete
/// CEP:FAILURE: none (total over any validated input).
/// CEP:ASSUMES: rv8_core::sha256 is deterministic and side-effect free.
/// CEP:COST: O(total content bytes).
/// CEP:EVIDENCE: uuid derivation test in tests/serializers.rs.
fn uuid_of_content(obj: &ObjectFile, order: &[usize]) -> [u8; MACHO_UUID_BYTES] {
    let mut content: Vec<u8> = Vec::new();
    for &i in order {
        let sec = &obj.sections[i];
        content.extend_from_slice(sec.name.as_bytes());
        content.push(0);
        content.push(u8::from(sec.ty == SectionType::NoBits));
        content.extend_from_slice(&sec.flags.to_le_bytes());
        content.extend_from_slice(&sec.align.to_le_bytes());
        content.extend_from_slice(&(sec.bytes.len() as u64).to_le_bytes());
        content.extend_from_slice(&sec.bytes);
    }
    for sym in &obj.symbols {
        content.extend_from_slice(sym.name.as_bytes());
        content.push(0);
        content.push(u8::from(sym.defined));
        content.push(u8::from(sym.global));
        content.extend_from_slice(&(sym.section.map_or(u64::MAX, |s| s as u64)).to_le_bytes());
        content.extend_from_slice(&sym.value.to_le_bytes());
        content.extend_from_slice(&sym.size.to_le_bytes());
    }
    let digest = sha256(&content);
    let mut uuid = [0u8; MACHO_UUID_BYTES];
    uuid.copy_from_slice(&digest[..MACHO_UUID_BYTES]);
    uuid
}

/// CEP:WHAT: n_type byte for one emitted symbol: N_SECT (0x0E) for local definitions,
///   N_SECT|N_EXT (0x0F) for global definitions, N_UNDF|N_EXT (0x01) for undefined
///   externals.
/// CEP:WHY: The Mach-O symbol classes are fixed by the partition rule; one function keeps
///   the numbering in one place.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: callers rejected undefined non-global symbols before this point.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: symbol-partition test in tests/serializers.rs.
fn nlist_type(defined: bool, global: bool) -> u8 {
    const N_SECT: u8 = 0x0E;
    const N_EXT: u8 = 0x01;
    match (defined, global) {
        (true, true) => N_SECT | N_EXT,
        (true, false) => N_SECT,
        (false, _) => N_EXT,
    }
}

/// CEP:WHAT: Emits a Mach-O 64-bit relocatable object (MH_OBJECT, CPU_TYPE_X86_64) from
///   the universal model with fixed load-command order: LC_BUILD_VERSION (fixed platform
///   and versions), LC_UUID (content hash), one LC_SEGMENT_64 per non-empty permission
///   group (__TEXT, __DATA, __DWARF; sections inside sorted by name), LC_SYMTAB
///   (locals, then defined externals, then undefined externals, model order within each
///   class), and LC_DYSYMTAB when any external symbol exists. Relocations are emitted per
///   section with the model addend written into the patched field (REL-style).
/// CEP:WHY: The Darwin serializer of the universal model; byte-determinism is the bar.
/// CEP:STATUS: complete
/// CEP:FAILURE: validate() rejections, InvalidName (duplicate section names, empty global
///   names, non-external undefined symbols, oversize mapped sectnames), BudgetExceeded
///   (MACHO_MAX_ALIGN, MACHO_MAX_FILE_BYTES, MACHO_SYMBOL_INDEX_SPACE), UnknownRelocSymbol,
///   ArithmeticOverflow, WriteOutOfBounds; never panics.
/// CEP:ASSUMES: see CEP:FILE header assumptions.
/// CEP:COST: O(S log S + R + Y + total bytes + content hash).
/// CEP:EVIDENCE: tests/serializers.rs (golden bytes, structural walks, negatives) and
///   tests/macho_serialization.rs (determinism, hand-computed offsets, strtab dedup,
///   relocation encoding, dysymtab partition, one negative per rule).
pub fn write_macho64(obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    obj.validate()?;

    // Duplicate model section names cannot round-trip unambiguously in the
    // (sectname, segname) world; reject before any layout work.
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

    // Duplicate non-empty symbol names are ambiguous inside one nlist table: two records
    // would both claim the same name, so relocation binding and link-time dedup could not
    // agree on which definition wins. Anonymous locals ("") may repeat.
    let mut sym_names: Vec<&str> = obj.symbols.iter().map(|s| s.name.as_str()).collect();
    sym_names.sort_unstable();
    for pair in sym_names.windows(2) {
        if !pair[0].is_empty() && pair[0] == pair[1] {
            return Err(ObjectError::InvalidName {
                name: pair[0].to_string(),
                reason: "duplicate symbol name",
            });
        }
    }

    // Symbol-level contracts beyond the model: externals need names, undefined symbols
    // need externality.
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
                reason: "undefined Mach-O symbols must be external",
            });
        }
    }

    for sec in &obj.sections {
        if sec.align > MACHO_MAX_ALIGN {
            return Err(ObjectError::BudgetExceeded {
                what: "MACHO_MAX_ALIGN",
                limit: MACHO_MAX_ALIGN,
                actual: sec.align,
            });
        }
        macho_section_name(&sec.name)?;
    }

    // Emission order: permission group, then section name (unique after the duplicate
    // check), so the order is a pure function of the model.
    let n = obj.sections.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        segment_rank(&obj.sections[a])
            .cmp(&segment_rank(&obj.sections[b]))
            .then_with(|| obj.sections[a].name.cmp(&obj.sections[b].name))
    });

    // Symbol partition: locals, then defined externals, then undefined externals; model
    // order inside each class (stable sort).
    let mut sym_order: Vec<usize> = (0..obj.symbols.len()).collect();
    sym_order.sort_by(|&a, &b| {
        let ka = sym_class_rank(&obj.symbols[a]);
        let kb = sym_class_rank(&obj.symbols[b]);
        ka.cmp(&kb)
    });
    let n_local = sym_order.iter().filter(|&&m| !obj.symbols[m].global).count();
    let n_extdef = sym_order
        .iter()
        .filter(|&&m| obj.symbols[m].global && obj.symbols[m].defined)
        .count();
    let n_undef = obj.symbols.len() - n_local - n_extdef;

    // String table (suffix-shared) in emitted symbol order.
    let mut strtab = StringTable::new();
    let mut sym_strx = vec![0u32; obj.symbols.len()];
    for &m in &sym_order {
        sym_strx[m] = strtab.add(&obj.symbols[m].name)?;
    }

    // First-occurrence emitted index per symbol name for relocation resolution.
    let mut first_sym: BTreeMap<&str, u32> = BTreeMap::new();
    for (pos, &m) in sym_order.iter().enumerate() {
        first_sym.entry(obj.symbols[m].name.as_str()).or_insert(pos as u32);
    }

    // UUID before any cursor math: it depends only on model content and emission order.
    let uuid = uuid_of_content(obj, &order);

    // Segment plan: one segment per non-empty permission group, in rank order.
    let mut segs: Vec<(&'static str, u32, Vec<usize>)> = Vec::new();
    for &i in &order {
        let (name, prot) = segment_of(&obj.sections[i]);
        match segs.iter_mut().find(|(n, _, _)| *n == name) {
            Some((_, _, members)) => members.push(i),
            None => segs.push((name, prot, vec![i])),
        }
    }

    let has_dysymtab = n_extdef + n_undef > 0;
    let ncmds = 2u32 + segs.len() as u32 + 1u32 + u32::from(has_dysymtab);
    let sizeofcmds = MACHO_CMD24_BYTES
        + MACHO_CMD24_BYTES
        + segs
            .iter()
            .map(|(_, _, members)| MACHO_SEGMENT_HEADER_BYTES + members.len() * MACHO_SECTION_BYTES)
            .sum::<usize>()
        + MACHO_CMD24_BYTES
        + if has_dysymtab { MACHO_DYSYMTAB_CMD_BYTES } else { 0 };

    // Cursor plan, every step checked against the u32 offset budget.
    let mut cursor = (MACHO_HEADER_BYTES + sizeofcmds) as u64;
    check_file_cursor(cursor)?;
    let mut sec_off = vec![0u64; n];
    let mut sec_addr = vec![0u64; n];
    let mut seg_plan: Vec<(u64, u64, u64)> = Vec::with_capacity(segs.len());
    for (_, _, members) in &segs {
        let seg_start = cursor;
        let mut file_end = seg_start;
        let mut mem_end = seg_start;
        for &i in members {
            let sec = &obj.sections[i];
            cursor = align_up(cursor, sec.align)?;
            sec_off[i] = cursor;
            sec_addr[i] = cursor;
            let size = sec.bytes.len() as u64;
            if sec.ty == SectionType::Progbits {
                cursor = cursor
                    .checked_add(size)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "macho section cursor" })?;
                file_end = cursor;
            } else {
                file_end = file_end.max(cursor);
            }
            mem_end = mem_end
                .checked_add(size)
                .ok_or(ObjectError::ArithmeticOverflow { context: "macho segment vmsize" })?;
            mem_end = mem_end.max(file_end);
        }
        check_file_cursor(cursor)?;
        seg_plan.push((seg_start, file_end - seg_start, mem_end - seg_start));
    }

    let mut reloc_off = vec![0u64; n];
    for &i in &order {
        if obj.sections[i].relocs.is_empty() {
            continue;
        }
        cursor = align_up(cursor, MACHO_RECORD_ALIGN)?;
        reloc_off[i] = cursor;
        cursor = cursor
            .checked_add((obj.sections[i].relocs.len() as u64) * MACHO_RELOC_BYTES as u64)
            .ok_or(ObjectError::ArithmeticOverflow { context: "macho reloc cursor" })?;
        check_file_cursor(cursor)?;
    }

    cursor = align_up(cursor, MACHO_RECORD_ALIGN)?;
    let symoff = cursor;
    cursor = cursor
        .checked_add((obj.symbols.len() as u64) * MACHO_NLIST_BYTES as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "macho symtab cursor" })?;
    check_file_cursor(cursor)?;
    let stroff = cursor;
    cursor = cursor
        .checked_add(strtab.len() as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "macho strtab cursor" })?;
    check_file_cursor(cursor)?;
    let total = cursor;

    let mut image = vec![0u8; usize::try_from(total)
        .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho image size" })?];

    // mach_header_64.
    let header: Vec<u8> = MH_MAGIC_64
        .to_le_bytes()
        .into_iter()
        .chain(CPU_TYPE_X86_64.to_le_bytes())
        .chain(CPU_SUBTYPE_X86_64_ALL.to_le_bytes())
        .chain(MH_OBJECT.to_le_bytes())
        .chain(ncmds.to_le_bytes())
        .chain((sizeofcmds as u32).to_le_bytes())
        .chain(0u32.to_le_bytes())
        .chain(0u32.to_le_bytes())
        .collect();
    write_bytes_at(&mut image, 0, &header)?;

    // LC_BUILD_VERSION.
    let mut cmd: Vec<u8> = Vec::with_capacity(MACHO_CMD24_BYTES);
    cmd.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
    cmd.extend_from_slice(&(MACHO_CMD24_BYTES as u32).to_le_bytes());
    cmd.extend_from_slice(&PLATFORM_MACOS.to_le_bytes());
    cmd.extend_from_slice(&MACHO_MIN_OS_VERSION.to_le_bytes());
    cmd.extend_from_slice(&MACHO_SDK_VERSION.to_le_bytes());
    write_bytes_at(&mut image, MACHO_HEADER_BYTES, &cmd)?;

    // LC_UUID.
    let mut cmd: Vec<u8> = Vec::with_capacity(MACHO_CMD24_BYTES);
    cmd.extend_from_slice(&LC_UUID.to_le_bytes());
    cmd.extend_from_slice(&(MACHO_CMD24_BYTES as u32).to_le_bytes());
    cmd.extend_from_slice(&uuid);
    write_bytes_at(&mut image, MACHO_HEADER_BYTES + MACHO_CMD24_BYTES, &cmd)?;

    // LC_SEGMENT_64 per group with its section records.
    let mut at = MACHO_HEADER_BYTES + 2 * MACHO_CMD24_BYTES;
    for ((name, prot, members), &(seg_start, filesize, memsize)) in
        segs.iter().zip(seg_plan.iter())
    {
        let cmdsize = MACHO_SEGMENT_HEADER_BYTES + members.len() * MACHO_SECTION_BYTES;
        let mut cmd: Vec<u8> = Vec::with_capacity(cmdsize);
        cmd.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
        cmd.extend_from_slice(&(cmdsize as u32).to_le_bytes());
        cmd.extend_from_slice(&pad_name(name)?);
        cmd.extend_from_slice(&seg_start.to_le_bytes()); // vmaddr
        cmd.extend_from_slice(&memsize.to_le_bytes()); // vmsize
        cmd.extend_from_slice(&seg_start.to_le_bytes()); // fileoff (lockstep)
        cmd.extend_from_slice(&filesize.to_le_bytes());
        cmd.extend_from_slice(&VM_PROT_ALL.to_le_bytes()); // maxprot
        cmd.extend_from_slice(&prot.to_le_bytes()); // initprot
        cmd.extend_from_slice(&(members.len() as u32).to_le_bytes());
        cmd.extend_from_slice(&0u32.to_le_bytes());
        for &i in members {
            let sec = &obj.sections[i];
            let (segname, _) = segment_of(sec);
            let mut rec: Vec<u8> = Vec::with_capacity(MACHO_SECTION_BYTES);
            rec.extend_from_slice(&pad_name(&macho_section_name(&sec.name)?)?);
            rec.extend_from_slice(&pad_name(segname)?);
            rec.extend_from_slice(&0u64.to_le_bytes()); // addr: MH_OBJECT convention
            rec.extend_from_slice(&(sec.bytes.len() as u64).to_le_bytes());
            let file_offset = match sec.ty {
                SectionType::Progbits => sec_off[i],
                SectionType::NoBits => 0,
            };
            // Both file-offset fields are u32 in section_64; cursors are already bounded by
            // MACHO_MAX_FILE_BYTES, and try_from keeps the invariant fail-closed.
            let off32 = u32::try_from(file_offset)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho section offset" })?;
            let reloff32 = u32::try_from(reloc_off[i])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho reloff" })?;
            rec.extend_from_slice(&off32.to_le_bytes());
            rec.extend_from_slice(&align_log2(sec.align)?.to_le_bytes());
            rec.extend_from_slice(&reloff32.to_le_bytes());
            rec.extend_from_slice(&(obj.sections[i].relocs.len() as u32).to_le_bytes());
            rec.extend_from_slice(&section_flags(sec).to_le_bytes());
            rec.extend_from_slice(&0u32.to_le_bytes()); // reserved1
            rec.extend_from_slice(&0u32.to_le_bytes()); // reserved2
            rec.extend_from_slice(&0u32.to_le_bytes()); // reserved3
            cmd.extend_from_slice(&rec);
        }
        write_bytes_at(&mut image, at, &cmd)?;
        at += cmdsize;
    }

    // LC_SYMTAB.
    let mut cmd: Vec<u8> = Vec::with_capacity(MACHO_CMD24_BYTES);
    cmd.extend_from_slice(&LC_SYMTAB.to_le_bytes());
    cmd.extend_from_slice(&(MACHO_CMD24_BYTES as u32).to_le_bytes());
    cmd.extend_from_slice(&(symoff as u32).to_le_bytes());
    cmd.extend_from_slice(&(obj.symbols.len() as u32).to_le_bytes());
    cmd.extend_from_slice(&(stroff as u32).to_le_bytes());
    cmd.extend_from_slice(&(strtab.len() as u32).to_le_bytes());
    write_bytes_at(&mut image, at, &cmd)?;
    at += MACHO_CMD24_BYTES;

    // LC_DYSYMTAB (only when externals exist).
    if has_dysymtab {
        let mut cmd: Vec<u8> = Vec::with_capacity(MACHO_DYSYMTAB_CMD_BYTES);
        cmd.extend_from_slice(&LC_DYSYMTAB.to_le_bytes());
        cmd.extend_from_slice(&(MACHO_DYSYMTAB_CMD_BYTES as u32).to_le_bytes());
        cmd.extend_from_slice(&0u32.to_le_bytes()); // ilocalsym
        cmd.extend_from_slice(&(n_local as u32).to_le_bytes());
        cmd.extend_from_slice(&(n_local as u32).to_le_bytes()); // iextdefsym
        cmd.extend_from_slice(&(n_extdef as u32).to_le_bytes());
        cmd.extend_from_slice(&((n_local + n_extdef) as u32).to_le_bytes()); // iundefsym
        cmd.extend_from_slice(&(n_undef as u32).to_le_bytes());
        cmd.extend_from_slice(&[0u8; MACHO_DYSYMTAB_CMD_BYTES - 32]);
        write_bytes_at(&mut image, at, &cmd)?;
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
                usize::try_from(sec_off[i])
                    .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho data offset" })?,
                &sec.bytes,
            )?;
            continue;
        }
        let mut data = sec.bytes.clone();
        for r in &sec.relocs {
            let at = usize::try_from(r.offset)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho reloc patch" })?;
            match r.kind {
                RelocKind::PcRel32 => {
                    write_bytes_at(&mut data, at, &(r.addend as i32).to_le_bytes())?;
                }
                RelocKind::Abs64 => {
                    write_bytes_at(&mut data, at, &(r.addend as u64).to_le_bytes())?;
                }
            }
        }
        write_bytes_at(
            &mut image,
            usize::try_from(sec_off[i])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho data offset" })?,
            &data,
        )?;
    }

    // Relocation entries: r_address (offset in section) + packed info word.
    for &i in &order {
        if reloc_off[i] == 0 {
            continue;
        }
        let mut table: Vec<u8> = Vec::with_capacity(obj.sections[i].relocs.len() * MACHO_RELOC_BYTES);
        for r in &obj.sections[i].relocs {
            let idx = first_sym
                .get(r.symbol.as_str())
                .copied()
                .ok_or_else(|| ObjectError::UnknownRelocSymbol { name: r.symbol.clone() })?;
            if idx as u64 > MACHO_SYMBOL_INDEX_SPACE {
                return Err(ObjectError::BudgetExceeded {
                    what: "MACHO_SYMBOL_INDEX_SPACE",
                    limit: MACHO_SYMBOL_INDEX_SPACE,
                    actual: idx as u64,
                });
            }
            let (pcrel, length, rtype) = match r.kind {
                RelocKind::Abs64 => (0u32, 3u32, X86_64_RELOC_UNSIGNED),
                RelocKind::PcRel32 => (1u32, 2u32, X86_64_RELOC_SIGNED),
            };
            let info = idx
                | (pcrel << 24)
                | (length << 25)
                | (1u32 << 27)
                | (rtype << 28);
            // r_address is a u32 section-relative offset; try_from keeps the u32 field
            // bound fail-closed instead of silently truncating.
            let addr32 = u32::try_from(r.offset)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho r_address" })?;
            table.extend_from_slice(&addr32.to_le_bytes());
            table.extend_from_slice(&info.to_le_bytes());
        }
        write_bytes_at(
            &mut image,
            usize::try_from(reloc_off[i])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho reloc offset" })?,
            &table,
        )?;
    }

    // nlist_64 records in partition order.
    let mut symtab: Vec<u8> = Vec::with_capacity(obj.symbols.len() * MACHO_NLIST_BYTES);
    for &m in &sym_order {
        let s = &obj.symbols[m];
        let n_value = if s.defined { s.value } else { 0 };
        let n_sect = if s.defined {
            u8::try_from(
                order
                    .iter()
                    .position(|&i| i == s.section.unwrap_or(0))
                    .map_or(0, |p| p + 1),
            )
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "macho n_sect" })?
        } else {
            0
        };
        symtab.extend_from_slice(&sym_strx[m].to_le_bytes());
        symtab.push(nlist_type(s.defined, s.global));
        symtab.push(n_sect);
        symtab.extend_from_slice(&0u16.to_le_bytes()); // n_desc
        symtab.extend_from_slice(&n_value.to_le_bytes());
    }
    write_bytes_at(
        &mut image,
        usize::try_from(symoff).map_err(|_| ObjectError::ArithmeticOverflow { context: "macho symoff" })?,
        &symtab,
    )?;
    write_bytes_at(
        &mut image,
        usize::try_from(stroff).map_err(|_| ObjectError::ArithmeticOverflow { context: "macho stroff" })?,
        strtab.as_bytes(),
    )?;

    Ok(image)
}

/// CEP:WHAT: Stable class rank for the symbol partition: locals (0), defined externals
///   (1), undefined externals (2).
/// CEP:WHY: Mach-O symtab conventions expect exactly this grouping.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: undefined non-global symbols were rejected before ordering.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: symbol-partition test in tests/serializers.rs.
fn sym_class_rank(s: &Symbol) -> u8 {
    if !s.defined {
        2
    } else if s.global {
        1
    } else {
        0
    }
}

/// CEP:WHAT: Zero-pads a fixed-width Mach-O name field to exactly 16 bytes.
/// CEP:WHY: Names are fixed-size NUL-padded fields, not string-table entries.
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidName when the name exceeds 16 bytes (defensive re-check).
/// CEP:ASSUMES: caller already length-checked (macho_section_name / named segments).
/// CEP:COST: O(bytes).
/// CEP:EVIDENCE: golden-byte tests.
fn pad_name(name: &str) -> Result<[u8; MACHO_SECTION_NAME_BYTES], ObjectError> {
    let bytes = name.as_bytes();
    if bytes.len() > MACHO_SECTION_NAME_BYTES {
        return Err(ObjectError::InvalidName {
            name: name.to_string(),
            reason: "Mach-O name field overflow",
        });
    }
    let mut out = [0u8; MACHO_SECTION_NAME_BYTES];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

/// CEP:WHAT: Section flags word: zero-fill type for NOBITS, instruction attributes for
///   code, S_ATTR_DEBUG inside __DWARF, zero otherwise.
/// CEP:WHY: One function owns the flag synthesis so section records stay consistent.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: segment_of grouping.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: nobits and section-ordering tests.
fn section_flags(sec: &Section) -> u32 {
    let mut flags = 0u32;
    if sec.ty == SectionType::NoBits {
        flags |= S_ZEROFILL;
    }
    if sec.is_exec() {
        flags |= S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS;
    }
    if segment_of(sec).0 == SEGNAME_NONALLOC {
        flags |= S_ATTR_DEBUG;
    }
    flags
}

/// CEP:WHAT: log2 of a power-of-two alignment for the section record's align field.
/// CEP:WHY: Mach-O stores align as an exponent; the model stores bytes.
/// CEP:STATUS: complete
/// CEP:FAILURE: ArithmeticOverflow when align is zero (unspecified) after the max(1) clamp
///   cannot happen; a non-power-of-two would be a model-contract bug (trailing_zeros is
///   only meaningful for powers of two, which validate() guarantees).
/// CEP:ASSUMES: align is 0 (treated as 1) or a power of two within MACHO_MAX_ALIGN.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: section-ordering test asserts the exponent for align 16.
fn align_log2(align: u64) -> Result<u32, ObjectError> {
    let a = align.max(1);
    let exp = a.ilog2();
    if (1u64 << exp) != a {
        return Err(ObjectError::ArithmeticOverflow { context: "macho align log2" });
    }
    Ok(exp)
}

/// CEP:WHAT: Rejects a file cursor above the u32 offset budget.
/// CEP:WHY: Mach-O offset fields are u32; overflow must be an error, not a wrap.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (MACHO_MAX_FILE_BYTES).
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: negative test macho_rejects_section_offset_overflow.
fn check_file_cursor(cursor: u64) -> Result<(), ObjectError> {
    if cursor > MACHO_MAX_FILE_BYTES {
        return Err(ObjectError::BudgetExceeded {
            what: "MACHO_MAX_FILE_BYTES",
            limit: MACHO_MAX_FILE_BYTES,
            actual: cursor,
        });
    }
    Ok(())
}
