// CEP:FILE: crates/rv8_object/src/elf.rs
// CEP:WHAT: Deterministic ELF64 serializers: the relocatable writer (ET_REL with generated
//   .symtab/.strtab/.rela.X/.shstrtab), the binary field encoders shared with the executable
//   writer in link.rs, checked little-endian write primitives, and the string table with
//   suffix sharing.
// CEP:WHY: STANDARDS 5.5 makes deterministic object emission the product: identical input
//   must produce identical bytes, which is achieved by pure index-driven ordering (no hash
//   iteration), stable sorts, and fixed padding.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete (hot/cold section split is the named stub flag HOT_COLD_SPLIT below)
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every emission step is bounds-checked; budget or bounds violations return
//   ObjectError (truncated writes are errors, never panics); validate() runs first.
// CEP:ASSUMES: little-endian x86-64 ELF64 ABI; input objects pass model validation;
//   relocations reference symbol names resolvable within the same object for ET_REL.
// CEP:COST: O(S log S + R + Y + total bytes) per emission; one output allocation; suffix
//   sharing adds O(table_scan) per added string, bounded by MAX_STRING_TABLE_BYTES.
// CEP:EVIDENCE: unit tests rv8_object::elf::tests (string table sharing, symtab/rela binary
//   layout, shstrtab suffix sharing, budgets); determinism acceptance test writes twice and
//   compares bytes.

use std::collections::BTreeMap;

use crate::error::ObjectError;
use crate::model::{align_up, check_name, layout_order, ObjectFile, SectionType, Symbol, FLAG_ALLOC, MAX_STRING_TABLE_BYTES};

// --- ELF64 constants (named; x86-64 System V ABI numbers) ---

pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
pub const ELFCLASS64: u8 = 2;
pub const ELFDATA2LSB: u8 = 1;
pub const EV_CURRENT: u8 = 1;
pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const EM_X86_64: u16 = 62;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;
pub const SHN_UNDEF: u16 = 0;
pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;
/// CEP:WHAT: Ehdr64 byte size.
/// CEP:WHY: Named constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ELF64 fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: encoders and tests share this constant.
pub const EHDR_BYTES: usize = 64;
/// CEP:WHAT: Shdr64 byte size.
/// CEP:WHY: Named constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ELF64 fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: encoders and tests share this constant.
pub const SHDR_BYTES: usize = 64;
/// CEP:WHAT: Phdr64 byte size.
/// CEP:WHY: Named constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ELF64 fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by link.rs program headers.
pub const PHDR_BYTES: usize = 56;
/// CEP:WHAT: Elf64_Sym byte size.
/// CEP:WHY: Named constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ELF64 fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by the symtab emitter and its tests.
pub const SYM_BYTES: usize = 24;
/// CEP:WHAT: Elf64_Rela byte size.
/// CEP:WHY: Named constant per standard section 2.6.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: ELF64 fixed layout.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: used by the rela emitter and its tests.
pub const RELA_BYTES: usize = 24;
/// CEP:WHAT: Page size used for segment congruence and RW separation.
/// CEP:WHY: PT_LOAD requires p_vaddr = p_offset (mod p_align); 0x1000 covers every
///   practically loaded x86-64 Linux configuration.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: target page size is a multiple of 0x1000.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: end-to-end acceptance test execs a linked image on Linux.
pub const ELF_PAGE_BYTES: u64 = 0x1000;
/// CEP:WHAT: Hard ceiling on emitted section-header count (below SHN_LORESERVE 0xFF00).
/// CEP:WHY: e_shnum/st_shndx are u16; staying below the reserve boundary means no ELF
///   section-index extension tables ever need to be emitted.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded when exceeded (unreachable under current MAX_SECTIONS).
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: checked in write_elf64_relocatable and link emission.
pub const MAX_OUTPUT_SECTIONS: usize = 0xFE00;

/// CEP:WHAT: Named flag for hot/cold section splitting (placing cold text into a separate
///   segment). Currently disabled.
/// CEP:WHY: The layout pipeline reserves the toggle point so a future pass can split
///   sections without changing emission code paths.
/// CEP:STATUS: stub (flag exists; splitting pass not implemented)
/// CEP:FAILURE: none
/// CEP:ASSUMES: cold sections would be identified by a naming or flag convention.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: none yet (stub).
pub const HOT_COLD_SPLIT: bool = false;

/// CEP:WHAT: An ELF string table under construction with exact-duplicate reuse and suffix
///   sharing: a string whose NUL-terminated bytes already appear anywhere in the table
///   reuses that offset, so ".text" reuses the tail of ".rela.text".
/// CEP:WHY: STANDARDS 5.5 requires suffix sharing; it shrinks .strtab/.shstrtab and keeps
///   offsets stable. The scan is a deterministic ascending offset search (no hashing), so
///   identical input always yields identical tables.
/// CEP:STATUS: complete
/// CEP:OWNER: axiomzero2
/// CEP:TICKET: RV8-100
/// CEP:FAILURE: add returns InvalidName for oversize/NUL names and BudgetExceeded when the
///   table would exceed MAX_STRING_TABLE_BYTES; never panics.
/// CEP:ASSUMES: names contain no interior NUL (enforced); offset 0 always means "".
/// CEP:COST: add is O(table_bytes * name_len) worst case with ascending scan; tables are
///   bounded by MAX_STRING_TABLE_BYTES and names by MAX_NAME_BYTES.
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::string_table_suffix_sharing and
///   string_table_budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTable {
    data: Vec<u8>,
}

impl StringTable {
    /// CEP:WHAT: Creates a table containing only the mandatory NUL byte (offset 0 = "").
    /// CEP:WHY: ELF string tables must start with a NUL.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::string_table_suffix_sharing.
    pub fn new() -> StringTable {
        StringTable { data: vec![0u8] }
    }

    /// CEP:WHAT: Interns `s` and returns its offset; reuses an existing occurrence,
    ///   preferring the smallest offset whose NUL-terminated bytes match exactly.
    /// CEP:WHY: Deduplication with suffix sharing per STANDARDS 5.5.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: InvalidName (oversize/NUL), BudgetExceeded (MAX_STRING_TABLE_BYTES).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(table * name) scan worst case, bounded by named budgets.
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::string_table_suffix_sharing.
    pub fn add(&mut self, s: &str) -> Result<u32, ObjectError> {
        check_name(s)?;
        if s.is_empty() {
            return Ok(0);
        }
        let bytes = s.as_bytes();
        let n = bytes.len();
        if self.data.len() + n + 1 > MAX_STRING_TABLE_BYTES {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_STRING_TABLE_BYTES",
                limit: MAX_STRING_TABLE_BYTES as u64,
                actual: self.data.len() as u64 + n as u64 + 1,
            });
        }
        let mut i = 1usize;
        while i + n < self.data.len() {
            if self.data[i] == bytes[0] && &self.data[i..i + n] == bytes && self.data[i + n] == 0 {
                return Ok(i as u32);
            }
            i += 1;
        }
        let off = self.data.len() as u32;
        self.data.extend_from_slice(bytes);
        self.data.push(0);
        Ok(off)
    }

    /// CEP:WHAT: The current table bytes (NUL-terminated strings back to back).
    /// CEP:WHY: Serializers copy these verbatim into .strtab/.shstrtab.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::string_table_suffix_sharing.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// CEP:WHAT: Current table size in bytes.
    /// CEP:WHY: Offset/sizing arithmetic in serializers.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used by write_elf64_relocatable cursor math.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// CEP:WHAT: True when the table holds only the initial NUL.
    /// CEP:WHY: Clippy len_without_is_empty companion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: none (trivial).
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Default for StringTable {
    /// CEP:WHAT: Default is an empty (NUL-only) table.
    /// CEP:WHY: Idiomatic Default alongside new().
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: none (trivial).
    fn default() -> Self {
        StringTable::new()
    }
}

/// CEP:WHAT: The ".rela" section name for an input section name.
/// CEP:WHY: ELF convention: ".rela" + name for dotted names ("text" -> ".rela.text").
/// CEP:STATUS: complete
/// CEP:FAILURE: none (callers validate names).
/// CEP:ASSUMES: names were check_name-validated.
/// CEP:COST: O(name).
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::shstrtab_suffix_sharing_in_object.
pub(crate) fn rela_section_name(section_name: &str) -> String {
    if section_name.starts_with('.') {
        format!(".rela{section_name}")
    } else {
        format!(".rela.{section_name}")
    }
}

/// CEP:WHAT: The fixed 64-byte fields of an ELF64 file header, in emission order.
/// CEP:WHY: Both writers (ET_REL here, ET_EXEC in link.rs) share one encoder so identity
///   fields cannot drift between the two serializers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (fixed-size array, infallible)
/// CEP:ASSUMES: little-endian target.
/// CEP:COST: O(1) copies into a stack array.
/// CEP:EVIDENCE: parsed back by unit tests rv8_object::elf::tests and the acceptance test.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ehdr64 {
    pub e_type: u16,
    pub e_machine: u16,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

impl Ehdr64 {
    /// CEP:WHAT: Serializes the header with fixed e_ident (64-bit, LSB, SYSV, version 1).
    /// CEP:WHY: Single source of truth for the first 64 bytes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
    pub(crate) fn to_bytes(self) -> [u8; EHDR_BYTES] {
        let mut b = [0u8; EHDR_BYTES];
        b[0..4].copy_from_slice(&ELF_MAGIC);
        b[4] = ELFCLASS64;
        b[5] = ELFDATA2LSB;
        b[6] = EV_CURRENT;
        b[16..18].copy_from_slice(&self.e_type.to_le_bytes());
        b[18..20].copy_from_slice(&self.e_machine.to_le_bytes());
        b[20..24].copy_from_slice(&1u32.to_le_bytes());
        b[24..32].copy_from_slice(&self.e_entry.to_le_bytes());
        b[32..40].copy_from_slice(&self.e_phoff.to_le_bytes());
        b[40..48].copy_from_slice(&self.e_shoff.to_le_bytes());
        b[48..52].copy_from_slice(&self.e_flags.to_le_bytes());
        b[52..54].copy_from_slice(&(EHDR_BYTES as u16).to_le_bytes());
        b[54..56].copy_from_slice(&self.e_phentsize.to_le_bytes());
        b[56..58].copy_from_slice(&self.e_phnum.to_le_bytes());
        b[58..60].copy_from_slice(&(SHDR_BYTES as u16).to_le_bytes());
        b[60..62].copy_from_slice(&self.e_shnum.to_le_bytes());
        b[62..64].copy_from_slice(&self.e_shstrndx.to_le_bytes());
        b
    }
}

/// CEP:WHAT: The 56-byte fields of an ELF64 program header.
/// CEP:WHY: link.rs emits PT_LOAD segments from these.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (fixed-size array, infallible)
/// CEP:ASSUMES: little-endian target.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: end-to-end acceptance test runs the emitted executable.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Phdr64 {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl Phdr64 {
    /// CEP:WHAT: Serializes one program header.
    /// CEP:WHY: Shared encoder for all PT_LOAD entries.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: end-to-end acceptance test.
    pub(crate) fn to_bytes(self) -> [u8; PHDR_BYTES] {
        let mut b = [0u8; PHDR_BYTES];
        b[0..4].copy_from_slice(&self.p_type.to_le_bytes());
        b[4..8].copy_from_slice(&self.p_flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.p_offset.to_le_bytes());
        b[16..24].copy_from_slice(&self.p_vaddr.to_le_bytes());
        b[24..32].copy_from_slice(&self.p_paddr.to_le_bytes());
        b[32..40].copy_from_slice(&self.p_filesz.to_le_bytes());
        b[40..48].copy_from_slice(&self.p_memsz.to_le_bytes());
        b[48..56].copy_from_slice(&self.p_align.to_le_bytes());
        b
    }
}

/// CEP:WHAT: The 64-byte fields of an ELF64 section header.
/// CEP:WHY: Shared encoder for both writers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (fixed-size array, infallible)
/// CEP:ASSUMES: little-endian target.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit tests parse these back from emitted images.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Shdr64 {
    pub sh_name: u32,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub sh_addr: u64,
    pub sh_offset: u64,
    pub sh_size: u64,
    pub sh_link: u32,
    pub sh_info: u32,
    pub sh_addralign: u64,
    pub sh_entsize: u64,
}

impl Shdr64 {
    /// CEP:WHAT: Serializes one section header.
    /// CEP:WHY: Shared encoder for all section headers.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
    pub(crate) fn to_bytes(self) -> [u8; SHDR_BYTES] {
        let mut b = [0u8; SHDR_BYTES];
        b[0..4].copy_from_slice(&self.sh_name.to_le_bytes());
        b[4..8].copy_from_slice(&self.sh_type.to_le_bytes());
        b[8..16].copy_from_slice(&self.sh_flags.to_le_bytes());
        b[16..24].copy_from_slice(&self.sh_addr.to_le_bytes());
        b[24..32].copy_from_slice(&self.sh_offset.to_le_bytes());
        b[32..40].copy_from_slice(&self.sh_size.to_le_bytes());
        b[40..44].copy_from_slice(&self.sh_link.to_le_bytes());
        b[44..48].copy_from_slice(&self.sh_info.to_le_bytes());
        b[48..56].copy_from_slice(&self.sh_addralign.to_le_bytes());
        b[56..64].copy_from_slice(&self.sh_entsize.to_le_bytes());
        b
    }
}

/// CEP:WHAT: The 24-byte fields of an Elf64_Sym.
/// CEP:WHY: The .symtab encoder packs model symbols into these.
/// CEP:STATUS: complete
/// CEP:FAILURE: none (fixed-size array, infallible)
/// CEP:ASSUMES: little-endian target.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sym64 {
    pub st_name: u32,
    pub st_info: u8,
    pub st_other: u8,
    pub st_shndx: u16,
    pub st_value: u64,
    pub st_size: u64,
}

impl Sym64 {
    /// CEP:WHAT: Serializes one symbol table entry.
    /// CEP:WHY: Shared layout for locals and globals.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
    pub(crate) fn to_bytes(self) -> [u8; SYM_BYTES] {
        let mut b = [0u8; SYM_BYTES];
        b[0..4].copy_from_slice(&self.st_name.to_le_bytes());
        b[4] = self.st_info;
        b[5] = self.st_other;
        b[6..8].copy_from_slice(&self.st_shndx.to_le_bytes());
        b[8..16].copy_from_slice(&self.st_value.to_le_bytes());
        b[16..24].copy_from_slice(&self.st_size.to_le_bytes());
        b
    }
}

/// CEP:WHAT: The 24-byte fields of an Elf64_Rela.
/// CEP:WHY: RELA entries carry the addend explicitly (no implicit addend hacks).
/// CEP:STATUS: complete
/// CEP:FAILURE: none (fixed-size array, infallible)
/// CEP:ASSUMES: little-endian target.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rela64 {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

impl Rela64 {
    /// CEP:WHAT: Serializes one RELA entry; r_info = (symbol index << 32) | type.
    /// CEP:WHY: ELF64_R_INFO contract from the pinned RelocKind mapping.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: symbol index fits 32 bits (MAX_SYMBOLS guarantees it).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
    pub(crate) fn to_bytes(self) -> [u8; RELA_BYTES] {
        let mut b = [0u8; RELA_BYTES];
        b[0..8].copy_from_slice(&self.r_offset.to_le_bytes());
        b[8..16].copy_from_slice(&self.r_info.to_le_bytes());
        b[16..24].copy_from_slice(&self.r_addend.to_le_bytes());
        b
    }
}

/// CEP:WHAT: ELF64_R_INFO helper.
/// CEP:WHY: Keeps the (sym << 32) | type packing in exactly one place.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: sym fits u32.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by both writers.
pub(crate) fn elf64_r_info(sym: u32, rtype: u32) -> u64 {
    ((sym as u64) << 32) | rtype as u64
}

/// CEP:WHAT: Bounds-checked byte-range write into an output image.
/// CEP:WHY: Standard hard rule: a truncated or misplaced write is Err, never a panic.
/// CEP:STATUS: complete
/// CEP:FAILURE: WriteOutOfBounds / ArithmeticOverflow.
/// CEP:ASSUMES: none
/// CEP:COST: O(n) copy after O(1) check.
/// CEP:EVIDENCE: used for every byte the two writers emit.
pub(crate) fn write_bytes_at(buf: &mut [u8], offset: usize, data: &[u8]) -> Result<(), ObjectError> {
    let end = offset
        .checked_add(data.len())
        .ok_or(ObjectError::ArithmeticOverflow { context: "write_bytes_at" })?;
    if end > buf.len() {
        return Err(ObjectError::WriteOutOfBounds {
            offset,
            len: buf.len(),
            need: data.len(),
        });
    }
    buf[offset..end].copy_from_slice(data);
    Ok(())
}

/// CEP:WHAT: Bounds-checked u32 little-endian write.
/// CEP:WHY: Field-level patching in emitted images.
/// CEP:STATUS: complete
/// CEP:FAILURE: WriteOutOfBounds.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: used by link.rs field writes.
pub(crate) fn write_u32_at(buf: &mut [u8], offset: usize, v: u32) -> Result<(), ObjectError> {
    write_bytes_at(buf, offset, &v.to_le_bytes())
}

/// CEP:WHAT: Bounds-checked u64 little-endian write.
/// CEP:WHY: Abs64 relocation application.
/// CEP:STATUS: complete
/// CEP:FAILURE: WriteOutOfBounds.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: link.rs applies Abs64 relocations through this.
pub(crate) fn write_u64_at(buf: &mut [u8], offset: usize, v: u64) -> Result<(), ObjectError> {
    write_bytes_at(buf, offset, &v.to_le_bytes())
}

/// CEP:WHAT: Bounds-checked i32 little-endian write.
/// CEP:WHY: PcRel32 relocation application.
/// CEP:STATUS: complete
/// CEP:FAILURE: WriteOutOfBounds.
/// CEP:ASSUMES: value already range-checked by the caller.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: link.rs applies PcRel32 relocations through this.
pub(crate) fn write_i32_at(buf: &mut [u8], offset: usize, v: i32) -> Result<(), ObjectError> {
    write_u32_at(buf, offset, v as u32)
}

/// CEP:WHAT: Derives st_info symbol type from the model: FUNC in exec sections, OBJECT in
///   allocated data or NOBITS, NOTYPE otherwise (including undefined references).
/// CEP:WHY: The pinned Symbol struct carries no type field; the section anchor is the
///   deterministic source of truth for a useful st_info byte.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: FLAG_EXEC/FLAG_ALLOC and SectionType carry the needed semantics.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_object::elf::tests::relocatable_layout_and_symtab.
fn symbol_stt(obj: &ObjectFile, sym: &Symbol) -> u8 {
    if !sym.defined {
        return STT_NOTYPE;
    }
    let Some(sec) = sym.section.map(|i| &obj.sections[i]) else {
        return STT_NOTYPE;
    };
    if sec.ty == SectionType::NoBits {
        return STT_OBJECT;
    }
    if sec.is_exec() {
        return STT_FUNC;
    }
    if sec.flags & FLAG_ALLOC != 0 {
        return STT_OBJECT;
    }
    STT_NOTYPE
}

/// CEP:WHAT: Emits a complete ELF64 relocatable object (ET_REL, EM_X86_64) from the
///   universal model, generating .symtab (locals first, sh_info = first global), .strtab,
///   .rela.X (SHT_RELA with ELF64_R_INFO(sym, RelocKind::elf_type())) and .shstrtab with
///   suffix sharing, over sections laid out by permission group then descending alignment.
/// CEP:WHY: The compiler's object-file product; byte-determinism is the acceptance bar
///   (two runs over the same input emit identical bytes).
/// CEP:STATUS: complete
/// CEP:FAILURE: validate() rejections, UnknownRelocSymbol, InvalidName, BudgetExceeded,
///   ArithmeticOverflow, WriteOutOfBounds; never panics.
/// CEP:ASSUMES: little-endian ELF64 x86-64 consumer (readelf/ld compatible layout);
///   relocations resolve against symbols of the same object; NOBITS sections emit size but
///   no file bytes.
/// CEP:COST: O(S log S + R + Y + emitted bytes); exactly one output allocation.
/// CEP:EVIDENCE: unit tests rv8_object::elf::tests parse the emitted image back; the
///   acceptance suite asserts byte equality across runs.
pub fn write_elf64_relocatable(obj: &ObjectFile) -> Result<Vec<u8>, ObjectError> {
    obj.validate()?;

    let n = obj.sections.len();
    let order = layout_order(&obj.sections);

    // Output slot bookkeeping: slot 0 is the mandatory null section header.
    let mut out_of_input = vec![0usize; n];
    for (pos, &orig) in order.iter().enumerate() {
        out_of_input[orig] = pos + 1;
    }
    let mut rela_slot_of_input = vec![usize::MAX; n];
    let mut n_rela = 0usize;
    for &orig in &order {
        if !obj.sections[orig].relocs.is_empty() {
            rela_slot_of_input[orig] = n + 1 + n_rela;
            n_rela += 1;
        }
    }
    let symtab_slot = n + 1 + n_rela;
    let strtab_slot = symtab_slot + 1;
    let shstrtab_slot = symtab_slot + 2;
    let shnum = shstrtab_slot + 1;
    if shnum > MAX_OUTPUT_SECTIONS {
        return Err(ObjectError::BudgetExceeded {
            what: "MAX_OUTPUT_SECTIONS",
            limit: MAX_OUTPUT_SECTIONS as u64,
            actual: shnum as u64,
        });
    }

    // Symbol partition: ELF requires locals before globals; the sort is stable, so
    // model order survives inside each bind class (deterministic).
    let mut sym_order: Vec<usize> = (0..obj.symbols.len()).collect();
    sym_order.sort_by_key(|&i| obj.symbols[i].global);
    let mut sym_out_index = vec![0u32; obj.symbols.len()];
    for (pos, &mi) in sym_order.iter().enumerate() {
        sym_out_index[mi] = (pos + 1) as u32;
    }
    let n_local = sym_order.iter().filter(|&&m| !obj.symbols[m].global).count();

    // .strtab for symbols (suffix-shared).
    let mut strtab = StringTable::new();
    let mut sym_name_off = vec![0u32; obj.symbols.len()];
    for &mi in &sym_order {
        sym_name_off[mi] = strtab.add(&obj.symbols[mi].name)?;
    }

    // .shstrtab: rela + generated names first, then input names, so dotted input names
    // suffix-share inside their ".rela.X" entries (".text" inside ".rela.text").
    let mut shstr = StringTable::new();
    let mut rela_name_off = vec![0u32; n];
    for &orig in &order {
        if rela_slot_of_input[orig] != usize::MAX {
            rela_name_off[orig] = shstr.add(&rela_section_name(&obj.sections[orig].name))?;
        }
    }
    let symtab_name = shstr.add(".symtab")?;
    let strtab_name = shstr.add(".strtab")?;
    let shstrtab_name = shstr.add(".shstrtab")?;
    let mut input_name_off = vec![0u32; n];
    for &orig in &order {
        input_name_off[orig] = shstr.add(&obj.sections[orig].name)?;
    }

    // First-occurrence symbol index per name for relocation resolution.
    let mut first_sym: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, s) in obj.symbols.iter().enumerate() {
        first_sym.entry(s.name.as_str()).or_insert(i);
    }

    // File cursor plan: section data (permission/alignment order), rela tables,
    // symtab, strtab, shstrtab, then the section header table. All arithmetic checked.
    let mut c: u64 = EHDR_BYTES as u64;
    let mut input_off = vec![0u64; n];
    for &orig in &order {
        let sec = &obj.sections[orig];
        c = align_up(c, sec.align)?;
        input_off[orig] = c;
        if sec.ty == SectionType::Progbits {
            c = c
                .checked_add(sec.bytes.len() as u64)
                .ok_or(ObjectError::ArithmeticOverflow { context: "section cursor" })?;
        }
    }
    let mut rela_off = vec![0u64; n];
    for &orig in &order {
        if rela_slot_of_input[orig] != usize::MAX {
            c = align_up(c, 8)?;
            rela_off[orig] = c;
            let bytes = (obj.sections[orig].relocs.len() as u64)
                .checked_mul(RELA_BYTES as u64)
                .ok_or(ObjectError::ArithmeticOverflow { context: "rela table size" })?;
            c = c
                .checked_add(bytes)
                .ok_or(ObjectError::ArithmeticOverflow { context: "rela cursor" })?;
        }
    }
    let symtab_off = align_up(c, 8)?;
    c = symtab_off
        .checked_add(
            (obj.symbols.len() as u64 + 1)
                .checked_mul(SYM_BYTES as u64)
                .ok_or(ObjectError::ArithmeticOverflow { context: "symtab size" })?,
        )
        .ok_or(ObjectError::ArithmeticOverflow { context: "symtab cursor" })?;
    let strtab_off = c;
    c = c
        .checked_add(strtab.len() as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "strtab cursor" })?;
    let shstrtab_off = c;
    c = c
        .checked_add(shstr.len() as u64)
        .ok_or(ObjectError::ArithmeticOverflow { context: "shstrtab cursor" })?;
    let shoff = align_up(c, 8)?;
    let total = shoff
        .checked_add(
            (shnum as u64)
                .checked_mul(SHDR_BYTES as u64)
                .ok_or(ObjectError::ArithmeticOverflow { context: "shdr table size" })?,
        )
        .ok_or(ObjectError::ArithmeticOverflow { context: "total image size" })?;

    let mut image = vec![0u8; usize::try_from(total)
        .map_err(|_| ObjectError::ArithmeticOverflow { context: "image allocation size" })?];

    let ehdr = Ehdr64 {
        e_type: ET_REL,
        e_machine: EM_X86_64,
        e_entry: 0,
        e_phoff: 0,
        e_shoff: shoff,
        e_flags: 0,
        e_phentsize: 0,
        e_phnum: 0,
        e_shnum: shnum as u16,
        e_shstrndx: shstrtab_slot as u16,
    };
    write_bytes_at(&mut image, 0, &ehdr.to_bytes())?;

    // Section contents.
    for &orig in &order {
        let sec = &obj.sections[orig];
        if sec.ty == SectionType::Progbits && !sec.bytes.is_empty() {
            write_bytes_at(
                &mut image,
                usize::try_from(input_off[orig])
                    .map_err(|_| ObjectError::ArithmeticOverflow { context: "section offset" })?,
                &sec.bytes,
            )?;
        }
    }

    // RELA tables.
    for &orig in &order {
        if rela_slot_of_input[orig] == usize::MAX {
            continue;
        }
        let sec = &obj.sections[orig];
        let mut table = Vec::with_capacity(sec.relocs.len() * RELA_BYTES);
        for r in &sec.relocs {
            let si = first_sym
                .get(r.symbol.as_str())
                .copied()
                .ok_or_else(|| ObjectError::UnknownRelocSymbol { name: r.symbol.clone() })?;
            table.extend_from_slice(
                &Rela64 {
                    r_offset: r.offset,
                    r_info: elf64_r_info(sym_out_index[si], r.kind.elf_type()),
                    r_addend: r.addend,
                }
                .to_bytes(),
            );
        }
        write_bytes_at(
            &mut image,
            usize::try_from(rela_off[orig])
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "rela offset" })?,
            &table,
        )?;
    }

    // .symtab: entry 0 is the null symbol, then locals, then globals.
    let mut symtab = Vec::with_capacity((obj.symbols.len() + 1) * SYM_BYTES);
    symtab.extend_from_slice(&[0u8; SYM_BYTES]);
    for &mi in &sym_order {
        let s = &obj.symbols[mi];
        let (shndx, value, size) = if s.defined {
            (
                out_of_input[s.section.unwrap_or(0)] as u16,
                s.value,
                s.size,
            )
        } else {
            (SHN_UNDEF, 0, 0)
        };
        let bind = if s.global { STB_GLOBAL } else { STB_LOCAL };
        symtab.extend_from_slice(
            &Sym64 {
                st_name: sym_name_off[mi],
                st_info: (bind << 4) | symbol_stt(obj, s),
                st_other: 0,
                st_shndx: shndx,
                st_value: value,
                st_size: size,
            }
            .to_bytes(),
        );
    }
    write_bytes_at(
        &mut image,
        usize::try_from(symtab_off)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "symtab offset" })?,
        &symtab,
    )?;
    write_bytes_at(
        &mut image,
        usize::try_from(strtab_off)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "strtab offset" })?,
        strtab.as_bytes(),
    )?;
    write_bytes_at(
        &mut image,
        usize::try_from(shstrtab_off)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "shstrtab offset" })?,
        shstr.as_bytes(),
    )?;

    // Section header table, slot by slot.
    let mut shdrs: Vec<[u8; SHDR_BYTES]> = Vec::with_capacity(shnum);
    shdrs.push([0u8; SHDR_BYTES]); // null section
    for &orig in &order {
        let sec = &obj.sections[orig];
        shdrs.push(
            Shdr64 {
                sh_name: input_name_off[orig],
                sh_type: if sec.ty == SectionType::NoBits { SHT_NOBITS } else { SHT_PROGBITS },
                sh_flags: sec.flags,
                sh_addr: 0,
                sh_offset: input_off[orig],
                sh_size: sec.bytes.len() as u64,
                sh_link: 0,
                sh_info: 0,
                sh_addralign: sec.align,
                sh_entsize: 0,
            }
            .to_bytes(),
        );
    }
    for &orig in &order {
        if rela_slot_of_input[orig] == usize::MAX {
            continue;
        }
        shdrs.push(
            Shdr64 {
                sh_name: rela_name_off[orig],
                sh_type: SHT_RELA,
                sh_flags: 0,
                sh_addr: 0,
                sh_offset: rela_off[orig],
                sh_size: (obj.sections[orig].relocs.len() as u64) * RELA_BYTES as u64,
                sh_link: symtab_slot as u32,
                sh_info: out_of_input[orig] as u32,
                sh_addralign: 8,
                sh_entsize: RELA_BYTES as u64,
            }
            .to_bytes(),
        );
    }
    shdrs.push(
        Shdr64 {
            sh_name: symtab_name,
            sh_type: SHT_SYMTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: symtab_off,
            sh_size: ((obj.symbols.len() + 1) * SYM_BYTES) as u64,
            sh_link: strtab_slot as u32,
            sh_info: n_local as u32,
            sh_addralign: 8,
            sh_entsize: SYM_BYTES as u64,
        }
        .to_bytes(),
    );
    shdrs.push(
        Shdr64 {
            sh_name: strtab_name,
            sh_type: SHT_STRTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: strtab_off,
            sh_size: strtab.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: 1,
            sh_entsize: 0,
        }
        .to_bytes(),
    );
    shdrs.push(
        Shdr64 {
            sh_name: shstrtab_name,
            sh_type: SHT_STRTAB,
            sh_flags: 0,
            sh_addr: 0,
            sh_offset: shstrtab_off,
            sh_size: shstr.len() as u64,
            sh_link: 0,
            sh_info: 0,
            sh_addralign: 1,
            sh_entsize: 0,
        }
        .to_bytes(),
    );
    for (i, sh) in shdrs.iter().enumerate() {
        write_bytes_at(&mut image, shoff as usize + i * SHDR_BYTES, sh)?;
    }

    debug_assert_eq!(shdrs.len(), shnum);
    Ok(image)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::model::RelocKind;

    fn rd_u16(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
    }
    fn rd_u32(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
    }
    fn rd_u64(b: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
    }
    fn rd_i64(b: &[u8], off: usize) -> i64 {
        i64::from_le_bytes(b[off..off + 8].try_into().unwrap())
    }

    struct RawShdr {
        name: u32,
        ty: u32,
        flags: u64,
        offset: u64,
        size: u64,
        link: u32,
        info: u32,
        align: u64,
        entsize: u64,
    }

    fn read_shdrs(buf: &[u8]) -> Vec<RawShdr> {
        let shoff = rd_u64(buf, 0x28) as usize;
        let shnum = rd_u16(buf, 0x3c) as usize;
        (0..shnum)
            .map(|i| {
                let b = &buf[shoff + i * SHDR_BYTES..shoff + (i + 1) * SHDR_BYTES];
                RawShdr {
                    name: rd_u32(b, 0),
                    ty: rd_u32(b, 4),
                    flags: rd_u64(b, 8),
                    offset: rd_u64(b, 24),
                    size: rd_u64(b, 32),
                    link: rd_u32(b, 40),
                    info: rd_u32(b, 44),
                    align: rd_u64(b, 48),
                    entsize: rd_u64(b, 56),
                }
            })
            .collect()
    }

    /// CEP:WHAT: Exact-duplicate reuse and suffix sharing in StringTable, including the
    ///   "main" / "add_main" / "ain" / "_main" family required by the acceptance list.
    /// CEP:WHY: STANDARDS 5.5 names suffix sharing as a required feature.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn string_table_suffix_sharing() {
        let mut st = StringTable::new();
        assert_eq!(st.as_bytes(), b"\0");
        assert_eq!(st.add("main").unwrap(), 1);
        let add_main = st.add("add_main").unwrap();
        assert_eq!(add_main, 6); // appended, no sharing available yet
        assert_eq!(st.add("main").unwrap(), 1); // exact duplicate reuse
        assert_eq!(st.add("ain").unwrap(), 2); // suffix of "main"
        assert_eq!(st.add("_main").unwrap(), add_main + 3); // suffix of "add_main"
        assert_eq!(st.add("").unwrap(), 0); // empty string is offset 0
        assert_eq!(st.as_bytes(), b"\0main\0add_main\0");
    }

    /// CEP:WHAT: MAX_STRING_TABLE_BYTES budget is enforced by StringTable::add.
    /// CEP:WHY: Named budget with fail-closed behavior.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap on mismatch (test-only).
    /// CEP:ASSUMES: names of ~4 KiB distinct suffixes prevent sharing.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn string_table_budget() {
        let mut st = StringTable::new();
        let mut i = 0u32;
        loop {
            let name = format!("{:0>4000}", i);
            match st.add(&name) {
                Ok(_) => i += 1,
                Err(ObjectError::BudgetExceeded { .. }) => break,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(i > 0);
        assert!(st.len() <= MAX_STRING_TABLE_BYTES);
    }

    /// CEP:WHAT: Parses an emitted ET_REL image back and checks ehdr fields, section
    ///   layout order, .symtab content (bind/type/shndx/value), and .rela entries.
    /// CEP:WHY: Binary-layout evidence for the writer contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn relocatable_layout_and_symtab() {
        let mut o = ObjectFile::new("t.o");
        // call rel32; ret; then 8 bytes of pointer slot for an Abs64.
        let text = o
            .add_text_section(".text", 16, vec![0xe8, 0, 0, 0, 0, 0xc3, 0, 0, 0, 0, 0, 0, 0, 0])
            .unwrap();
        let data = o.add_data_section(".data", 8, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        o.add_symbol("l0", true, false, Some(text), 0, 1).unwrap();
        o.add_symbol("start", true, true, Some(text), 0, 6).unwrap();
        o.add_symbol("ext", false, true, None, 0, 0).unwrap();
        o.add_symbol("data_sym", true, true, Some(data), 0, 8).unwrap();
        o.add_reloc(text, 1, RelocKind::PcRel32, "ext", -4).unwrap();
        o.add_reloc(text, 6, RelocKind::Abs64, "data_sym", 0).unwrap();

        let img = write_elf64_relocatable(&o).unwrap();
        assert_eq!(&img[0..4], &ELF_MAGIC);
        assert_eq!(rd_u16(&img, 16), ET_REL);
        assert_eq!(rd_u16(&img, 18), EM_X86_64);
        assert_eq!(rd_u16(&img, 56), 0); // e_phnum
        let shdrs = read_shdrs(&img);
        // null, .text, .data (RX before RW), .rela.text, .symtab, .strtab, .shstrtab
        assert_eq!(shdrs.len(), 7);
        assert_eq!(shdrs[1].ty, SHT_PROGBITS);
        assert_eq!(shdrs[1].flags, crate::model::FLAG_ALLOC | crate::model::FLAG_EXEC);
        assert_eq!(shdrs[1].align, 16);
        assert_eq!(shdrs[2].ty, SHT_PROGBITS);
        assert_eq!(shdrs[3].ty, SHT_RELA);
        assert_eq!(shdrs[4].ty, SHT_SYMTAB);
        assert_eq!(shdrs[5].ty, SHT_STRTAB);
        assert_eq!(shdrs[6].ty, SHT_STRTAB);
        assert_eq!(rd_u16(&img, 62), 6); // e_shstrndx

        // .symtab: null, l0 (local FUNC), start (global FUNC), ext (global NOTYPE, undef),
        // data_sym (global OBJECT in .data).
        let sym_off = shdrs[4].offset as usize;
        let sym = |i: usize| &img[sym_off + i * SYM_BYTES..sym_off + (i + 1) * SYM_BYTES];
        assert_eq!(sym(0), &[0u8; SYM_BYTES]);
        assert_eq!(sym(1)[4], STB_LOCAL << 4 | STT_FUNC);
        assert_eq!(rd_u16(sym(1), 6), 1); // st_shndx -> .text
        assert_eq!(sym(2)[4], STB_GLOBAL << 4 | STT_FUNC);
        assert_eq!(rd_u16(sym(2), 6), 1);
        assert_eq!(sym(3)[4], STB_GLOBAL << 4 | STT_NOTYPE);
        assert_eq!(rd_u16(sym(3), 6), SHN_UNDEF); // undefined
        assert_eq!(sym(4)[4], STB_GLOBAL << 4 | STT_OBJECT);
        assert_eq!(rd_u16(sym(4), 6), 2); // .data
        assert_eq!(rd_u64(sym(4), 16), 8); // st_size
        assert_eq!(shdrs[4].info, 1); // sh_info: first global index (after null + l0)
        assert_eq!(shdrs[4].link, 5); // -> .strtab
        assert_eq!(shdrs[4].entsize, SYM_BYTES as u64);

        // .rela.text: sh_link = symtab slot (4), sh_info = .text slot (1).
        assert_eq!(shdrs[3].link, 4);
        assert_eq!(shdrs[3].info, 1);
        assert_eq!(shdrs[3].entsize, RELA_BYTES as u64);
        let rela_off = shdrs[3].offset as usize;
        let r0 = &img[rela_off..rela_off + RELA_BYTES];
        let r1 = &img[rela_off + RELA_BYTES..rela_off + 2 * RELA_BYTES];
        assert_eq!(rd_u64(r0, 0), 1); // r_offset
        assert_eq!(rd_u64(r0, 8), elf64_r_info(3, 2)); // "ext", PcRel32
        assert_eq!(rd_i64(r0, 16), -4); // addend
        assert_eq!(rd_u64(r1, 0), 6);
        assert_eq!(rd_u64(r1, 8), elf64_r_info(4, 1)); // "data_sym", Abs64
        assert_eq!(rd_i64(r1, 16), 0);
    }

    /// CEP:WHAT: The emitted .shstrtab suffix-shares ".text" inside ".rela.text".
    /// CEP:WHY: Proves suffix sharing reaches real emitted bytes, not only the unit type.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: emission adds rela names before input names (documented order).
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn shstrtab_suffix_sharing_in_object() {
        let mut o = ObjectFile::new("share.o");
        let text = o.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0]).unwrap();
        o.add_symbol("ext", false, true, None, 0, 0).unwrap();
        o.add_reloc(text, 1, RelocKind::PcRel32, "ext", -4).unwrap();
        let img = write_elf64_relocatable(&o).unwrap();
        let shdrs = read_shdrs(&img);
        // slots: 0 null, 1 .text, 2 .rela.text, 3 .symtab, 4 .strtab, 5 .shstrtab
        assert_eq!(shdrs.len(), 6);
        let shstr_off = shdrs[5].offset as usize;
        let shstr_len = shdrs[5].size as usize;
        let shstr = &img[shstr_off..shstr_off + shstr_len];
        assert_eq!(shstr, b"\0.rela.text\0.symtab\0.strtab\0.shstrtab\0");
        assert_eq!(shdrs[2].name, 1); // ".rela.text"
        assert_eq!(shdrs[1].name, 6); // ".text" shares the suffix of ".rela.text"
    }

    /// CEP:WHAT: A relocation naming a symbol absent from its object fails closed.
    /// CEP:WHY: ET_REL files are self-contained; silent zero-fill would corrupt links.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn unknown_reloc_symbol_rejected() {
        let mut o = ObjectFile::new("ghost.o");
        let text = o.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0]).unwrap();
        o.add_reloc(text, 1, RelocKind::PcRel32, "ghost", -4).unwrap();
        assert_eq!(
            write_elf64_relocatable(&o),
            Err(ObjectError::UnknownRelocSymbol { name: "ghost".into() })
        );
    }

    /// CEP:WHAT: NOBITS sections emit sh_size with no file bytes.
    /// CEP:WHY: .bss-style reservations must not grow the file.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn nobits_has_no_file_bytes() {
        let mut o = ObjectFile::new("bss.o");
        o.add_nobits_section(".bss", 64, 128).unwrap();
        let img = write_elf64_relocatable(&o).unwrap();
        let shdrs = read_shdrs(&img);
        assert_eq!(shdrs[1].ty, SHT_NOBITS);
        assert_eq!(shdrs[1].size, 128);
        assert_eq!(shdrs[1].align, 64);
        // File stays small: header + shdr table + shstrtab only.
        assert!(img.len() < 1024);
    }

    /// CEP:WHAT: Two emissions of the same object are byte-identical.
    /// CEP:WHY: Local re-check of determinism next to the writer.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test; the full determinism matrix lives in tests/acceptance.rs.
    #[test]
    fn relocatable_emission_is_deterministic() {
        let mut o = ObjectFile::new("det.o");
        let t = o
            .add_text_section(".text", 16, vec![0x8d, 0x04, 0x37, 0xc3])
            .unwrap();
        o.add_symbol("add", true, true, Some(t), 0, 4).unwrap();
        let a = write_elf64_relocatable(&o).unwrap();
        let b = write_elf64_relocatable(&o).unwrap();
        assert_eq!(a, b);
    }
}
