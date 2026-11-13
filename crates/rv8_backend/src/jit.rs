// CEP:FILE: crates/rv8_backend/src/jit.rs
// CEP:WHAT: The Runtime Code Emitter's instruction path (design.md "The Runtime Code
//   Emitter" 3.1-3.4, RV8-141): a LIR module is compiled through the canonical AOT
//   pipeline (compile_lir - the JIT's correctness oracle, so the executed bytes are the
//   object-path bytes by construction), emitted into one deterministic W^X CodePool, and
//   every relocation is resolved BEFORE finalization: module-local call targets through
//   the module's own slots, external targets through a caller-provided function table
//   (ordinal -> absolute address), PcRel32 displacements range-checked against i32,
//   Abs64 sites written in full width. Pure-AOT output is untouched: this module only
//   consumes compile_lir bytes and never feeds back into the object path.
// CEP:WHY: STANDARDS 5.13 acceptance ("emitted code executes and returns the expected
//   result in-process") needs more than leaf bodies: real programs call. encode.rs is
//   design 3.3 (the instruction encoder), jitmem is 3.2/3.5/3.6 (memory, coherency,
//   patching), and this module is 3.4 (the relocation resolver) plus the emitter
//   assembly that turns a LIR module into executable function entries.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete for the instruction path; region deopt, OSR, tiering and the
//   runtime-optimization catalog remain per RV8-141 (design 1, 2, 4, 5).
// CEP:FAILURE: every fallible operation returns JitEmitterError; resolution validates
//   every patch before any byte is written and a failed build drops the whole pool, so
//   no half-resolved module can escape; unknown or ambiguous symbols, rel32 range
//   overflows, and patch sites outside a body fail closed; compile and jitmem errors are
//   threaded through. Library code never panics.
// CEP:ASSUMES: x86-64 (the shipped JIT target; encode.rs produces its bytes); the
//   "fn{ordinal}" symbol convention from encode::call_symbol_name; extern addresses are
//   valid call targets for the process lifetime and are quiesced by the caller; module
//   addresses fit in 47-bit user space so usize -> i64 address casts are lossless.
// CEP:COST: one compile_lir per function (the AOT pipeline cost), O(relocs) resolution,
//   O(pages) finalization; memory bounded by the CodePool page budget.
// CEP:EVIDENCE: the unit tests here (resolver formulas, range and atomicity fail-closed
//   paths) and crates/rv8_backend/tests/jit_module.rs (in-process execution of
//   cross-function calls: forward, backward, chained, shared, and external targets;
//   patch-time call-site redirect; layout determinism; executed bytes equal linked AOT
//   bytes).

use crate::encode::call_symbol_name;
use crate::{compile_lir, BackendError, CodegenOutput, RelocKind, Relocation};
use rv8_ir::lir::LirFunction;
use rv8_runtime::jitmem::{CodePool, CodeSlot, JitError};
use std::collections::BTreeMap;

/// CEP:WHAT: Every way the emitter's instruction path refuses to produce broken code.
/// CEP:WHY: Fail-closed contract of the module (CEP:FAILURE): symbol, range, and site
///   errors are distinct so a caller can tell a missing function table entry from a
///   layout that genuinely cannot reach its target.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type IS the failure surface.
/// CEP:ASSUMES: none
/// CEP:COST: inline data; one String per symbol-carrying variant.
/// CEP:EVIDENCE: every variant is pinned by at least one unit or integration test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitEmitterError {
    /// LIR failed the AOT pipeline (isel, regalloc, rewrite, schedule, verify, encode).
    Backend(BackendError),
    /// The W^X memory manager refused an operation.
    Memory(JitError),
    /// A relocation names a symbol that is neither a module-local ordinal nor in the
    /// function table.
    UnknownSymbol { symbol: String },
    /// An ordinal is both module-local and present in the function table (or declared
    /// twice in the same source), so its target is ambiguous.
    AmbiguousSymbol { symbol: String },
    /// The same module-local ordinal was declared twice in build()'s input.
    DuplicateOrdinal { ordinal: i64 },
    /// A PcRel32 displacement does not fit in 32 bits (targets beyond +/-2GiB; pools in
    /// this tier are small, so this fires only on genuinely unreachable layouts).
    RelocOutOfRange { symbol: String, value: i64 },
    /// A patch site plus its width lies outside the function's code bytes.
    RelocSiteOutOfBounds { symbol: String, offset: u64, code_len: usize },
    /// build() received no functions.
    EmptyModule,
}

impl std::fmt::Display for JitEmitterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitEmitterError::Backend(e) => write!(f, "jit emitter: backend pipeline: {e}"),
            JitEmitterError::Memory(e) => write!(f, "jit emitter: memory manager: {e}"),
            JitEmitterError::UnknownSymbol { symbol } => {
                write!(f, "jit emitter: unresolved relocation symbol {symbol}")
            }
            JitEmitterError::AmbiguousSymbol { symbol } => {
                write!(f, "jit emitter: symbol {symbol} is module-local and external")
            }
            JitEmitterError::DuplicateOrdinal { ordinal } => {
                write!(f, "jit emitter: ordinal {ordinal} declared twice in the module")
            }
            JitEmitterError::RelocOutOfRange { symbol, value } => {
                write!(f, "jit emitter: {symbol} displacement {value} exceeds rel32 range")
            }
            JitEmitterError::RelocSiteOutOfBounds { symbol, offset, code_len } => {
                write!(f, "jit emitter: {symbol} patch site {offset} outside {code_len} bytes")
            }
            JitEmitterError::EmptyModule => write!(f, "jit emitter: module has no functions"),
        }
    }
}

impl From<BackendError> for JitEmitterError {
    fn from(e: BackendError) -> Self {
        JitEmitterError::Backend(e)
    }
}

impl From<JitError> for JitEmitterError {
    fn from(e: JitError) -> Self {
        JitEmitterError::Memory(e)
    }
}

/// CEP:WHAT: One computed relocation patch: write `bytes[..len]` at byte `offset` of the
///   function's code.
/// CEP:WHY: The resolver returns prepared patches instead of writing, so validation of
///   the whole set completes before any byte changes (the all-or-nothing contract).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: len is 4 (PcRel32) or 8 (Abs64).
/// CEP:COST: inline data.
/// CEP:EVIDENCE: resolver unit tests assert the exact bytes and lengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPatch {
    /// Byte offset from the start of the function's code.
    pub offset: usize,
    /// Patch payload, left-aligned; only bytes[..len] are meaningful.
    pub bytes: [u8; 8],
    /// Meaningful payload length (4 or 8).
    pub len: usize,
}

/// CEP:WHAT: Design 3.4 relocation resolver (pure): computes every patch for one
///   function's relocations against `resolve_symbol`, validates all of them (site
///   bounds, rel32 range), and only then returns them. PcRel32 writes S + A - P as
///   signed 32-bit LE (A = addend, P = code_base + offset) and Abs64 writes S + A as
///   64-bit LE - exactly the rv8_object/ELF semantics, so the JIT bytes equal the
///   linked AOT bytes.
/// CEP:WHY: The all-or-nothing contract: the caller applies returned patches to
///   still-writable memory, so a rejected function is never half-patched.
/// CEP:STATUS: complete
/// CEP:FAILURE: UnknownSymbol (through resolve_symbol), RelocOutOfRange, and
///   RelocSiteOutOfBounds; no patch is produced when any input is rejected.
/// CEP:ASSUMES: code_len is the function's full emitted length; code_base is the
///   address the code will occupy after the W^X flip (page addresses are stable).
/// CEP:COST: O(relocs).
/// CEP:EVIDENCE: the resolver unit tests below pin the formulas byte for byte.
pub fn resolve_relocs(
    code_len: usize,
    code_base: usize,
    relocs: &[Relocation],
    resolve_symbol: &dyn Fn(&str) -> Result<usize, JitEmitterError>,
) -> Result<Vec<ResolvedPatch>, JitEmitterError> {
    let mut out = Vec::with_capacity(relocs.len());
    for r in relocs {
        let width = match r.kind {
            RelocKind::PcRel32 => 4usize,
            RelocKind::Abs64 => 8usize,
        };
        let site = usize::try_from(r.offset)
            .ok()
            .and_then(|o| o.checked_add(code_base))
            .ok_or_else(|| JitEmitterError::RelocSiteOutOfBounds {
                symbol: r.symbol.clone(),
                offset: r.offset,
                code_len,
            })?;
        let end = usize::try_from(r.offset)
            .ok()
            .and_then(|o| o.checked_add(width))
            .ok_or_else(|| JitEmitterError::RelocSiteOutOfBounds {
                symbol: r.symbol.clone(),
                offset: r.offset,
                code_len,
            })?;
        if end > code_len {
            return Err(JitEmitterError::RelocSiteOutOfBounds {
                symbol: r.symbol.clone(),
                offset: r.offset,
                code_len,
            });
        }
        let target = resolve_symbol(&r.symbol)?;
        let bytes = match r.kind {
            RelocKind::PcRel32 => {
                let value = target as i64 + r.addend - site as i64;
                if value < i32::MIN as i64 || value > i32::MAX as i64 {
                    return Err(JitEmitterError::RelocOutOfRange {
                        symbol: r.symbol.clone(),
                        value,
                    });
                }
                let v = (value as i32).to_le_bytes();
                ResolvedPatch { offset: end - width, bytes: [v[0], v[1], v[2], v[3], 0, 0, 0, 0], len: 4 }
            }
            RelocKind::Abs64 => {
                let value = (target as u64).wrapping_add(r.addend as u64).to_le_bytes();
                ResolvedPatch { offset: end - width, bytes: value, len: 8 }
            }
        };
        out.push(bytes);
    }
    Ok(out)
}

/// CEP:WHAT: Pool geometry for a JIT module (page capacity and page budget of the
///   underlying CodePool).
/// CEP:WHY: Budgets are the repo norm (fail closed, never allocate unbounded); the
///   caller decides how much executable memory a module may take.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: page_cap is a positive PAGE_SIZE multiple (validated by CodePool::new).
/// CEP:COST: POD, Copy.
/// CEP:EVIDENCE: oversized_module_fails_closed integration test.
#[derive(Debug, Clone, Copy)]
pub struct JitModuleConfig {
    /// Per-page capacity (a PAGE_SIZE multiple).
    pub page_cap: usize,
    /// Maximum number of pages in the segment list.
    pub max_pages: usize,
}

/// CEP:WHAT: One compiled-and-resolved function inside a `JitModule`: its function-table
///   ordinal, the CodeSlot holding its bytes, and its code length.
/// CEP:WHY: Patch-time redirection (deopt/OSR/inline caches, design 3.6) addresses code
///   through slots, not raw pointers; the ordinal is the module's public name.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the owning module outlives every use of the slot.
/// CEP:COST: inline data, Copy.
/// CEP:EVIDENCE: call_site_redirect integration test patches through a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitFunction {
    /// The function-table ordinal this body implements (symbol fn{ordinal}).
    pub ordinal: i64,
    /// Where the body lives in the module's pool.
    pub slot: CodeSlot,
    /// Full emitted code length (relocation sites are offsets into it).
    pub code_len: usize,
}

/// CEP:WHAT: The assembled JIT module: one W^X CodePool holding every function's
///   AOT-identical bytes with all relocations resolved, finalized read-execute, plus the
///   ordinal -> slot directory.
/// CEP:WHY: The emitter's end product (design 3.1): executable entries for each
///   function-table ordinal, patchable through slots for the later deopt/OSR tiers.
/// CEP:STATUS: complete
/// CEP:FAILURE: build() fails closed with JitEmitterError before any memory maps (compile
///   and symbol validation) or drops the pool (resolution); entry/slot refuse unknown
///   ordinals.
/// CEP:ASSUMES: single-threaded use for this tier (CodePool is !Send by its raw
///   pointers); extern addresses stay valid and quiesced for the module's lifetime.
/// CEP:COST: the pool budget plus one compile_lir per function.
/// CEP:EVIDENCE: tests/jit_module.rs executes cross-function call graphs in-process.
#[derive(Debug)]
pub struct JitModule {
    pool: CodePool,
    functions: Vec<JitFunction>,
}

impl JitModule {
    /// CEP:WHAT: The instruction path (design 3.1-3.4): compile every LIR function
    ///   through the AOT oracle, validate ordinals (non-empty, unique, unambiguous
    ///   against the function table), emit bodies into one pool in ascending-ordinal
    ///   order (canonical layout), resolve every relocation through the local slot table
    ///   and the external function table, apply the patches while pages are still
    ///   writable, and finalize the pool read-execute.
    /// CEP:WHY: One entry point keeps the layout (and therefore the executed bytes)
    ///   canonical; symbol resolution precedes finalization so a finalized page never
    ///   carries an unresolved or wrong address.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: EmptyModule, DuplicateOrdinal/AmbiguousSymbol before any memory
    ///   maps; Backend for a rejected body; Memory for pool/refusal errors; the whole
    ///   pool is dropped on any failure, so no partial module escapes.
    /// CEP:ASSUMES: ordinals are non-negative function-table indices (encode.rs
    ///   convention); extern ordinals do not collide with module-local ones.
    /// CEP:COST: O(sum pipeline costs) + O(relocs) + O(pages).
    /// CEP:EVIDENCE: every integration test in tests/jit_module.rs goes through build.
    pub fn build(
        fns: &[(i64, LirFunction)],
        externs: &[(i64, usize)],
        cfg: JitModuleConfig,
    ) -> Result<Self, JitEmitterError> {
        if fns.is_empty() {
            return Err(JitEmitterError::EmptyModule);
        }
        // Ordinal validation before anything maps: duplicates and local/external
        // collisions are input contract violations, not runtime conditions.
        let mut ordered: Vec<(i64, &LirFunction)> = fns.iter().map(|(o, f)| (*o, f)).collect();
        ordered.sort_by_key(|(ord, _)| *ord);
        for (i, &(ord, _)) in ordered.iter().enumerate() {
            if i + 1 < ordered.len() && ordered[i + 1].0 == ord {
                return Err(JitEmitterError::DuplicateOrdinal { ordinal: ord });
            }
            let symbol = call_symbol_name(ord);
            if externs.iter().any(|&(e, _)| e == ord) {
                return Err(JitEmitterError::AmbiguousSymbol { symbol });
            }
        }
        let mut extern_table: BTreeMap<String, usize> = BTreeMap::new();
        for &(ord, addr) in externs {
            let symbol = call_symbol_name(ord);
            if extern_table.insert(symbol.clone(), addr).is_some() {
                return Err(JitEmitterError::AmbiguousSymbol { symbol });
            }
        }
        // Compile every body through the AOT oracle before any memory maps: a rejected
        // LIR leaves no pages behind.
        let mut compiled: Vec<(i64, CodegenOutput)> = Vec::with_capacity(ordered.len());
        for (ord, lir) in &ordered {
            let out = compile_lir(lir, &call_symbol_name(*ord))?;
            compiled.push((*ord, out));
        }
        let mut pool = CodePool::new(cfg.page_cap, cfg.max_pages)?;
        let mut functions = Vec::with_capacity(compiled.len());
        for (ord, out) in &compiled {
            let slot = pool.emit(&out.code)?;
            functions.push(JitFunction { ordinal: *ord, slot, code_len: out.code.len() });
        }
        // Symbol table: module-local slots first (their addresses are the write-phase
        // page bases), the function table as fallback.
        let mut symbols: BTreeMap<String, usize> = BTreeMap::new();
        for f in &functions {
            let base = pool.page_addr(f.slot.page)? as usize + f.slot.offset;
            symbols.insert(call_symbol_name(f.ordinal), base);
        }
        for f in &functions {
            let out = &compiled.iter().find(|(o, _)| *o == f.ordinal).expect("paired").1;
            let resolve = |symbol: &str| -> Result<usize, JitEmitterError> {
                symbols
                    .get(symbol)
                    .or_else(|| extern_table.get(symbol))
                    .copied()
                    .ok_or_else(|| JitEmitterError::UnknownSymbol {
                        symbol: symbol.to_string(),
                    })
            };
            let patches = resolve_relocs(f.code_len, symbols[&call_symbol_name(f.ordinal)], &out.relocs, &resolve)?;
            for p in patches {
                pool.write_slot(f.slot, p.offset, &p.bytes[..p.len])?;
            }
        }
        pool.finalize_all()?;
        Ok(JitModule { pool, functions })
    }

    /// CEP:WHAT: Executable entry for `ordinal` (finalized pool only).
    /// CEP:WHY: The sanctioned way to obtain a call target; execution sites transmute it
    ///   under their own SAFETY contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Memory(NotFinalized/OffsetOutOfRange) or UnknownSymbol for an
    ///   ordinal the module does not implement.
    /// CEP:ASSUMES: the module outlives every call through the pointer.
    /// CEP:COST: O(log n) ordinal lookup.
    /// CEP:EVIDENCE: every execution test enters through entry.
    pub fn entry(&self, ordinal: i64) -> Result<*const u8, JitEmitterError> {
        let f = self.lookup(ordinal)?;
        Ok(self.pool.code_ptr(f.slot)?)
    }

    /// CEP:WHAT: The CodeSlot of one function, for patch-time redirection (design 3.6).
    /// CEP:WHY: Deopt/OSR/inline-cache tiers patch through slots; the directory is the
    ///   only sanctioned mapping from ordinal to slot.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnknownSymbol for an ordinal the module does not implement.
    /// CEP:ASSUMES: the module outlives the slot's use.
    /// CEP:COST: O(log n) ordinal lookup.
    /// CEP:EVIDENCE: call_site_redirect integration test.
    pub fn slot(&self, ordinal: i64) -> Result<JitFunction, JitEmitterError> {
        self.lookup(ordinal).copied()
    }

    /// CEP:WHAT: Patch one function's code at `in_slot` bytes into its body through the
    ///   jitmem atomic protocol (write-enable, at most MAX_ATOMIC_PATCH bytes inside one
    ///   8-byte window, fence, re-protect).
    /// CEP:WHY: Design 3.6 code patching (deopt/OSR/inline-cache redirection) addresses
    ///   live code by ordinal; the module is the only sanctioned patch entry so every
    ///   rewrite goes through the W^X protocol.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnknownSymbol for an unimplemented ordinal; Memory(PatchTooLarge/
    ///   PatchUnalignedWindow/OffsetOutOfRange/...) from the protocol.
    /// CEP:ASSUMES: threads executing this module are quiesced by the caller (the tier
    ///   contract of design 3.6 step 1).
    /// CEP:COST: two mprotect + copy + fence.
    /// CEP:EVIDENCE: call_site_redirect_via_patch integration test redirects a live call
    ///   and observes the new target's result.
    pub fn patch(&mut self, ordinal: i64, in_slot: usize, bytes: &[u8]) -> Result<(), JitEmitterError> {
        let f = self.lookup(ordinal)?;
        self.pool.patch(f.slot, in_slot, bytes)?;
        Ok(())
    }

    /// CEP:WHAT: How many functions the module implements.
    /// CEP:WHY: Tests and diagnostics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: module_shape integration test.
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// CEP:WHAT: How many pages the module's segment list occupies.
    /// CEP:WHY: Tests, diagnostics, and budget accounting.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: module_shape integration test.
    pub fn page_count(&self) -> usize {
        self.pool.page_count()
    }

    /// CEP:WHAT: Ordinal lookup over the ascending directory.
    /// CEP:WHY: Shared by entry and slot.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnknownSymbol.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(log n).
    /// CEP:EVIDENCE: entry_unknown_ordinal_fails integration test.
    fn lookup(&self, ordinal: i64) -> Result<&JitFunction, JitEmitterError> {
        self.functions
            .binary_search_by_key(&ordinal, |f| f.ordinal)
            .map(|i| &self.functions[i])
            .map_err(|_| JitEmitterError::UnknownSymbol { symbol: call_symbol_name(ordinal) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CEP:WHAT: PcRel32 writes S + A - P as signed 32-bit LE, forward and backward:
    ///   base 0x1000, site at offset 8 (P = 0x1008), target 0x1040 gives 0x34;
    ///   target 0x0F00 gives -0x10C.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn pcrel32_displacement_formula_forward_and_backward() {
        let reloc = |sym: &str| Relocation {
            offset: 8,
            kind: RelocKind::PcRel32,
            symbol: sym.to_string(),
            addend: -4,
        };
        let resolve = |s: &str| Ok::<usize, JitEmitterError>(if s == "fn1" { 0x1040 } else { 0x0F00 });
        let fwd = resolve_relocs(32, 0x1000, &[reloc("fn1")], &resolve).expect("forward");
        assert_eq!(fwd.len(), 1);
        assert_eq!(fwd[0].offset, 8);
        assert_eq!(fwd[0].len, 4);
        assert_eq!(&fwd[0].bytes[..4], &[0x34, 0x00, 0x00, 0x00], "0x1040 - 4 - 0x1008");
        let bwd = resolve_relocs(32, 0x1000, &[reloc("fn0")], &resolve).expect("backward");
        let v = i32::from_le_bytes([bwd[0].bytes[0], bwd[0].bytes[1], bwd[0].bytes[2], bwd[0].bytes[3]]);
        assert_eq!(v, -0x10C, "0x0F00 - 4 - 0x1008");
    }

    /// CEP:WHAT: A PcRel32 displacement beyond +/-2GiB fails closed with the exact
    ///   value reported.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn pcrel32_out_of_range_fails_closed() {
        let relocs = [Relocation {
            offset: 8,
            kind: RelocKind::PcRel32,
            symbol: "fn1".to_string(),
            addend: -4,
        }];
        let resolve = |_s: &str| Ok::<usize, JitEmitterError>(0x2_0000_0000);
        let err = resolve_relocs(32, 0x1000, &relocs, &resolve).unwrap_err();
        assert_eq!(
            err,
            JitEmitterError::RelocOutOfRange { symbol: "fn1".to_string(), value: 0x1_FFFF_EFF4 }
        );
    }

    /// CEP:WHAT: Abs64 writes S + A in full 64-bit width, addend honored.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn abs64_writes_s_plus_a() {
        let relocs = [Relocation {
            offset: 16,
            kind: RelocKind::Abs64,
            symbol: "data".to_string(),
            addend: 8,
        }];
        let resolve = |_s: &str| Ok::<usize, JitEmitterError>(0xDEAD_0000);
        let out = resolve_relocs(32, 0x1000, &relocs, &resolve).expect("abs64");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].offset, 16);
        assert_eq!(out[0].len, 8);
        assert_eq!(&out[0].bytes[..8], &0xDEAD_0008u64.to_le_bytes());
    }

    /// CEP:WHAT: An unresolved symbol is reported with its name; a patch site outside
    ///   the code (4- and 8-byte widths, and an offset beyond usize) is refused.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn unknown_symbol_and_site_bounds_fail_closed() {
        let resolve = |_s: &str| Err(JitEmitterError::UnknownSymbol { symbol: "fn9".into() });
        let relocs = [Relocation {
            offset: 8,
            kind: RelocKind::PcRel32,
            symbol: "fn9".to_string(),
            addend: -4,
        }];
        assert_eq!(
            resolve_relocs(32, 0x1000, &relocs, &resolve).unwrap_err(),
            JitEmitterError::UnknownSymbol { symbol: "fn9".to_string() }
        );
        let resolve = |_s: &str| Ok::<usize, JitEmitterError>(0x1000);
        // 4-byte width: offset 29 + 4 > 32.
        let near_end = [Relocation {
            offset: 29,
            kind: RelocKind::PcRel32,
            symbol: "fn1".to_string(),
            addend: -4,
        }];
        assert!(matches!(
            resolve_relocs(32, 0x1000, &near_end, &resolve),
            Err(JitEmitterError::RelocSiteOutOfBounds { .. })
        ));
        // 8-byte width: offset 25 + 8 > 32.
        let abs_end = [Relocation {
            offset: 25,
            kind: RelocKind::Abs64,
            symbol: "fn1".to_string(),
            addend: 0,
        }];
        assert!(matches!(
            resolve_relocs(32, 0x1000, &abs_end, &resolve),
            Err(JitEmitterError::RelocSiteOutOfBounds { .. })
        ));
    }

    /// CEP:WHAT: Resolution is all-or-nothing: a valid first relocation plus a broken
    ///   second produces an error and no patches at all.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn resolution_is_all_or_nothing() {
        let relocs = [
            Relocation {
                offset: 8,
                kind: RelocKind::PcRel32,
                symbol: "fn1".to_string(),
                addend: -4,
            },
            Relocation {
                offset: 1000,
                kind: RelocKind::PcRel32,
                symbol: "fn1".to_string(),
                addend: -4,
            },
        ];
        let resolve = |_s: &str| Ok::<usize, JitEmitterError>(0x1040);
        assert!(matches!(
            resolve_relocs(32, 0x1000, &relocs, &resolve),
            Err(JitEmitterError::RelocSiteOutOfBounds { .. })
        ));
    }
}
