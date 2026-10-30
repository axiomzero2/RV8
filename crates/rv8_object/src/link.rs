// CEP:FILE: crates/rv8_object/src/link.rs
// CEP:WHAT: The static executable linker: cross-object symbol resolution (strong/weak
//   lattice), section GC by reachability over relocation edges, page-granular RX/RW
//   segment layout with deterministic alignment-first ordering, relocation application
//   (PcRel32: S+A-P in i32; Abs64: S+A), the ET_EXEC writer with PT_LOAD program headers
//   plus section headers for debuggability, and the _start synthesizer.
// CEP:WHY: STANDARDS 5.5 acceptance is an executable linked from universal objects that
//   runs and exits with a known code; every stage here is deterministic by construction
//   (input order, stable sorts, BTreeMap-only name maps).
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: duplicates strong globals, unresolved entry symbols, undefined symbols in
//   kept sections, out-of-range relocations, budget and arithmetic overflows, and bounded
//   write failures all return ObjectError; no panics, no recursion (GC is a worklist).
// CEP:ASSUMES: the pinned model carries no `weak` bit in this revision, so all defined
//   globals merge as strong; the weak/strong lattice is implemented in merge_def and
//   unit-tested so a future model field plugs in without touching resolution. Relocations
//   reference global names, with same-object local definitions shadowing globals (ELF
//   semantics). base_vaddr must be page-aligned. Sections are kept distinct (no merge by
//   name), which preserves alignment deterministically.
// CEP:COST: resolution O(Y log Y); GC O(S + R) flat-section worklist with O(log Y) name
//   lookups; layout O(S log S) stable sorts; emission O(image bytes); one allocation.
// CEP:EVIDENCE: acceptance test links add/main objects plus synthesize_start and runs the
//   result to exit code 13; unit tests pin _start bytes, the merge lattice, GC behavior,
//   alignment, and the PcRel32 range check.

use std::collections::{btree_map::Entry, BTreeMap, VecDeque};

use crate::elf::{
    write_bytes_at, write_i32_at, write_u64_at, Ehdr64, ELF_PAGE_BYTES, EHDR_BYTES, EM_X86_64,
    ET_EXEC, MAX_OUTPUT_SECTIONS, PF_R, PF_W, PF_X, PHDR_BYTES, Phdr64, PT_LOAD, SHDR_BYTES,
    Shdr64, SHT_NOBITS, SHT_PROGBITS, SHT_STRTAB, StringTable,
};
use crate::error::ObjectError;
use crate::model::{
    align_up, ObjectFile, Reloc, RelocKind, Section, SectionType, FLAG_ALLOC, FLAG_EXEC,
    MAX_INPUT_OBJECTS,
};

/// CEP:WHAT: Default load base for linked executables.
/// CEP:WHY: The canonical non-PIE x86-64 Linux base; keeps images out of the zero page.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: callers may override; overrides must stay page-aligned.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: acceptance test links at 0x400000 and execs the result.
pub const DEFAULT_BASE_VADDR: u64 = 0x0040_0000;

/// CEP:WHAT: One laid-out (kept) section in emission order with its final file offset and
///   virtual address.
/// CEP:WHY: plan_executable exposes the layout for driver observability and for GC tests;
///   the emitter consumes the same records.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: offset/vaddr are final; vaddr includes the image base.
/// CEP:COST: plain data.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::gc_plan_keeps_and_drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSection {
    /// Input object index.
    pub object: usize,
    /// Section index within that object.
    pub section: usize,
    /// Section name.
    pub name: String,
    /// Model flags (ELF SHF_*-compatible).
    pub flags: u64,
    /// Alignment in bytes (0 means 1).
    pub align: u64,
    /// File offset of the contents.
    pub offset: u64,
    /// Virtual address (base included).
    pub vaddr: u64,
    /// Size in bytes (memory size for NOBITS).
    pub size: u64,
    /// False for NOBITS (no file bytes).
    pub file_backed: bool,
}

/// CEP:WHAT: One emitted PT_LOAD segment.
/// CEP:WHY: Program headers are part of the deterministic image contract.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: p_vaddr - p_offset equals the page-aligned image base.
/// CEP:COST: plain data.
/// CEP:EVIDENCE: acceptance-test executable loads and runs on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSegment {
    /// Combined PF_* flags.
    pub flags: u32,
    /// File offset of the segment start.
    pub offset: u64,
    /// Virtual address of the segment start.
    pub vaddr: u64,
    /// Bytes backed by the file.
    pub filesz: u64,
    /// Bytes occupied in memory (>= filesz when zero-fill follows).
    pub memsz: u64,
}

/// CEP:WHAT: The complete deterministic link plan: segments, kept sections in emission
///   order, entry address, and the end-of-content cursor.
/// CEP:WHY: Separating plan from emission lets the driver inspect layout and lets tests
///   observe GC without parsing binaries.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: plain data.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::gc_plan_keeps_and_drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlan {
    /// Image base used for the layout.
    pub base_vaddr: u64,
    /// Final virtual address of the entry symbol.
    pub entry: u64,
    /// PT_LOAD segments (RX first, then RW).
    pub segments: Vec<PlannedSegment>,
    /// Kept sections in emission order (RX, RW data, RW zero-fill).
    pub sections: Vec<PlannedSection>,
    /// File cursor just past the last section byte (shstrtab starts here, aligned).
    pub content_end: u64,
}

/// CEP:WHAT: Internal record of one winning symbol definition.
/// CEP:WHY: Resolution needs (object, symbol) coordinates plus strength for the lattice.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: symbol.section is Some (validated).
/// CEP:COST: plain data.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::merge_lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Def {
    pub obj: usize,
    pub sym: usize,
    pub strong: bool,
}

/// CEP:WHAT: Merges one definition into the global table under the strong/weak lattice:
///   strong beats weak, two strong definitions are an error, first definition wins among
///   equals.
/// CEP:WHY: The resolution rule of STANDARDS 5.5 in one auditable function.
/// CEP:STATUS: complete
/// CEP:FAILURE: DuplicateStrongSymbol for strong/strong collisions.
/// CEP:ASSUMES: the model revision has no weak bit (all inputs strong); the weak paths are
///   exercised here so a future field lands without touching resolution logic.
/// CEP:COST: O(log n) BTreeMap operations.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::merge_lattice.
pub(crate) fn merge_def(
    table: &mut BTreeMap<String, Def>,
    name: &str,
    def: Def,
) -> Result<(), ObjectError> {
    match table.entry(name.to_string()) {
        Entry::Vacant(v) => {
            v.insert(def);
            Ok(())
        }
        Entry::Occupied(mut o) => {
            let existing = o.get();
            if existing.strong && def.strong {
                return Err(ObjectError::DuplicateStrongSymbol { name: name.to_string() });
            }
            if def.strong && !existing.strong {
                o.insert(def); // strong wins over weak regardless of input order
            }
            Ok(()) // otherwise the existing entry stays (weak loses; first wins on ties)
        }
    }
}

/// CEP:WHAT: Builds the global definition table across all input objects in deterministic
///   order (object index, then symbol index).
/// CEP:WHY: A single pass over all symbols gives the linker its name -> definition map.
/// CEP:STATUS: complete
/// CEP:FAILURE: DuplicateStrongSymbol.
/// CEP:ASSUMES: objects were validated; defined globals carry a valid section index.
/// CEP:COST: O(Y log Y).
/// CEP:EVIDENCE: exercised by every link test.
pub(crate) fn resolve_globals(objs: &[ObjectFile]) -> Result<BTreeMap<String, Def>, ObjectError> {
    let mut table = BTreeMap::new();
    for (oi, o) in objs.iter().enumerate() {
        for (si, s) in o.symbols.iter().enumerate() {
            if s.global && s.defined {
                merge_def(&mut table, &s.name, Def { obj: oi, sym: si, strong: true })?;
            }
        }
    }
    Ok(table)
}

/// CEP:WHAT: Internal linker state: flat section id space, resolution tables, GC bitmap,
///   and assigned virtual addresses.
/// CEP:WHY: Keeps plan and emission consistent; flat ids make the GC worklist O(S + R).
/// CEP:STATUS: complete
/// CEP:FAILURE: construction fails on duplicate strong symbols.
/// CEP:ASSUMES: input objects validated by the entry points.
/// CEP:COST: O(Y log Y + S) build; no recursion anywhere.
/// CEP:EVIDENCE: all link tests go through this type.
struct Linker<'a> {
    objs: &'a [ObjectFile],
    /// `flat = base[obj] + section`; `base` has `objs.len() + 1` entries.
    base: Vec<usize>,
    total_sections: usize,
    globals: BTreeMap<String, Def>,
    /// Per object: first defined local symbol index per name (locals shadow globals).
    locals: Vec<BTreeMap<String, usize>>,
    kept: Vec<bool>,
    /// Virtual address of each flat section (0 when not laid out).
    sec_va: Vec<u64>,
}

impl<'a> Linker<'a> {
    fn new(objs: &'a [ObjectFile]) -> Result<Linker<'a>, ObjectError> {
        let globals = resolve_globals(objs)?;
        let mut base = Vec::with_capacity(objs.len() + 1);
        let mut total = 0usize;
        for o in objs {
            base.push(total);
            total = total
                .checked_add(o.sections.len())
                .ok_or(ObjectError::ArithmeticOverflow { context: "flat section space" })?;
        }
        base.push(total);
        let locals = objs
            .iter()
            .map(|o| {
                let mut m = BTreeMap::new();
                for (si, s) in o.symbols.iter().enumerate() {
                    if s.defined && !s.global {
                        m.entry(s.name.clone()).or_insert(si);
                    }
                }
                m
            })
            .collect();
        Ok(Linker {
            objs,
            base,
            total_sections: total,
            globals,
            locals,
            kept: vec![false; total],
            sec_va: vec![0u64; total],
        })
    }

    fn flat(&self, obj: usize, section: usize) -> usize {
        self.base[obj] + section
    }

    /// CEP:WHAT: Maps a flat section id back to (object, section); ids are dense so the
    ///   scan over base offsets is O(objects), bounded by MAX_INPUT_OBJECTS.
    /// CEP:WHY: The flat space serves the O(S+R) worklist; unflattening stays allocation-
    ///   free and deterministic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (ids are constructed from the same base vector)
    /// CEP:ASSUMES: f < total_sections.
    /// CEP:COST: O(objects).
    /// CEP:EVIDENCE: used by GC, layout, and relocation application.
    fn unflat(&self, f: usize) -> (usize, usize) {
        for i in 0..self.objs.len() {
            if f < self.base[i + 1] {
                return (i, f - self.base[i]);
            }
        }
        (self.objs.len() - 1, f - self.base[self.objs.len() - 1])
    }

    fn section_of(&self, f: usize) -> &Section {
        let (oi, si) = self.unflat(f);
        &self.objs[oi].sections[si]
    }

    /// CEP:WHAT: Resolves a relocation reference from `from_obj`: same-object defined
    ///   locals first (shadowing), then the global definition table.
    /// CEP:WHY: ELF binding semantics in one place, used by GC and relocation application
    ///   alike.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when the name is undefined (callers decide the policy).
    /// CEP:ASSUMES: validated objects (defined symbols have sections).
    /// CEP:COST: O(log Y).
    /// CEP:EVIDENCE: exercised by GC, entry lookup, and every linked-relocation test.
    fn resolve(&self, from_obj: usize, name: &str) -> Option<(usize, u64)> {
        if let Some(&si) = self.locals[from_obj].get(name) {
            let sym = &self.objs[from_obj].symbols[si];
            if let Some(sec) = sym.section {
                return Some((self.flat(from_obj, sec), sym.value));
            }
        }
        if let Some(d) = self.globals.get(name) {
            let sym = &self.objs[d.obj].symbols[d.sym];
            if let Some(sec) = sym.section {
                return Some((self.flat(d.obj, sec), sym.value));
            }
        }
        None
    }

    /// CEP:WHAT: Marks all sections reachable from the entry symbol via a worklist over
    ///   relocation edges; iterative, O(S + R).
    /// CEP:WHY: Section GC (STANDARDS 5.5); untrusted input can never drive recursion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: MissingEntrySymbol when the entry is not a defined global.
    /// CEP:ASSUMES: kept[] starts all-false; called exactly once per link.
    /// CEP:COST: O(S + R) flat ops plus O(log Y) per relocation lookup.
    /// CEP:EVIDENCE: unit tests gc_plan_keeps_and_drops and gc_removes_the_needing_section.
    fn mark_reachable(&mut self, entry: &str) -> Result<(), ObjectError> {
        let d = *self
            .globals
            .get(entry)
            .ok_or_else(|| ObjectError::MissingEntrySymbol { name: entry.to_string() })?;
        let root_sym = &self.objs[d.obj].symbols[d.sym];
        let root = self.flat(d.obj, root_sym.section.unwrap_or(0));
        let mut work = VecDeque::new();
        self.kept[root] = true;
        work.push_back(root);
        while let Some(f) = work.pop_front() {
            let (oi, si) = self.unflat(f);
            for r in &self.objs[oi].sections[si].relocs {
                if let Some((tf, _)) = self.resolve(oi, &r.symbol) {
                    if !self.kept[tf] {
                        self.kept[tf] = true;
                        work.push_back(tf);
                    }
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Rejects undefined references from kept sections (the post-GC policy
    ///   check: "undefined after GC is an error, unless GC removed the needing section").
    /// CEP:WHY: Distinguishes genuine unresolved references from dead ones.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UndefinedSymbol naming the first undefined reference in flat order.
    /// CEP:ASSUMES: called after kept[] is final.
    /// CEP:COST: O(S + R).
    /// CEP:EVIDENCE: acceptance negative tests (undefined with GC on and off).
    fn check_undefined(&self) -> Result<(), ObjectError> {
        for f in 0..self.total_sections {
            if !self.kept[f] {
                continue;
            }
            let (oi, si) = self.unflat(f);
            for r in &self.objs[oi].sections[si].relocs {
                if self.resolve(oi, &r.symbol).is_none() {
                    return Err(ObjectError::UndefinedSymbol { name: r.symbol.clone() });
                }
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Assigns file offsets and virtual addresses: one RX segment (exec
    ///   Progbits) then one RW segment (data Progbits, then zero-fill), each
    ///   alignment-first (descending, stable by input order), page-separated, and
    ///   base-congruent (p_vaddr - p_offset == base).
    /// CEP:WHY: Permission grouping plus alignment-first ordering is the layout rule of
    ///   STANDARDS 5.5; lockstep cursor advancement keeps segments loadable.
    /// CEP:STATUS: complete (rodata-only segment and hot/cold split: see HOT_COLD_SPLIT
    ///   stub flag in elf.rs)
    /// CEP:FAILURE: ArithmeticOverflow; InvalidBaseVaddr for non-page-aligned bases;
    ///   MissingEntrySymbol for unresolvable entry names.
    /// CEP:ASSUMES: kept[] final.
    /// CEP:COST: O(S log S) stable sorts plus O(S) checked cursor math.
    /// CEP:EVIDENCE: unit tests layout_respects_alignment and gc_plan_keeps_and_drops.
    fn layout(&mut self, entry: &str, base_vaddr: u64) -> Result<LinkPlan, ObjectError> {
        if !base_vaddr.is_multiple_of(ELF_PAGE_BYTES) {
            return Err(ObjectError::InvalidBaseVaddr { base: base_vaddr });
        }
        // Classify kept sections in input order, then stable-sort each bucket by
        // descending alignment (ties keep input order; deterministic).
        let mut rx: Vec<usize> = Vec::new();
        let mut rw_data: Vec<usize> = Vec::new();
        let mut rw_bss: Vec<usize> = Vec::new();
        for f in 0..self.total_sections {
            if !self.kept[f] {
                continue;
            }
            let sec = self.section_of(f);
            if sec.ty == SectionType::NoBits {
                rw_bss.push(f);
            } else if sec.is_exec() {
                rx.push(f);
            } else {
                rw_data.push(f);
            }
        }
        {
            let by_align_desc =
                |a: &usize, b: &usize| self.section_of(*b).align.cmp(&self.section_of(*a).align);
            rx.sort_by(by_align_desc);
            rw_data.sort_by(by_align_desc);
            rw_bss.sort_by(by_align_desc);
        }

        let n_phdrs = usize::from(!rx.is_empty())
            + usize::from(!(rw_data.is_empty() && rw_bss.is_empty()));
        let mut c: u64 = (EHDR_BYTES + PHDR_BYTES * n_phdrs) as u64;
        let mut off = vec![0u64; self.total_sections];

        // Places one bucket, advancing the lockstep cursor; returns the end cursor.
        fn place(
            lk: &Linker<'_>,
            list: &[usize],
            off: &mut [u64],
            c: &mut u64,
        ) -> Result<u64, ObjectError> {
            for &f in list {
                let sec = lk.section_of(f);
                *c = align_up(*c, sec.align)?;
                off[f] = *c;
                *c = c
                    .checked_add(sec.bytes.len() as u64)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "layout cursor" })?;
            }
            Ok(*c)
        }

        let rx_end = place(self, &rx, &mut off, &mut c)?;
        let data_end;
        if !rw_data.is_empty() || !rw_bss.is_empty() {
            c = align_up(c, ELF_PAGE_BYTES)?;
            data_end = place(self, &rw_data, &mut off, &mut c)?;
            c = place(self, &rw_bss, &mut off, &mut c)?;
        } else {
            data_end = rx_end;
        }
        let content_end = c;

        // Virtual addresses follow file offsets (base-congruent lockstep).
        for ((&file_off, &kept), va) in off
            .iter()
            .zip(self.kept.iter())
            .zip(self.sec_va.iter_mut())
        {
            if kept {
                *va = base_vaddr
                    .checked_add(file_off)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "section vaddr" })?;
            }
        }

        let mut segments = Vec::with_capacity(n_phdrs);
        if !rx.is_empty() {
            let start = off[rx[0]];
            let filesz = rx_end
                .checked_sub(start)
                .ok_or(ObjectError::ArithmeticOverflow { context: "rx filesz" })?;
            segments.push(PlannedSegment {
                flags: PF_R | PF_X,
                offset: start,
                vaddr: base_vaddr
                    .checked_add(start)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "rx vaddr" })?,
                filesz,
                memsz: filesz,
            });
        }
        if !rw_data.is_empty() || !rw_bss.is_empty() {
            let start = if rw_data.is_empty() { off[rw_bss[0]] } else { off[rw_data[0]] };
            let filesz = data_end
                .checked_sub(start)
                .ok_or(ObjectError::ArithmeticOverflow { context: "rw filesz" })?;
            let memsz = c
                .checked_sub(start)
                .ok_or(ObjectError::ArithmeticOverflow { context: "rw memsz" })?;
            segments.push(PlannedSegment {
                flags: PF_R | PF_W,
                offset: start,
                vaddr: base_vaddr
                    .checked_add(start)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "rw vaddr" })?,
                filesz,
                memsz,
            });
        }

        let entry_def = *self
            .globals
            .get(entry)
            .ok_or_else(|| ObjectError::MissingEntrySymbol { name: entry.to_string() })?;
        let entry_sym = &self.objs[entry_def.obj].symbols[entry_def.sym];
        let entry_flat = self.flat(entry_def.obj, entry_sym.section.unwrap_or(0));
        let entry_vaddr = self.sec_va[entry_flat]
            .checked_add(entry_sym.value)
            .ok_or(ObjectError::ArithmeticOverflow { context: "entry vaddr" })?;

        let mut sections = Vec::with_capacity(rx.len() + rw_data.len() + rw_bss.len());
        for f in rx.iter().chain(rw_data.iter()).chain(rw_bss.iter()) {
            let (oi, si) = self.unflat(*f);
            let sec = &self.objs[oi].sections[si];
            sections.push(PlannedSection {
                object: oi,
                section: si,
                name: sec.name.clone(),
                flags: sec.flags,
                align: sec.align,
                offset: off[*f],
                vaddr: self.sec_va[*f],
                size: sec.bytes.len() as u64,
                file_backed: sec.ty != SectionType::NoBits,
            });
        }

        Ok(LinkPlan {
            base_vaddr,
            entry: entry_vaddr,
            segments,
            sections,
            content_end,
        })
    }

    /// CEP:WHAT: Applies one relocation into the output image: PcRel32 writes S + A - P
    ///   (must fit i32), Abs64 writes S + A (must fit u64).
    /// CEP:WHY: The final fixup step; range violations must be errors, never wraps.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UndefinedSymbol (defensive; the post-GC check already ran),
    ///   RelocOutOfRange, ArithmeticOverflow, WriteOutOfBounds.
    /// CEP:ASSUMES: model validation guarantees the write lands inside section bytes.
    /// CEP:COST: O(1) plus one bounded write.
    /// CEP:EVIDENCE: acceptance e2e run (call relocations resolve to exit code 13) and
    ///   unit test pc_rel_out_of_range_rejected.
    fn apply_reloc(
        &self,
        image: &mut [u8],
        ps: &PlannedSection,
        r: &Reloc,
    ) -> Result<(), ObjectError> {
        let (tf, sv) = self
            .resolve(ps.object, &r.symbol)
            .ok_or_else(|| ObjectError::UndefinedSymbol { name: r.symbol.clone() })?;
        let s = self.sec_va[tf]
            .checked_add(sv)
            .ok_or(ObjectError::ArithmeticOverflow { context: "symbol vaddr" })?;
        let site = ps
            .vaddr
            .checked_add(r.offset)
            .ok_or(ObjectError::ArithmeticOverflow { context: "reloc site vaddr" })?;
        let file_at = ps
            .offset
            .checked_add(r.offset)
            .ok_or(ObjectError::ArithmeticOverflow { context: "reloc file offset" })?;
        let file_at = usize::try_from(file_at)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "reloc file offset" })?;
        match r.kind {
            RelocKind::PcRel32 => {
                let v = (s as i128) + (r.addend as i128) - (site as i128);
                if v < i32::MIN as i128 || v > i32::MAX as i128 {
                    return Err(ObjectError::RelocOutOfRange {
                        symbol: r.symbol.clone(),
                        kind: r.kind.as_str(),
                        value: v,
                    });
                }
                write_i32_at(image, file_at, v as i32)
            }
            RelocKind::Abs64 => {
                let v = (s as i128) + (r.addend as i128);
                if v < 0 || v > u64::MAX as i128 {
                    return Err(ObjectError::RelocOutOfRange {
                        symbol: r.symbol.clone(),
                        kind: r.kind.as_str(),
                        value: v,
                    });
                }
                write_u64_at(image, file_at, v as u64)
            }
        }
    }

    /// CEP:WHAT: Assembles the ET_EXEC image from a plan: Ehdr, PTLOAD Phdrs, section
    ///   bytes with relocations applied, .shstrtab, and a section header table.
    /// CEP:WHY: Executables keep section headers so standard tools (readelf, gdb, objdump)
    ///   remain useful; program headers are what the kernel loads.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded, ArithmeticOverflow, WriteOutOfBounds; never panics.
    /// CEP:ASSUMES: plan was produced by layout() over the same kept set.
    /// CEP:COST: O(image bytes); exactly one allocation.
    /// CEP:EVIDENCE: acceptance test execs the produced image on Linux.
    fn emit(&self, plan: &LinkPlan) -> Result<Vec<u8>, ObjectError> {
        let mut shstr = StringTable::new();
        let mut name_off = Vec::with_capacity(plan.sections.len());
        for ps in &plan.sections {
            name_off.push(shstr.add(&ps.name)?);
        }
        let shstrtab_name = shstr.add(".shstrtab")?;
        let shnum = 1 + plan.sections.len() + 1;
        if shnum > MAX_OUTPUT_SECTIONS {
            return Err(ObjectError::BudgetExceeded {
                what: "MAX_OUTPUT_SECTIONS",
                limit: MAX_OUTPUT_SECTIONS as u64,
                actual: shnum as u64,
            });
        }
        let shstrtab_off = align_up(plan.content_end, 8)?;
        let shoff = align_up(
            shstrtab_off
                .checked_add(shstr.len() as u64)
                .ok_or(ObjectError::ArithmeticOverflow { context: "shstrtab end" })?,
            8,
        )?;
        let total = shoff
            .checked_add(
                (shnum as u64)
                    .checked_mul(SHDR_BYTES as u64)
                    .ok_or(ObjectError::ArithmeticOverflow { context: "shdr table size" })?,
            )
            .ok_or(ObjectError::ArithmeticOverflow { context: "image size" })?;
        let mut image = vec![0u8; usize::try_from(total)
            .map_err(|_| ObjectError::ArithmeticOverflow { context: "image allocation" })?];

        let ehdr = Ehdr64 {
            e_type: ET_EXEC,
            e_machine: EM_X86_64,
            e_entry: plan.entry,
            e_phoff: EHDR_BYTES as u64,
            e_shoff: shoff,
            e_flags: 0,
            e_phentsize: PHDR_BYTES as u16,
            e_phnum: plan.segments.len() as u16,
            e_shnum: shnum as u16,
            e_shstrndx: (shnum - 1) as u16,
        };
        write_bytes_at(&mut image, 0, &ehdr.to_bytes())?;
        for (i, seg) in plan.segments.iter().enumerate() {
            let ph = Phdr64 {
                p_type: PT_LOAD,
                p_flags: seg.flags,
                p_offset: seg.offset,
                p_vaddr: seg.vaddr,
                p_paddr: seg.vaddr,
                p_filesz: seg.filesz,
                p_memsz: seg.memsz,
                p_align: ELF_PAGE_BYTES,
            };
            write_bytes_at(&mut image, EHDR_BYTES + i * PHDR_BYTES, &ph.to_bytes())?;
        }

        for ps in &plan.sections {
            if !ps.file_backed {
                continue;
            }
            let sec = &self.objs[ps.object].sections[ps.section];
            let at = usize::try_from(ps.offset)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "section offset" })?;
            write_bytes_at(&mut image, at, &sec.bytes)?;
            for r in &sec.relocs {
                self.apply_reloc(&mut image, ps, r)?;
            }
        }

        write_bytes_at(
            &mut image,
            usize::try_from(shstrtab_off)
                .map_err(|_| ObjectError::ArithmeticOverflow { context: "shstrtab offset" })?,
            shstr.as_bytes(),
        )?;

        let mut shdrs: Vec<[u8; SHDR_BYTES]> = Vec::with_capacity(shnum);
        shdrs.push([0u8; SHDR_BYTES]);
        for (i, ps) in plan.sections.iter().enumerate() {
            shdrs.push(
                Shdr64 {
                    sh_name: name_off[i],
                    sh_type: if ps.file_backed { SHT_PROGBITS } else { SHT_NOBITS },
                    sh_flags: ps.flags,
                    sh_addr: ps.vaddr,
                    sh_offset: ps.offset,
                    sh_size: ps.size,
                    sh_link: 0,
                    sh_info: 0,
                    sh_addralign: ps.align,
                    sh_entsize: 0,
                }
                .to_bytes(),
            );
        }
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
        Ok(image)
    }
}

/// CEP:WHAT: Plans the static executable: validate inputs, resolve symbols, optionally GC
///   sections from the entry symbol, and assign addresses.
/// CEP:WHY: Inspectable layout is required for deterministic-debugging workflows and lets
///   tests observe GC without parsing binaries.
/// CEP:STATUS: complete
/// CEP:FAILURE: all resolution/GC/layout failures (see stage functions); never panics.
/// CEP:ASSUMES: base_vaddr page-aligned; entry is a defined global symbol.
/// CEP:COST: O(Y log Y + S + R + S log S).
/// CEP:EVIDENCE: unit test rv8_object::link::tests::gc_plan_keeps_and_drops.
pub fn plan_executable(
    objs: &[ObjectFile],
    entry: &str,
    base_vaddr: u64,
    gc_sections: bool,
) -> Result<LinkPlan, ObjectError> {
    plan_internal(objs, entry, base_vaddr, gc_sections).map(|(_, plan)| plan)
}

/// CEP:WHAT: Links a static ET_EXEC x86-64 executable from universal objects.
/// CEP:WHY: The acceptance product: deterministic bytes that the kernel can run.
/// CEP:STATUS: complete
/// CEP:FAILURE: plan_executable failures plus emission bounds failures.
/// CEP:ASSUMES: see plan_executable.
/// CEP:COST: plan cost plus O(image bytes) emission.
/// CEP:EVIDENCE: acceptance test links [add.o, main.o, synth(_start)] and the process
///   exits 13; determinism tests assert byte equality across runs.
pub fn link_executable(
    objs: &[ObjectFile],
    entry: &str,
    base_vaddr: u64,
    gc_sections: bool,
) -> Result<Vec<u8>, ObjectError> {
    let (lk, plan) = plan_internal(objs, entry, base_vaddr, gc_sections)?;
    lk.emit(&plan)
}

/// CEP:WHAT: Shared pipeline: validate, build linker state, GC (or keep-all), reject
///   undefined references in kept sections, and lay out. Returns the state together with
///   the plan so emission reuses the assigned section addresses.
/// CEP:WHY: One pipeline for both entry points guarantees plan and emitted image can
///   never disagree about addresses.
/// CEP:STATUS: complete
/// CEP:FAILURE: delegated to the stage functions; never panics.
/// CEP:ASSUMES: none
/// CEP:COST: the plan cost.
/// CEP:EVIDENCE: exercised by every link test including abs64_pointer_table.
fn plan_internal<'a>(
    objs: &'a [ObjectFile],
    entry: &str,
    base_vaddr: u64,
    gc_sections: bool,
) -> Result<(Linker<'a>, LinkPlan), ObjectError> {
    let mut lk = new_validated_linker(objs)?;
    if gc_sections {
        lk.mark_reachable(entry)?;
    } else {
        for k in lk.kept.iter_mut() {
            *k = true;
        }
    }
    lk.check_undefined()?;
    let plan = lk.layout(entry, base_vaddr)?;
    Ok((lk, plan))
}

/// CEP:WHAT: Validates inputs under the object budget and constructs the linker state.
/// CEP:WHY: Shared fail-closed preamble for plan_executable and link_executable.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded (MAX_INPUT_OBJECTS), model validation errors,
///   DuplicateStrongSymbol.
/// CEP:ASSUMES: none
/// CEP:COST: O(Y log Y + S).
/// CEP:EVIDENCE: exercised by every link test.
fn new_validated_linker(objs: &[ObjectFile]) -> Result<Linker<'_>, ObjectError> {
    if objs.len() > MAX_INPUT_OBJECTS {
        return Err(ObjectError::BudgetExceeded {
            what: "MAX_INPUT_OBJECTS",
            limit: MAX_INPUT_OBJECTS as u64,
            actual: objs.len() as u64,
        });
    }
    for o in objs {
        o.validate()?;
    }
    Linker::new(objs)
}

/// CEP:WHAT: Builds the synthetic startup object: `.text` is EXACTLY
///   `e8 <rel32>` (call main, PcRel32 against `main_symbol`, addend -4),
///   `48 89 c7` (mov rdi, rax), `b8 3c 00 00 00` (mov eax, 60 = SYS_exit), `0f 05`
///   (syscall); symbol `_start` is defined, global, at offset 0, size 15.
/// CEP:WHY: Freestanding executables need an ABI-conformant entry shim that forwards
///   main's return value to exit(2).
/// CEP:STATUS: complete
/// CEP:FAILURE: none at construction (static bytes); a bad main_symbol fails at link time
///   as UndefinedSymbol/MissingEntrySymbol per policy.
/// CEP:ASSUMES: Linux x86-64 syscall ABI (rax = 60, rdi = code); main returns its status
///   in eax.
/// CEP:COST: O(1) allocations.
/// CEP:EVIDENCE: unit test rv8_object::link::tests::synth_start_exact_bytes and the
///   acceptance run exits 13.
pub fn synthesize_start(main_symbol: &str) -> ObjectFile {
    const START_BYTES: [u8; 15] = [
        0xe8, 0x00, 0x00, 0x00, 0x00, // call rel32 (patched by the PcRel32 relocation)
        0x48, 0x89, 0xc7, // mov rdi, rax
        0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, 60 (SYS_exit)
        0x0f, 0x05, // syscall
    ];
    ObjectFile {
        name: "rv8.synth.start".to_string(),
        sections: vec![Section {
            name: ".text".to_string(),
            ty: SectionType::Progbits,
            flags: FLAG_ALLOC | FLAG_EXEC,
            align: 16,
            bytes: START_BYTES.to_vec(),
            relocs: vec![Reloc {
                offset: 1,
                kind: RelocKind::PcRel32,
                symbol: main_symbol.to_string(),
                addend: -4,
            }],
        }],
        symbols: vec![crate::model::Symbol {
            name: "_start".to_string(),
            defined: true,
            global: true,
            section: Some(0),
            value: 0,
            size: START_BYTES.len() as u64,
        }],
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Pins the exact 15 bytes and relocation of synthesize_start.
    /// CEP:WHY: The startup shim is a binary contract with the target kernel.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn synth_start_exact_bytes() {
        let o = synthesize_start("main");
        assert_eq!(o.sections.len(), 1);
        assert_eq!(o.symbols.len(), 1);
        assert_eq!(
            o.sections[0].bytes,
            vec![
                0xe8, 0x00, 0x00, 0x00, 0x00, // call rel32
                0x48, 0x89, 0xc7, // mov rdi, rax
                0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, 60
                0x0f, 0x05, // syscall
            ]
        );
        assert_eq!(o.sections[0].relocs.len(), 1);
        assert_eq!(
            o.sections[0].relocs[0],
            Reloc {
                offset: 1,
                kind: RelocKind::PcRel32,
                symbol: "main".to_string(),
                addend: -4,
            }
        );
        let s = &o.symbols[0];
        assert_eq!(s.name, "_start");
        assert!(s.defined && s.global);
        assert_eq!(s.section, Some(0));
        assert_eq!(s.value, 0);
        assert_eq!(s.size, 15);
        assert!(o.validate().is_ok());
    }

    /// CEP:WHAT: The strong/weak merge lattice: strong beats weak in both input orders,
    ///   strong+strong is an error, first wins among equals.
    /// CEP:WHY: Resolution rule of STANDARDS 5.5.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: model revision has no weak bit; exercised through merge_def directly.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn merge_lattice() {
        let mut t = BTreeMap::new();
        merge_def(&mut t, "f", Def { obj: 0, sym: 0, strong: false }).unwrap();
        merge_def(&mut t, "f", Def { obj: 1, sym: 0, strong: true }).unwrap();
        assert_eq!(t.get("f"), Some(&Def { obj: 1, sym: 0, strong: true })); // strong won

        let mut t = BTreeMap::new();
        merge_def(&mut t, "g", Def { obj: 1, sym: 0, strong: true }).unwrap();
        merge_def(&mut t, "g", Def { obj: 0, sym: 0, strong: false }).unwrap();
        assert_eq!(t.get("g"), Some(&Def { obj: 1, sym: 0, strong: true })); // strong kept

        let mut t = BTreeMap::new();
        merge_def(&mut t, "h", Def { obj: 0, sym: 0, strong: true }).unwrap();
        assert_eq!(
            merge_def(&mut t, "h", Def { obj: 1, sym: 0, strong: true }),
            Err(ObjectError::DuplicateStrongSymbol { name: "h".into() })
        );

        let mut t = BTreeMap::new();
        merge_def(&mut t, "w", Def { obj: 0, sym: 0, strong: false }).unwrap();
        merge_def(&mut t, "w", Def { obj: 1, sym: 0, strong: false }).unwrap();
        assert_eq!(t.get("w"), Some(&Def { obj: 0, sym: 0, strong: false })); // first wins
    }

    /// CEP:WHAT: GC drops unreachable sections under gc_sections=true and keeps everything
    ///   under false; the kept set covers the full call chain entry -> main -> helper.
    /// CEP:WHY: Required GC behavior of the acceptance list.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: all text sections share alignment 16, so emission order is input order.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn gc_plan_keeps_and_drops() {
        let mut o = ObjectFile::new("app.o");
        let main = o
            .add_text_section(".text", 16, vec![0xe8, 0, 0, 0, 0, 0xc3])
            .unwrap(); // call helper; ret
        o.add_symbol("main", true, true, Some(main), 0, 6).unwrap();
        o.add_reloc(main, 1, RelocKind::PcRel32, "helper", -4).unwrap();
        let helper = o.add_text_section(".text.helper", 16, vec![0xc3]).unwrap();
        o.add_symbol("helper", true, true, Some(helper), 0, 1).unwrap();
        let dead = o.add_data_section(".data.dead", 8, vec![0xAA; 16]).unwrap();
        o.add_symbol("dead_sym", true, true, Some(dead), 0, 16).unwrap();

        let objs = vec![o, synthesize_start("main")];
        let plan_gc = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        let names_gc: Vec<&str> = plan_gc.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names_gc, vec![".text", ".text.helper", ".text"]);
        assert!(!plan_gc.sections.iter().any(|s| s.name == ".data.dead"));

        let plan_all = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, false).unwrap();
        assert_eq!(plan_all.sections.len(), plan_gc.sections.len() + 1);
        assert!(plan_all.sections.iter().any(|s| s.name == ".data.dead"));
        assert_eq!(plan_all.segments.len(), 2); // RX + RW
        assert_eq!(plan_all.segments[1].flags, PF_R | PF_W);

        // The entry address lands in the synthetic _start section (last RX section).
        let synth = plan_gc.sections.last().unwrap();
        assert_eq!(plan_gc.entry, synth.vaddr);
        assert_eq!(plan_gc.segments.len(), 1); // the dropped data section leaves RX only
        assert_eq!(plan_gc.segments[0].flags, PF_R | PF_X);
    }

    /// CEP:WHAT: Layout honors section alignment with descending-align ordering inside the
    ///   RX segment.
    /// CEP:WHY: Alignment-first ordering is a layout requirement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn layout_respects_alignment() {
        let mut o = ObjectFile::new("align.o");
        // Loose section first in input order, strict second: strict must be placed first.
        let loose = o.add_text_section(".text.loose", 1, vec![0x90; 16]).unwrap();
        let strict = o.add_text_section(".text.strict", 64, vec![0x90, 0xc3]).unwrap();
        o.add_symbol("loose", true, true, Some(loose), 0, 16).unwrap();
        o.add_symbol("strict", true, true, Some(strict), 0, 2).unwrap();
        // Chain entry -> loose -> strict so both survive GC; Abs64 pointer at offset 0.
        o.add_reloc(loose, 0, RelocKind::Abs64, "strict", 0).unwrap();
        let objs = vec![o, synthesize_start("loose")];
        let plan = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        assert_eq!(plan.sections[0].name, ".text.strict"); // align 64 first
        assert_eq!(plan.sections[0].vaddr % 64, 0);
        assert_eq!(plan.sections[1].name, ".text"); // synth, align 16
        assert_eq!(plan.sections[2].name, ".text.loose"); // align 1 last
        assert!(plan.sections[2].vaddr >= plan.sections[0].vaddr + 2);
    }

    /// CEP:WHAT: A PcRel32 relocation whose value exceeds i32 fails closed.
    /// CEP:WHY: Range-check requirement; the addend drives the value beyond range without
    ///   multi-gigabyte sections.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn pc_rel_out_of_range_rejected() {
        let mut o = ObjectFile::new("far.o");
        let t = o.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0, 0xc3]).unwrap();
        o.add_symbol("main", true, true, Some(t), 0, 6).unwrap();
        o.add_reloc(t, 1, RelocKind::PcRel32, "main", 1 << 40).unwrap();
        let objs = vec![o, synthesize_start("main")];
        match link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true) {
            Err(ObjectError::RelocOutOfRange { kind, .. }) => assert_eq!(kind, "PcRel32"),
            other => panic!("expected RelocOutOfRange, got {other:?}"),
        }
    }

    /// CEP:WHAT: Duplicate strong globals across objects are rejected.
    /// CEP:WHY: Required negative behavior.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn duplicate_strong_rejected() {
        let mut a = ObjectFile::new("a.o");
        let ta = a.add_text_section(".text", 1, vec![0xc3]).unwrap();
        a.add_symbol("dup", true, true, Some(ta), 0, 1).unwrap();
        let mut b = ObjectFile::new("b.o");
        let tb = b.add_text_section(".text", 1, vec![0xc3]).unwrap();
        b.add_symbol("dup", true, true, Some(tb), 0, 1).unwrap();
        let objs = vec![a, b, synthesize_start("dup")];
        assert_eq!(
            link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true),
            Err(ObjectError::DuplicateStrongSymbol { name: "dup".into() })
        );
    }

    /// CEP:WHAT: Undefined references from kept sections are rejected with and without GC.
    /// CEP:WHY: Required negative behavior.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn undefined_rejected() {
        let mut o = ObjectFile::new("undef.o");
        let t = o.add_text_section(".text", 1, vec![0xe8, 0, 0, 0, 0, 0xc3]).unwrap();
        o.add_symbol("main", true, true, Some(t), 0, 6).unwrap();
        o.add_reloc(t, 1, RelocKind::PcRel32, "ghost", -4).unwrap();
        let objs = vec![o, synthesize_start("main")];
        for gc in [true, false] {
            assert_eq!(
                link_executable(&objs, "_start", DEFAULT_BASE_VADDR, gc),
                Err(ObjectError::UndefinedSymbol { name: "ghost".into() })
            );
        }
    }

    /// CEP:WHAT: MAX_INPUT_OBJECTS budget is enforced by the link entry points.
    /// CEP:WHY: Named budget enforcement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: empty objects are cheap.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn object_budget_enforced() {
        let objs: Vec<ObjectFile> =
            (0..=MAX_INPUT_OBJECTS).map(|_| ObjectFile::new("o.o")).collect();
        assert!(matches!(
            plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, false),
            Err(ObjectError::BudgetExceeded { .. })
        ));
    }

    /// CEP:WHAT: A dead section carrying an undefined reference links fine under GC but
    ///   fails without it: GC removes the needing section.
    /// CEP:WHY: The precise "unless gc_sections removed the needing section" rule.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn gc_removes_the_needing_section() {
        let mut o = ObjectFile::new("need.o");
        let main = o.add_text_section(".text", 1, vec![0xc3]).unwrap();
        o.add_symbol("main", true, true, Some(main), 0, 1).unwrap();
        let dead = o.add_data_section(".data.dead", 1, vec![0; 8]).unwrap();
        o.add_reloc(dead, 0, RelocKind::Abs64, "ghost", 0).unwrap();
        let objs = vec![o, synthesize_start("main")];
        assert!(link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).is_ok());
        assert_eq!(
            link_executable(&objs, "_start", DEFAULT_BASE_VADDR, false),
            Err(ObjectError::UndefinedSymbol { name: "ghost".into() })
        );
    }

    /// CEP:WHAT: An Abs64 pointer-table relocation resolves to the target section vaddr
    ///   plus symbol value.
    /// CEP:WHY: Covers the non-call relocation path end to end.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via unwrap/assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn abs64_pointer_table() {
        let mut o = ObjectFile::new("ptr.o");
        let main = o
            .add_text_section(".text.main", 1, vec![0, 0, 0, 0, 0, 0, 0, 0, 0xc3])
            .unwrap();
        o.add_symbol("main", true, true, Some(main), 8, 1).unwrap();
        o.add_reloc(main, 0, RelocKind::Abs64, "table", 0).unwrap(); // GC edge to .data
        let tab = o.add_data_section(".data", 8, vec![0; 8]).unwrap();
        o.add_symbol("table", true, true, Some(tab), 0, 8).unwrap();
        o.add_reloc(tab, 0, RelocKind::Abs64, "main", 0).unwrap();
        let objs = vec![o, synthesize_start("main")];
        let plan = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        let img = link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        let data = plan.sections.iter().find(|s| s.name == ".data").unwrap();
        let off = data.offset as usize;
        let got = u64::from_le_bytes(img[off..off + 8].try_into().unwrap());
        let main_ps = plan.sections.iter().find(|s| s.name == ".text.main").unwrap();
        assert_eq!(got, main_ps.vaddr + 8); // S + A with A = 0; main sits at value 8
    }

    /// CEP:WHAT: Two links over identical inputs produce identical bytes; the plan is
    ///   identical too.
    /// CEP:WHY: Determinism is the product (STANDARDS 5.5).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test; the full determinism matrix lives in tests/acceptance.rs.
    #[test]
    fn executable_emission_is_deterministic() {
        let mut o = ObjectFile::new("det.o");
        let t = o
            .add_text_section(".text", 16, vec![0x8d, 0x04, 0x37, 0xc3])
            .unwrap();
        o.add_symbol("main", true, true, Some(t), 0, 4).unwrap();
        let objs = vec![o, synthesize_start("main")];
        let a = link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        let b = link_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        assert_eq!(a, b);
        let pa = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        let pb = plan_executable(&objs, "_start", DEFAULT_BASE_VADDR, true).unwrap();
        assert_eq!(pa, pb);
    }
}
