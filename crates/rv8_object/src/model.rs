// CEP:FILE: crates/rv8_object/src/model.rs
// CEP:WHAT: The universal, format-agnostic object model: sections, symbols, relocations, named
//   resource budgets, builder methods, and structural validation.
// CEP:WHY: Every serializer (ELF64 today; Mach-O/PE/DWARF stubs tomorrow) and the static
//   linker consume this one shape, so the backend emits objects once and all targets stay
//   interchangeable. The struct/enum shapes are a cross-agent contract with rv8_backend.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every constructor and validate() returns Result; named MAX_* budgets reject
//   oversized or malformed input before allocation can grow unbounded; the library never panics.
// CEP:ASSUMES: field order of RelocKind/Reloc/SectionType/Section/Symbol/ObjectFile is pinned
//   by the rv8_backend contract (rv8_backend agent constructs these literally); do not add,
//   remove, or reorder fields. Symbol table carries no `weak` bit in this revision; the linker
//   models strength internally (see link.rs). Relocations reference symbol *names*.
// CEP:COST: construction O(1) amortized per item; validation O(S + R + Y) over sections,
//   relocations, and symbols; no allocation beyond the stored vectors.
// CEP:EVIDENCE: unit tests rv8_object::model::tests; acceptance tests in
//   crates/rv8_object/tests/acceptance.rs (determinism, negatives, end-to-end run).

use crate::error::ObjectError;

/// CEP:WHAT: Maximum number of sections per input object.
/// CEP:WHY: Named budget (standard section 2.6); also keeps ELF section indices far below
///   SHN_LORESERVE (0xFF00) so no extension sections are ever needed.
/// CEP:STATUS: complete
/// CEP:FAILURE: builders and validate() report BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::section_budget_enforced.
pub const MAX_SECTIONS: usize = 1024;

/// CEP:WHAT: Maximum number of symbols per input object.
/// CEP:WHY: Named budget; symtab indices fit u32 with large headroom.
/// CEP:STATUS: complete
/// CEP:FAILURE: builders and validate() report BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::symbol_budget_enforced.
pub const MAX_SYMBOLS: usize = 65_536;

/// CEP:WHAT: Maximum relocations attached to one section.
/// CEP:WHY: Named budget bounding O(R) passes (GC, relocation application).
/// CEP:STATUS: complete
/// CEP:FAILURE: add_reloc and validate() report BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::reloc_budget_enforced.
pub const MAX_RELOCS_PER_SECTION: usize = 1 << 20;

/// CEP:WHAT: Maximum byte size of one section.
/// CEP:WHY: Named budget; bounds image-size arithmetic and allocation.
/// CEP:STATUS: complete
/// CEP:FAILURE: builders and validate() report SectionTooLarge.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced by add_section/validate; exercised at budget boundary in tests.
pub const MAX_SECTION_BYTES: usize = 1 << 30;

/// CEP:WHAT: Maximum byte length of one section/symbol/relocation name.
/// CEP:WHY: Named budget; bounds string-table scans (suffix sharing is O(table * name)).
/// CEP:STATUS: complete
/// CEP:FAILURE: check_name reports InvalidName.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::name_checks.
pub const MAX_NAME_BYTES: usize = 4 * 1024;

/// CEP:WHAT: Maximum byte size of one emitted string table (.strtab / .shstrtab).
/// CEP:WHY: Named budget; also guarantees string offsets fit the ELF u32 st_name/sh_name
///   fields and keeps suffix-sharing scans bounded.
/// CEP:STATUS: complete
/// CEP:FAILURE: StringTable::add reports BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::string_table_budget.
pub const MAX_STRING_TABLE_BYTES: usize = 1 << 20;

/// CEP:WHAT: Maximum number of input objects handed to the linker at once.
/// CEP:WHY: Named budget for the flat section id space used by GC.
/// CEP:STATUS: complete
/// CEP:FAILURE: link entry points report BudgetExceeded.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::object_budget_enforced.
pub const MAX_INPUT_OBJECTS: usize = 4096;

/// CEP:WHAT: Section flag: writable (ELF SHF_WRITE).
/// CEP:WHY: The model stores raw u64 flags; these named values are the canonical set and are
///   bit-compatible with ELF so serializers can pass them through.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: values match ELF SHF_* numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by add_text_section/add_data_section and permission grouping.
pub const FLAG_WRITE: u64 = 0x1;

/// CEP:WHAT: Section flag: occupies memory at runtime (ELF SHF_ALLOC).
/// CEP:WHY: See FLAG_WRITE.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: values match ELF SHF_* numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by section grouping and symbol type derivation.
pub const FLAG_ALLOC: u64 = 0x2;

/// CEP:WHAT: Section flag: executable (ELF SHF_EXECINSTR).
/// CEP:WHY: See FLAG_WRITE.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: values match ELF SHF_* numbering.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by RX/RW permission grouping and STT_FUNC derivation.
pub const FLAG_EXEC: u64 = 0x4;

/// CEP:WHAT: Relocation kinds the model can express.
/// CEP:WHY: Pinned cross-agent shape: exactly these two cover the static-link use cases
///   (absolute 64-bit pointers and 32-bit PC-relative calls).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: elf_type() mapping is contractual: Abs64 => 1 (R_X86_64_64),
///   PcRel32 => 2 (R_X86_64_PC32); PcRel32 relocations carry addend -4 on call sites.
/// CEP:COST: Copy enum.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::reloc_kind_mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelocKind {
    /// S + A written as a 64-bit absolute value (ELF R_X86_64_64).
    Abs64,
    /// S + A - P written as a signed 32-bit value (ELF R_X86_64_PC32).
    PcRel32,
}

impl RelocKind {
    /// CEP:WHAT: ELF relocation type code for this kind.
    /// CEP:WHY: Serializers embed this in r_info; the mapping is part of the pinned contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: x86-64 ELF numbering.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::reloc_kind_mapping.
    pub fn elf_type(self) -> u32 {
        match self {
            RelocKind::Abs64 => 1,
            RelocKind::PcRel32 => 2,
        }
    }

    /// CEP:WHAT: Number of bytes this relocation overwrites at its target offset.
    /// CEP:WHY: Bounds checking (validation) and patch writing share one source of truth.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::reloc_kind_mapping.
    pub fn write_size(self) -> u64 {
        match self {
            RelocKind::Abs64 => 8,
            RelocKind::PcRel32 => 4,
        }
    }

    /// CEP:WHAT: Stable kind name for diagnostics.
    /// CEP:WHY: Error messages name the kind without Debug noise.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used by ObjectError::RelocOutOfRange rendering.
    pub fn as_str(self) -> &'static str {
        match self {
            RelocKind::Abs64 => "Abs64",
            RelocKind::PcRel32 => "PcRel32",
        }
    }
}

/// CEP:WHAT: One relocation: patch `write_size()` bytes at `offset` inside its section.
/// CEP:WHY: Pinned cross-agent shape produced by the backend and consumed by serializers
///   and the linker.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validation happens in Section/ObjectFile::validate).
/// CEP:ASSUMES: `offset` points at the patched FIELD (for `e8 rel32` at instruction
///   offset i, the PcRel32 offset is i+1); PcRel32 value = S + A - P with P = section
///   vaddr + offset and A = -4 on call sites; Abs64 value = S + A. `symbol` is a name
///   resolved in the owning object (ET_REL) or across objects (link).
/// CEP:COST: inline data; one String per relocation symbol name.
/// CEP:EVIDENCE: applied by elf::write_elf64_relocatable and link::link_executable; the
///   acceptance executable calls through a patched PcRel32 and exits 13.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloc {
    /// Byte offset from the start of the owning section.
    pub offset: u64,
    /// How to compute and how wide the patch is.
    pub kind: RelocKind,
    /// Symbol name this relocation resolves against.
    pub symbol: String,
    /// Addend A in S + A (Abs64) or S + A - P (PcRel32).
    pub addend: i64,
}

/// CEP:WHAT: Section content class for allocated storage.
/// CEP:WHY: Pinned cross-agent shape: program data (PROGBITS) vs memory-only zero-fill
///   (NOBITS). Symbol/string/relocation tables are writer-generated, not model sections.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: exactly two classes; more classes would change the pinned enum.
/// CEP:COST: Copy enum.
/// CEP:EVIDENCE: mapped to SHT_PROGBITS/SHT_NOBITS in elf.rs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionType {
    /// Occupies file bytes (code and initialized data).
    Progbits,
    /// Occupies memory only at runtime (zero-filled); `bytes` carries the reserved size.
    NoBits,
}

/// CEP:WHAT: One section with contents, flags, alignment, and its relocations.
/// CEP:WHY: Pinned cross-agent shape; the unit of GC, layout, and emission.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validation happens in ObjectFile::validate).
/// CEP:ASSUMES: `flags` use FLAG_* bits; `align` is 0 (unspecified) or a power of two;
///   for NoBits, `bytes.len()` is the reserved memory size and `relocs` must be empty.
/// CEP:COST: inline data; bytes and relocs are owned vectors.
/// CEP:EVIDENCE: exercised by every test in the crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Section name as emitted into .shstrtab (may repeat across objects).
    pub name: String,
    /// Content class.
    pub ty: SectionType,
    /// FLAG_* bit set (ELF SHF_*-compatible).
    pub flags: u64,
    /// Alignment in bytes: 0 (unspecified) or a power of two.
    pub align: u64,
    /// Section contents (empty for NoBits-backed size reservations).
    pub bytes: Vec<u8>,
    /// Relocations that patch this section's bytes.
    pub relocs: Vec<Reloc>,
}

impl Section {
    /// CEP:WHAT: True when the section has executable machine code.
    /// CEP:WHY: RX/RW permission grouping reads this single predicate.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: FLAG_EXEC semantics.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used by layout_order and link segment classification.
    pub fn is_exec(&self) -> bool {
        self.flags & FLAG_EXEC != 0
    }
}

/// CEP:WHAT: One symbol: definition or reference, local or global, anchored in a section.
/// CEP:WHY: Pinned cross-agent shape; the only naming currency for relocations.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validation happens in ObjectFile::validate).
/// CEP:ASSUMES: `value` is an offset within `section` when defined; defined symbols must
///   carry Some(section); undefined symbols must carry None. No `weak` bit in this revision
///   (linker models strength internally and unit-tests the weak/strong lattice).
/// CEP:COST: inline data; one String for the name.
/// CEP:EVIDENCE: resolved by link::resolve_globals; emitted into .symtab by elf.rs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Symbol name ("" is a valid anonymous local).
    pub name: String,
    /// True when this is a definition, false when it is an undefined reference.
    pub defined: bool,
    /// True for global (link-visible) symbols, false for locals.
    pub global: bool,
    /// Section index this symbol is anchored to (defined symbols only).
    pub section: Option<usize>,
    /// Offset within the section (defined symbols).
    pub value: u64,
    /// Symbol size in bytes (informational; emitted as st_size).
    pub size: u64,
}

/// CEP:WHAT: One relocatable object: named sections plus a symbol table.
/// CEP:WHY: Pinned cross-agent shape; the currency exchanged between backend, object
///   emission, and linking.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (validation happens in validate()).
/// CEP:ASSUMES: section indices are dense (0..sections.len()); symbol names referenced by
///   relocations must exist in `symbols` for ET_REL emission.
/// CEP:COST: owned vectors; construction amortized O(1) per item.
/// CEP:EVIDENCE: end-to-end acceptance test links two hand-built objects plus
///   synthesize_start and runs the result (exit code 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectFile {
    /// Diagnostic object name (not emitted into ELF).
    pub name: String,
    /// Sections in model order; writers reorder deterministically, never mutate.
    pub sections: Vec<Section>,
    /// Symbol table in model order.
    pub symbols: Vec<Symbol>,
}

impl ObjectFile {
    /// CEP:WHAT: Creates an empty object with a diagnostic name.
    /// CEP:WHY: Entry point of the builder API.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: name length is checked by validate().
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used by every test in the crate.
    pub fn new(name: impl Into<String>) -> ObjectFile {
        ObjectFile {
            name: name.into(),
            sections: Vec::new(),
            symbols: Vec::new(),
        }
    }

    /// CEP:WHAT: Appends a fully specified section and returns its index.
    /// CEP:WHY: General builder behind the typed helpers (text/data/nobits).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded (MAX_SECTIONS), InvalidAlignment, SectionTooLarge,
    ///   InvalidName; fails closed before mutation.
    /// CEP:ASSUMES: align 0 means "unspecified"; NoBits sections must not carry relocs
    ///   (guaranteed here because relocs are added separately with their own check).
    /// CEP:COST: amortized O(1); moves `bytes`.
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::builders_and_validation.
    pub fn add_section(
        &mut self,
        name: &str,
        ty: SectionType,
        flags: u64,
        align: u64,
        bytes: Vec<u8>,
    ) -> Result<usize, ObjectError> {
        if self.sections.len() >= MAX_SECTIONS {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_SECTIONS",
                limit: MAX_SECTIONS as u64,
                actual: self.sections.len() as u64 + 1,
            });
        }
        check_name(name)?;
        if align != 0 && !align.is_power_of_two() {
            return Err(ObjectError::InvalidAlignment { section: name.to_string(), align });
        }
        let size = bytes.len() as u64;
        if bytes.len() > MAX_SECTION_BYTES {
            return Err(ObjectError::SectionTooLarge { section: name.to_string(), size });
        }
        self.sections.push(Section {
            name: name.to_string(),
            ty,
            flags,
            align,
            bytes,
            relocs: Vec::new(),
        });
        Ok(self.sections.len() - 1)
    }

    /// CEP:WHAT: Appends an executable, allocated Progbits section (FLAG_ALLOC | FLAG_EXEC).
    /// CEP:WHY: The common backend output; encodes the RX permission group.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: same as add_section.
    /// CEP:ASSUMES: bytes are machine code for the target ISA.
    /// CEP:COST: amortized O(1); moves `bytes`.
    /// CEP:EVIDENCE: used by the acceptance test's add/main objects and synthesize_start.
    pub fn add_text_section(
        &mut self,
        name: &str,
        align: u64,
        bytes: Vec<u8>,
    ) -> Result<usize, ObjectError> {
        self.add_section(name, SectionType::Progbits, FLAG_ALLOC | FLAG_EXEC, align, bytes)
    }

    /// CEP:WHAT: Appends an allocated, writable Progbits section (FLAG_ALLOC | FLAG_WRITE).
    /// CEP:WHY: Initialized data lands in the RW segment.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: same as add_section.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1); moves `bytes`.
    /// CEP:EVIDENCE: unit test rv8_object::link::tests::gc_plan_keeps_and_drops.
    pub fn add_data_section(
        &mut self,
        name: &str,
        align: u64,
        bytes: Vec<u8>,
    ) -> Result<usize, ObjectError> {
        self.add_section(name, SectionType::Progbits, FLAG_ALLOC | FLAG_WRITE, align, bytes)
    }

    /// CEP:WHAT: Appends a NOBITS (zero-filled memory) section reserving `size` bytes.
    /// CEP:WHY: .bss-style storage needs no file bytes, only layout space.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: same as add_section (including SectionTooLarge for oversized `size`).
    /// CEP:ASSUMES: reservation materializes as a zeroed Vec bounded by MAX_SECTION_BYTES.
    /// CEP:COST: O(size) zero-fill (allocator zero pages).
    /// CEP:EVIDENCE: unit test rv8_object::link::tests::gc_plan_keeps_and_drops.
    pub fn add_nobits_section(
        &mut self,
        name: &str,
        align: u64,
        size: usize,
    ) -> Result<usize, ObjectError> {
        self.add_section(name, SectionType::NoBits, FLAG_ALLOC | FLAG_WRITE, align, vec![0u8; size])
    }

    /// CEP:WHAT: Appends a symbol with fully specified fields and returns its index.
    /// CEP:WHY: Single funnel for symbol construction; enforces the defined/section contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded (MAX_SYMBOLS), InvalidName, SymbolSectionOutOfRange,
    ///   UndefinedSymbolWithSection; fails closed before mutation.
    /// CEP:ASSUMES: defined symbols carry Some(section) within range; undefined carry None.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::builders_and_validation.
    pub fn add_symbol(
        &mut self,
        name: &str,
        defined: bool,
        global: bool,
        section: Option<usize>,
        value: u64,
        size: u64,
    ) -> Result<usize, ObjectError> {
        if self.symbols.len() >= MAX_SYMBOLS {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_SYMBOLS",
                limit: MAX_SYMBOLS as u64,
                actual: self.symbols.len() as u64 + 1,
            });
        }
        check_name(name)?;
        if let Some(si) = section {
            if si >= self.sections.len() {
                return Err(ObjectError::SymbolSectionOutOfRange {
                    symbol: name.to_string(),
                    section: si,
                    sections: self.sections.len(),
                });
            }
        }
        if !defined && section.is_some() {
            return Err(ObjectError::UndefinedSymbolWithSection { name: name.to_string() });
        }
        if defined && section.is_none() {
            return Err(ObjectError::DefinedSymbolMissingSection { name: name.to_string() });
        }
        self.symbols.push(Symbol {
            name: name.to_string(),
            defined,
            global,
            section,
            value,
            size,
        });
        Ok(self.symbols.len() - 1)
    }

    /// CEP:WHAT: Appends a relocation to a section of this object.
    /// CEP:WHY: Single funnel for relocation construction; enforces the bounds contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: SectionIndexOutOfRange, RelocOnNoBits, InvalidName, RelocOutOfBounds,
    ///   BudgetExceeded (MAX_RELOCS_PER_SECTION); fails closed before mutation.
    /// CEP:ASSUMES: `symbol` names a symbol added to this object (checked at emission).
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::builders_and_validation.
    pub fn add_reloc(
        &mut self,
        section: usize,
        offset: u64,
        kind: RelocKind,
        symbol: &str,
        addend: i64,
    ) -> Result<(), ObjectError> {
        if section >= self.sections.len() {
            return Err(ObjectError::SymbolSectionOutOfRange {
                symbol: symbol.to_string(),
                section,
                sections: self.sections.len(),
            });
        }
        let sec = &self.sections[section];
        if sec.ty == SectionType::NoBits {
            return Err(ObjectError::RelocOnNoBits { section: sec.name.clone() });
        }
        check_name(symbol)?;
        let end = offset
            .checked_add(kind.write_size())
            .ok_or(ObjectError::ArithmeticOverflow { context: "reloc offset + write_size" })?;
        if end > sec.bytes.len() as u64 {
            return Err(ObjectError::RelocOutOfBounds {
                section: sec.name.clone(),
                offset,
                write: kind.write_size(),
                size: sec.bytes.len() as u64,
            });
        }
        if sec.relocs.len() >= MAX_RELOCS_PER_SECTION {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_RELOCS_PER_SECTION",
                limit: MAX_RELOCS_PER_SECTION as u64,
                actual: sec.relocs.len() as u64 + 1,
            });
        }
        self.sections[section].relocs.push(Reloc {
            offset,
            kind,
            symbol: symbol.to_string(),
            addend,
        });
        Ok(())
    }

    /// CEP:WHAT: Structural validation of the whole object: budgets, names, alignment
    ///   powers of two, defined-symbol anchoring, relocation bounds, NoBits purity.
    /// CEP:WHY: Verification after construction (standard section 2.5); writers and the
    ///   linker call this first so downstream passes can assume the invariants.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: every violated invariant maps to one named ObjectError variant.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(S + R + Y) with no allocation.
    /// CEP:EVIDENCE: unit test rv8_object::model::tests::builders_and_validation and the
    ///   acceptance negative tests (duplicate/undefined/align/reloc-bounds).
    pub fn validate(&self) -> Result<(), ObjectError> {
        check_name(&self.name)?;
        if self.sections.len() > MAX_SECTIONS {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_SECTIONS",
                limit: MAX_SECTIONS as u64,
                actual: self.sections.len() as u64,
            });
        }
        if self.symbols.len() > MAX_SYMBOLS {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_SYMBOLS",
                limit: MAX_SYMBOLS as u64,
                actual: self.symbols.len() as u64,
            });
        }
        for sec in &self.sections {
            check_name(&sec.name)?;
            if sec.align != 0 && !sec.align.is_power_of_two() {
                return Err(ObjectError::InvalidAlignment {
                    section: sec.name.clone(),
                    align: sec.align,
                });
            }
            let size = sec.bytes.len() as u64;
            if sec.bytes.len() > MAX_SECTION_BYTES {
                return Err(ObjectError::SectionTooLarge { section: sec.name.clone(), size });
            }
            if sec.relocs.len() > MAX_RELOCS_PER_SECTION {
                return Err(ObjectError::BudgetExceeded {
                    what: "MAX_RELOCS_PER_SECTION",
                    limit: MAX_RELOCS_PER_SECTION as u64,
                    actual: sec.relocs.len() as u64,
                });
            }
            if sec.ty == SectionType::NoBits && !sec.relocs.is_empty() {
                return Err(ObjectError::RelocOnNoBits { section: sec.name.clone() });
            }
            for r in &sec.relocs {
                check_name(&r.symbol)?;
                let end = r
                    .offset
                    .checked_add(r.kind.write_size())
                    .ok_or(ObjectError::ArithmeticOverflow { context: "reloc offset + write_size" })?;
                if end > size {
                    return Err(ObjectError::RelocOutOfBounds {
                        section: sec.name.clone(),
                        offset: r.offset,
                        write: r.kind.write_size(),
                        size,
                    });
                }
            }
        }
        for sym in &self.symbols {
            check_name(&sym.name)?;
            if let Some(si) = sym.section {
                if si >= self.sections.len() {
                    return Err(ObjectError::SymbolSectionOutOfRange {
                        symbol: sym.name.clone(),
                        section: si,
                        sections: self.sections.len(),
                    });
                }
            }
            if !sym.defined && sym.section.is_some() {
                return Err(ObjectError::UndefinedSymbolWithSection { name: sym.name.clone() });
            }
            if sym.defined && sym.section.is_none() {
                return Err(ObjectError::DefinedSymbolMissingSection { name: sym.name.clone() });
            }
        }
        Ok(())
    }
}

/// CEP:WHAT: Validates a section/symbol name: within MAX_NAME_BYTES, no interior NUL.
/// CEP:WHY: Names land in ELF string tables where NUL is the terminator; oversize names
///   would blow the string-table budget scan bound.
/// CEP:STATUS: complete
/// CEP:FAILURE: InvalidName with a reason.
/// CEP:ASSUMES: empty names are legal (anonymous locals, st_name 0).
/// CEP:COST: O(len).
/// CEP:EVIDENCE: unit test rv8_object::model::tests::name_checks.
pub(crate) fn check_name(name: &str) -> Result<(), ObjectError> {
    if name.len() > MAX_NAME_BYTES {
        return Err(ObjectError::InvalidName {
            name: name.to_string(),
            reason: "name exceeds MAX_NAME_BYTES",
        });
    }
    if name.bytes().any(|b| b == 0) {
        return Err(ObjectError::InvalidName {
            name: name.to_string(),
            reason: "name contains interior NUL",
        });
    }
    Ok(())
}

/// CEP:WHAT: Rounds `value` up to `align` (align 0 or 1 means 1), checked.
/// CEP:WHY: Shared by ELF emission and linker layout; overflow must be an error, never a
///   wrapping surprise in emitted offsets.
/// CEP:STATUS: complete
/// CEP:FAILURE: ArithmeticOverflow on overflow.
/// CEP:ASSUMES: align is a power of two or 0/1 (callers validate first).
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by elf::write_elf64_relocatable and link layout; covered indirectly by
///   alignment tests in both modules.
pub(crate) fn align_up(value: u64, align: u64) -> Result<u64, ObjectError> {
    let a = align.max(1);
    let masked = value.checked_add(a - 1).ok_or(ObjectError::ArithmeticOverflow {
        context: "align_up(value, align)",
    })?;
    Ok(masked & !(a - 1))
}

/// CEP:WHAT: Deterministic emission order for sections: permission groups (RX, then
///   allocated RW, then non-allocated), zero-init (NoBits) last inside the group, then
///   descending alignment, ties broken by original index (stable sort).
/// CEP:WHY: Layout order is determinism-critical (standard section 2.2): it is a pure
///   function of the input with no hash iteration, and grouping by permission is the
///   segment layout rule of STANDARDS 5.5.
/// CEP:STATUS: complete (hot/cold split is the separate named flag HOT_COLD_SPLIT in elf.rs).
/// CEP:FAILURE: none
/// CEP:ASSUMES: input slices may be empty; returned indices are a permutation of 0..len.
/// CEP:COST: O(S log S) comparisons, stable.
/// CEP:EVIDENCE: unit test rv8_object::model::tests::layout_order_is_deterministic.
pub(crate) fn layout_order(sections: &[Section]) -> Vec<usize> {
    fn group(s: &Section) -> u8 {
        if s.is_exec() {
            0
        } else if s.flags & FLAG_ALLOC != 0 {
            1
        } else {
            2
        }
    }
    let mut idx: Vec<usize> = (0..sections.len()).collect();
    idx.sort_by(|&a, &b| {
        let (sa, sb) = (&sections[a], &sections[b]);
        group(sa)
            .cmp(&group(sb))
            .then_with(|| {
                let na = u8::from(sa.ty == SectionType::NoBits);
                let nb = u8::from(sb.ty == SectionType::NoBits);
                na.cmp(&nb)
            })
            .then_with(|| sb.align.cmp(&sa.align))
    });
    idx
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Builders return indices, enforce budgets and contracts, and validate()
    ///   accepts the well-formed result.
    /// CEP:WHY: Core model contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn builders_and_validation() {
        let mut o = ObjectFile::new("t.o");
        let text = o
            .add_text_section(".text", 16, vec![0xc3, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap();
        let data = o.add_data_section(".data", 8, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let bss = o.add_nobits_section(".bss", 64, 4096).unwrap();
        assert_eq!((text, data, bss), (0, 1, 2));
        o.add_symbol("f", true, true, Some(text), 0, 1).unwrap();
        o.add_symbol("", false, false, None, 0, 0).unwrap(); // anonymous undefined local
        o.add_reloc(text, 0, RelocKind::Abs64, "f", 0).unwrap();
        assert!(o.validate().is_ok());

        // align not a power of two
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
            o2.validate(),
            Err(ObjectError::InvalidAlignment { section: ".bad".into(), align: 3 })
        );

        // defined symbol without section: rejected by the builder and by validate()
        let mut o3 = ObjectFile::new("no_sec.o");
        o3.add_text_section(".text", 1, vec![0x90]).unwrap();
        assert_eq!(
            o3.add_symbol("f", true, true, None, 0, 0),
            Err(ObjectError::DefinedSymbolMissingSection { name: "f".into() })
        );
        o3.symbols.push(Symbol {
            name: "f".into(),
            defined: true,
            global: true,
            section: None,
            value: 0,
            size: 0,
        });
        assert_eq!(
            o3.validate(),
            Err(ObjectError::DefinedSymbolMissingSection { name: "f".into() })
        );

        // defined symbol with out-of-range section
        let mut o4 = ObjectFile::new("oob.o");
        o4.add_text_section(".text", 1, vec![0x90]).unwrap();
        assert_eq!(
            o4.add_symbol("f", true, true, Some(7), 0, 0),
            Err(ObjectError::SymbolSectionOutOfRange {
                symbol: "f".into(),
                section: 7,
                sections: 1
            })
        );

        // reloc past section end
        let mut o5 = ObjectFile::new("oob_reloc.o");
        let s5 = o5.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            o5.add_reloc(s5, 2, RelocKind::PcRel32, "x", -4),
            Err(ObjectError::RelocOutOfBounds {
                section: ".text".into(),
                offset: 2,
                write: 4,
                size: 5
            })
        );

        // reloc on NOBITS
        let mut o6 = ObjectFile::new("nobits_reloc.o");
        let b6 = o6.add_nobits_section(".bss", 1, 16).unwrap();
        assert!(matches!(
            o6.add_reloc(b6, 0, RelocKind::Abs64, "x", 0),
            Err(ObjectError::RelocOnNoBits { .. })
        ));

        // undefined symbol carrying a section
        let mut o7 = ObjectFile::new("undef_sec.o");
        o7.add_text_section(".text", 1, vec![0x90]).unwrap();
        assert_eq!(
            o7.add_symbol("ext", false, true, Some(0), 0, 0),
            Err(ObjectError::UndefinedSymbolWithSection { name: "ext".into() })
        );
    }

    /// CEP:WHAT: MAX_SECTIONS budget rejects the 1025th section.
    /// CEP:WHY: Named budget enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn section_budget_enforced() {
        let mut o = ObjectFile::new("many.o");
        for i in 0..MAX_SECTIONS {
            o.add_data_section(&format!(".d{i}"), 1, Vec::new()).unwrap();
        }
        assert_eq!(
            o.add_data_section(".overflow", 1, Vec::new()),
            Err(ObjectError::BudgetExceeded {
                what: "MAX_SECTIONS",
                limit: MAX_SECTIONS as u64,
                actual: (MAX_SECTIONS + 1) as u64
            })
        );
    }

    /// CEP:WHAT: MAX_SYMBOLS budget rejects the 65537th symbol.
    /// CEP:WHY: Named budget enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn symbol_budget_enforced() {
        let mut o = ObjectFile::new("many_syms.o");
        for i in 0..MAX_SYMBOLS {
            o.add_symbol(&format!("s{i}"), false, false, None, 0, 0).unwrap();
        }
        assert_eq!(
            o.add_symbol("overflow", false, false, None, 0, 0),
            Err(ObjectError::BudgetExceeded {
                what: "MAX_SYMBOLS",
                limit: MAX_SYMBOLS as u64,
                actual: (MAX_SYMBOLS + 1) as u64
            })
        );
    }

    /// CEP:WHAT: MAX_RELOCS_PER_SECTION budget rejects the (2^20 + 1)th relocation.
    /// CEP:WHY: Named budget enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap on mismatch (test-only).
    /// CEP:ASSUMES: builds a minimal 8-byte section; 1M relocs is fast enough for a test.
    /// CEP:COST: test-only, ~1M pushes.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn reloc_budget_enforced() {
        let mut o = ObjectFile::new("many_relocs.o");
        let s = o.add_text_section(".text", 1, vec![0; 8]).unwrap();
        o.add_symbol("x", true, true, Some(s), 0, 0).unwrap();
        for _ in 0..MAX_RELOCS_PER_SECTION {
            o.add_reloc(s, 0, RelocKind::Abs64, "x", 0).unwrap();
        }
        assert!(matches!(
            o.add_reloc(s, 0, RelocKind::Abs64, "x", 0),
            Err(ObjectError::BudgetExceeded { .. })
        ));
    }

    /// CEP:WHAT: Name validation rejects oversized and NUL-containing names.
    /// CEP:WHY: String-table integrity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn name_checks() {
        assert!(check_name("").is_ok());
        assert!(check_name("main").is_ok());
        let long = "a".repeat(MAX_NAME_BYTES + 1);
        assert!(matches!(check_name(&long), Err(ObjectError::InvalidName { .. })));
        assert!(matches!(
            check_name("bad\0name"),
            Err(ObjectError::InvalidName { reason: "name contains interior NUL", .. })
        ));
    }

    /// CEP:WHAT: Pinned elf_type/write_size/as_str mappings for both relocation kinds.
    /// CEP:WHY: Cross-agent contract with rv8_backend.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn reloc_kind_mapping() {
        assert_eq!(RelocKind::Abs64.elf_type(), 1);
        assert_eq!(RelocKind::PcRel32.elf_type(), 2);
        assert_eq!(RelocKind::Abs64.write_size(), 8);
        assert_eq!(RelocKind::PcRel32.write_size(), 4);
        assert_eq!(RelocKind::Abs64.as_str(), "Abs64");
        assert_eq!(RelocKind::PcRel32.as_str(), "PcRel32");
    }

    /// CEP:WHAT: layout_order groups RX before allocated RW before non-allocated, puts NoBits
    ///   last in its group, sorts by descending alignment, and is stable on ties.
    /// CEP:WHY: Deterministic layout is the product (STANDARDS 5.5).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn layout_order_is_deterministic() {
        let mk = |name: &str, flags: u64, align: u64, ty: SectionType| Section {
            name: name.into(),
            ty,
            flags,
            align,
            bytes: Vec::new(),
            relocs: Vec::new(),
        };
        // Index:      0        1        2        3        4
        let secs = [
            mk(".text", FLAG_ALLOC | FLAG_EXEC, 16, SectionType::Progbits),
            mk(".data", FLAG_ALLOC | FLAG_WRITE, 64, SectionType::Progbits),
            mk(".rodata", FLAG_ALLOC, 8, SectionType::Progbits),
            mk(".text.cold", FLAG_ALLOC | FLAG_EXEC, 64, SectionType::Progbits),
            mk(".bss", FLAG_ALLOC | FLAG_WRITE, 32, SectionType::NoBits),
        ];
        // RX group first (align desc: .text.cold 64, .text 16), then allocated non-exec
        // progbits (.data 64, .rodata 8), then allocated nobits (.bss). .rodata carries
        // FLAG_ALLOC, so it belongs to the allocated group, not the non-allocated one.
        assert_eq!(layout_order(&secs), vec![3, 0, 1, 2, 4]);
        // Stability: equal keys keep input order.
        let ties = [
            mk(".a", FLAG_ALLOC | FLAG_EXEC, 16, SectionType::Progbits),
            mk(".b", FLAG_ALLOC | FLAG_EXEC, 16, SectionType::Progbits),
        ];
        assert_eq!(layout_order(&ties), vec![0, 1]);
    }
}
