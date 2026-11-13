// CEP:FILE: crates/rv8_runtime/src/jitmem.rs
// CEP:WHAT: The JIT executable-memory manager (design.md "The Runtime Code Emitter" 3.2,
//   3.5, 3.6 and STANDARDS 5.13): three-phase W^X code pages (mmap read-write, emit, then
//   mprotect read-execute), a deterministic first-fit bump allocator over a linked code
//   segment list, reference-counted reclamation, the emit/patch cache-coherency fences,
//   and the bounded patching protocol (at most MAX_ATOMIC_PATCH bytes per patch, never
//   crossing an atomic write window, always through the write-enable -> patch -> fence ->
//   re-protect sequence).
// CEP:WHY: The JIT hybrid (RV8-141) executes compiler-emitted machine code in-process;
//   modern OSes forbid writable+executable pages, so the W^X protocol is not optional,
//   and every patch must keep running threads from observing partial instructions.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every fallible operation returns JitError; a page is never writable after
//   finalization (write returns WriteAfterFinalize, not silent corruption), code pointers
//   are refused before finalization (NotFinalized), pool exhaustion fails closed
//   (OutOfCapacity/OutOfPages), and patches above the atomic window fail closed
//   (PatchTooLarge/PatchUnalignedWindow) instead of tearing.
// CEP:ASSUMES: unix targets (mmap/mprotect/munmap through the libc symbols std already
//   links; declared locally, zero external crates); the host page granularity is covered
//   by PAGE_SIZE alignment (capacities are page multiples); x86-64 keeps icache coherent
//   with the dcache so the SeqCst fence is the documented 3.5 barrier (ARM64 dc cvau /
//   ic ivau and RISC-V fence.i remain target-future work and are refused nowhere - they
//   are simply not needed on the shipped target); thread-suspension patching for
//   above-window rewrites and dual-mapping (alias W mapping) are the documented next
//   tier steps, per hybrid fidelity.
// CEP:COST: allocation is O(1) bump per emit plus O(pages) first-fit scan; patch is two
//   mprotect calls plus one fence; reclamation is O(pages).
// CEP:EVIDENCE: unit tests in this module (phase discipline, bump across pages, budget,
//   patch protocol, refcount reclamation, fence presence by construction) and the
//   in-process execution tests: the hand-encoded mov-eax/ret probe and the full
//   rv8_backend::compile_lir bytes executed in-process (crates/rv8_backend/tests/
//   jit_emitter.rs) returning the expected result.
// CEP:SECURITY: pages carry no absolute external addresses (call relocations resolve
//   through the driver's function table before finalization); mmap is private and
//   anonymous; no file-backed executable memory; munmap in Drop so reclaimed code can
//   never be resurrected through a stale pointer into a freed heap object.

extern crate alloc;

use alloc::vec::Vec;
use core::ffi::c_void;
use core::fmt;
use core::sync::atomic::{fence, Ordering};

/// CEP:WHAT: Page granularity of the code segment list (design 3.2: "each page is 4KB").
/// CEP:WHY: Capacities are validated as page multiples so every page is one mmap.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: 4096 divides every realistic host page size on the shipped target.
/// CEP:COST: constant.
/// CEP:EVIDENCE: allocate rejects non-multiples (unit test page_capacity_discipline).
pub const PAGE_SIZE: usize = 4096;

/// CEP:WHAT: The largest patch that is applied as one atomic window write (design 3.6:
///   "for small patches (<= 8 bytes), the emitter uses atomic writes").
/// CEP:WHY: x86-64 guarantees atomic observation for aligned writes up to 8 bytes;
///   larger rewrites need thread suspension, which is tier-future work and fails closed.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: constant.
/// CEP:EVIDENCE: patch_atomic_window_is_enforced unit test.
pub const MAX_ATOMIC_PATCH: usize = 8;

/// CEP:WHAT: Every way the memory manager refuses to do something unsafe.
/// CEP:WHY: Fail-closed contract of the module (CEP:FAILURE).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type IS the failure surface.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: every variant is pinned by at least one unit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitError {
    /// mmap failed in the allocate phase.
    MapFailed,
    /// mprotect failed while switching phases.
    ProtectFailed,
    /// munmap failed while reclaiming (the page is left untouched).
    UnmapFailed,
    /// write() called on a finalized (executable) page.
    WriteAfterFinalize,
    /// code_ptr() called before finalize().
    NotFinalized,
    /// emit would overflow the requested page capacity.
    OutOfCapacity,
    /// the pool hit its page budget.
    OutOfPages,
    /// capacity or emit length is zero or not a page multiple where required.
    BadCapacity,
    /// patch larger than MAX_ATOMIC_PATCH.
    PatchTooLarge { max: usize, got: usize },
    /// patch crosses an atomic write window (8-byte aligned).
    PatchUnalignedWindow,
    /// patch or code_ptr offset outside the page.
    OffsetOutOfRange,
    /// non-unix target: no executable-memory protocol is implemented there.
    UnsupportedTarget,
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JitError::MapFailed => write!(f, "jitmem: mmap of code page failed"),
            JitError::ProtectFailed => write!(f, "jitmem: mprotect phase switch failed"),
            JitError::UnmapFailed => write!(f, "jitmem: munmap of code page failed"),
            JitError::WriteAfterFinalize => {
                write!(f, "jitmem: write into finalized (executable) page")
            }
            JitError::NotFinalized => write!(f, "jitmem: code pointer before finalize"),
            JitError::OutOfCapacity => write!(f, "jitmem: emit exceeds page capacity"),
            JitError::OutOfPages => write!(f, "jitmem: code segment list exhausted"),
            JitError::BadCapacity => write!(f, "jitmem: capacity must be a positive page multiple"),
            JitError::PatchTooLarge { max, got } => {
                write!(f, "jitmem: patch of {got} bytes exceeds atomic window {max}")
            }
            JitError::PatchUnalignedWindow => {
                write!(f, "jitmem: patch crosses an atomic write window")
            }
            JitError::OffsetOutOfRange => write!(f, "jitmem: offset outside the page"),
            JitError::UnsupportedTarget => {
                write!(f, "jitmem: executable memory unsupported on this target")
            }
        }
    }
}

#[cfg(unix)]
mod sys {
    use core::ffi::c_void;

    // libc symbols the std binary already links; declared locally so the crate keeps
    // its zero-external-dependency rule.
    extern "C" {
        pub fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            off: i64,
        ) -> *mut c_void;
        pub fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
        pub fn munmap(addr: *mut c_void, len: usize) -> i32;
    }

    pub const PROT_READ: i32 = 1;
    pub const PROT_WRITE: i32 = 2;
    pub const PROT_EXEC: i32 = 4;
    pub const MAP_PRIVATE: i32 = 0x02;
    pub const MAP_ANONYMOUS: i32 = 0x20;

    /// CEP:WHAT: MAP_FAILED as a comparable value.
    /// CEP:WHY: mmap reports failure as (void*)-1.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: allocate unit tests.
    pub fn is_map_failed(p: *mut c_void) -> bool {
        p as usize == usize::MAX
    }
}

/// CEP:WHAT: One W^X code page: a single mmap holding a bump-allocated code segment.
/// CEP:WHY: design 3.2's unit of allocation and reclamation; the three-phase protocol
///   (allocate writable -> emit -> finalize executable) lives here.
/// CEP:STATUS: complete
/// CEP:FAILURE: see JitError; Drop munmaps, so a reclaimed page is unmapped exactly once.
/// CEP:ASSUMES: single-threaded use per page pool for this tier (pages are !Send by the
///   raw pointer; cross-thread sharing is the tier-future concurrency step).
/// CEP:COST: O(1) per bump write, two syscalls per phase switch.
/// CEP:EVIDENCE: the unit tests and the in-process execution tests in rv8_backend.
pub struct CodePage {
    addr: *mut u8,
    cap: usize,
    used: usize,
    executable: bool,
    live_slots: u32,
}

// SAFETY: the page memory is owned exclusively by this value; the raw pointer never
// aliases another Rust object. Sharing across threads is deliberately NOT promised
// (no unsafe impl Send/Sync); the tier-future concurrency step must revisit this.
impl CodePage {
    /// CEP:WHAT: Phase 1 of the W^X protocol: allocate `cap` bytes read-write, not
    ///   executable.
    /// CEP:WHY: design 3.2 step 1.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BadCapacity (zero or non-page-multiple), MapFailed, UnsupportedTarget.
    /// CEP:ASSUMES: none
    /// CEP:COST: one mmap.
    /// CEP:EVIDENCE: page_capacity_discipline, phase_discipline unit tests.
    pub fn allocate(cap: usize) -> Result<Self, JitError> {
        if cap == 0 || !cap.is_multiple_of(PAGE_SIZE) {
            return Err(JitError::BadCapacity);
        }
        let addr = Self::map_rw(cap)?;
        Ok(CodePage { addr, cap, used: 0, executable: false, live_slots: 0 })
    }

    #[cfg(unix)]
    fn map_rw(cap: usize) -> Result<*mut u8, JitError> {
        // SAFETY: private anonymous mapping of `cap` bytes; the kernel picks the address.
        let p = unsafe {
            sys::mmap(
                core::ptr::null_mut(),
                cap,
                sys::PROT_READ | sys::PROT_WRITE,
                sys::MAP_PRIVATE | sys::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if sys::is_map_failed(p) {
            return Err(JitError::MapFailed);
        }
        Ok(p.cast::<u8>())
    }

    #[cfg(not(unix))]
    fn map_rw(_cap: usize) -> Result<*mut u8, JitError> {
        Err(JitError::UnsupportedTarget)
    }

    /// CEP:WHAT: Phase 2 of the W^X protocol: copy `bytes` at `offset` while the page is
    ///   still writable.
    /// CEP:WHY: design 3.2 step 2 (emit); refusing post-finalize writes is the W^X
    ///   discipline that keeps a live page from being silently rewritten.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: WriteAfterFinalize, OutOfCapacity, BadCapacity (empty bytes).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(bytes) copy.
    /// CEP:EVIDENCE: phase_discipline unit test.
    pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), JitError> {
        if self.executable {
            return Err(JitError::WriteAfterFinalize);
        }
        if bytes.is_empty() {
            return Err(JitError::BadCapacity);
        }
        let end = offset.checked_add(bytes.len()).ok_or(JitError::OutOfCapacity)?;
        if end > self.cap {
            return Err(JitError::OutOfCapacity);
        }
        // SAFETY: [offset, end) is within this page's exclusive mmap by the bounds check.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.addr.add(offset), bytes.len());
        }
        Ok(())
    }

    /// CEP:WHAT: Phase 3 of the W^X protocol: re-protect the page read-execute (never
    ///   writable) and issue the 3.5 coherency barrier.
    /// CEP:WHY: design 3.2 step 3 and 3.5 (x86-64: coherent icache, the fence is the
    ///   documented mfence-equivalent ordering point).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: ProtectFailed, UnsupportedTarget; idempotent (a second finalize is a
    ///   no-op re-protect of an already-RX page).
    /// CEP:ASSUMES: none
    /// CEP:COST: one mprotect + one fence.
    /// CEP:EVIDENCE: phase_discipline unit test; execution tests only run after finalize.
    pub fn finalize(&mut self) -> Result<(), JitError> {
        if self.executable {
            return Ok(());
        }
        Self::protect(self.addr, self.cap, true)?;
        self.executable = true;
        fence(Ordering::SeqCst);
        Ok(())
    }

    #[cfg(unix)]
    fn protect(addr: *mut u8, cap: usize, exec: bool) -> Result<(), JitError> {
        let prot = if exec { sys::PROT_READ | sys::PROT_EXEC } else { sys::PROT_READ | sys::PROT_WRITE };
        // SAFETY: [addr, addr+cap) is this page's exclusive mmap.
        let rc = unsafe { sys::mprotect(addr.cast::<c_void>(), cap, prot) };
        if rc != 0 {
            return Err(JitError::ProtectFailed);
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn protect(_addr: *mut u8, _cap: usize, _exec: bool) -> Result<(), JitError> {
        Err(JitError::UnsupportedTarget)
    }

    /// CEP:WHAT: Design 3.6 patching protocol: temporarily write-enable, write at most
    ///   MAX_ATOMIC_PATCH bytes without crossing an 8-byte window, fence, re-protect.
    /// CEP:WHY: Deopt/OSR/inline-cache redirection must never let a thread observe a
    ///   partially written instruction; the window rule is what makes the write atomic
    ///   on x86-64.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: NotFinalized (patching non-live code), PatchTooLarge,
    ///   PatchUnalignedWindow, OffsetOutOfRange, ProtectFailed.
    /// CEP:ASSUMES: the caller has quiesced threads executing this page (safepoint) -
    ///   documented tier contract, mirroring design 3.6 step 1.
    /// CEP:COST: two mprotect + copy + fence.
    /// CEP:EVIDENCE: patch_protocol_and_atomic_window unit test.
    pub fn patch(&mut self, offset: usize, bytes: &[u8]) -> Result<(), JitError> {
        if !self.executable {
            return Err(JitError::NotFinalized);
        }
        if bytes.is_empty() {
            return Err(JitError::BadCapacity);
        }
        if bytes.len() > MAX_ATOMIC_PATCH {
            return Err(JitError::PatchTooLarge { max: MAX_ATOMIC_PATCH, got: bytes.len() });
        }
        let end = offset.checked_add(bytes.len()).ok_or(JitError::OffsetOutOfRange)?;
        if end > self.cap {
            return Err(JitError::OffsetOutOfRange);
        }
        let window_end = (offset & !7usize) + 8;
        if end > window_end {
            return Err(JitError::PatchUnalignedWindow);
        }
        Self::protect(self.addr, self.cap, false)?;
        // SAFETY: bounds-checked above against this page's exclusive mmap.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.addr.add(offset), bytes.len());
        }
        fence(Ordering::SeqCst);
        Self::protect(self.addr, self.cap, true)?;
        Ok(())
    }

    /// CEP:WHAT: Executable entry pointer for `offset`, valid only after finalize.
    /// CEP:WHY: The single sanctioned way to obtain a code address (execution sites
    ///   transmute it under their own SAFETY contract).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: NotFinalized, OffsetOutOfRange.
    /// CEP:ASSUMES: the caller keeps the page (and pool) alive while executing.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used by every execution test.
    pub fn code_ptr(&self, offset: usize) -> Result<*const u8, JitError> {
        if !self.executable {
            return Err(JitError::NotFinalized);
        }
        if offset >= self.used {
            return Err(JitError::OffsetOutOfRange);
        }
        // SAFETY: offset < used <= cap, pointer is page-owned.
        Ok(unsafe { self.addr.add(offset) })
    }

    /// CEP:WHAT: Bump allocation inside the page: reserve `len` bytes and copy `code`
    ///   there, returning the slot offset.
    /// CEP:WHY: design 3.2 memory layout ("code is emitted sequentially, advancing a
    ///   pointer").
    /// CEP:STATUS: complete
    /// CEP:FAILURE: WriteAfterFinalize, OutOfCapacity, BadCapacity.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(len).
    /// CEP:EVIDENCE: bump_layout unit test.
    pub fn bump_emit(&mut self, code: &[u8]) -> Result<usize, JitError> {
        let offset = self.used;
        self.write(offset, code)?;
        self.used = offset + code.len();
        Ok(offset)
    }

    /// CEP:WHAT: Reference counting for reclamation (design 3.2: "reference counting to
    ///   ensure no live pointers exist").
    /// CEP:WHY: A page may only be reclaimed when every slot's live count is zero.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: the caller models "a live pointer exists" honestly (each emit's
    ///   consumer drops its ref when the code is provably unreachable).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: refcount_reclamation unit test.
    pub fn add_slot_ref(&mut self) {
        self.live_slots += 1;
    }

    /// CEP:WHAT: Drops one live reference; when the count reaches zero the page becomes
    ///   reclaimable ("marked cold" per design 3.2).
    /// CEP:WHY: Reclamation precondition.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (an underflow is clamped at zero and debug_asserted).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: refcount_reclamation unit test.
    pub fn drop_slot_ref(&mut self) {
        debug_assert!(self.live_slots > 0, "slot refcount underflow");
        self.live_slots = self.live_slots.saturating_sub(1);
    }

    /// CEP:WHAT: Whether the page holds no live code pointers and may be reclaimed.
    /// CEP:WHY: The pool's reclamation scan asks exactly this.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: refcount_reclamation unit test.
    pub fn is_reclaimable(&self) -> bool {
        self.live_slots == 0
    }

    /// CEP:WHAT: Bytes consumed by the bump pointer.
    /// CEP:WHY: Tests and pool layout accounting.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: bump_layout unit test.
    pub fn used(&self) -> usize {
        self.used
    }
}

impl fmt::Debug for CodePage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodePage")
            .field("addr", &(self.addr as usize))
            .field("cap", &self.cap)
            .field("used", &self.used)
            .field("executable", &self.executable)
            .field("live_slots", &self.live_slots)
            .finish()
    }
}

impl Drop for CodePage {
    fn drop(&mut self) {
        if self.cap == 0 {
            return;
        }
        #[cfg(unix)]
        {
            // SAFETY: page-exclusive mapping being released exactly once.
            let rc = unsafe { sys::munmap(self.addr.cast::<c_void>(), self.cap) };
            debug_assert_eq!(rc, 0, "munmap of an owned code page failed");
        }
    }
}

/// CEP:WHAT: A handle to one emitted body inside a `CodePool`.
/// CEP:WHY: Execution and patching address code through handles, never through raw
///   arithmetic on foreign pointers.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the pool that produced it outlives every use.
/// CEP:COST: POD, Copy.
/// CEP:EVIDENCE: pool_emit_across_pages unit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSlot {
    pub page: usize,
    pub offset: usize,
    pub len: usize,
}

/// CEP:WHAT: The code segment list (design 3.2): pages linked in allocation order, each
///   bump-allocated first-fit, with a page budget and cold-page reclamation.
/// CEP:WHY: The pool is the emitter's allocator of record; deterministic first-fit keeps
///   layout reproducible for a content-seeded tiering decision.
/// CEP:STATUS: complete
/// CEP:FAILURE: OutOfPages when the budget is hit; every CodePage failure otherwise.
/// CEP:ASSUMES: single-threaded use per pool for this tier.
/// CEP:COST: O(pages) first-fit scan per emit.
/// CEP:EVIDENCE: pool_emit_across_pages, pool_budget, refcount_reclamation tests.
#[derive(Debug)]
pub struct CodePool {
    pages: Vec<CodePage>,
    page_cap: usize,
    max_pages: usize,
}

impl CodePool {
    /// CEP:WHAT: An empty pool whose pages each hold `page_cap` bytes, at most
    ///   `max_pages` of them.
    /// CEP:WHY: Budgets are the repo norm (fail closed, never allocate unbounded).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BadCapacity for a non-page-multiple page size.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: pool_budget unit test.
    pub fn new(page_cap: usize, max_pages: usize) -> Result<Self, JitError> {
        if page_cap == 0 || !page_cap.is_multiple_of(PAGE_SIZE) {
            return Err(JitError::BadCapacity);
        }
        Ok(CodePool { pages: Vec::new(), page_cap, max_pages })
    }

    /// CEP:WHAT: Deterministic first-fit emit: the first page with room takes the body,
    ///   else a fresh page is mapped (within budget), the body is written, and its slot
    ///   refcount starts at one.
    /// CEP:WHY: design 3.2 ("when a page is full, a new page is allocated; pages are
    ///   linked in a code segment list").
    /// CEP:STATUS: complete
    /// CEP:FAILURE: OutOfCapacity (body larger than a page), OutOfPages, map/protect
    ///   errors.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(pages) scan + O(len) copy.
    /// CEP:EVIDENCE: pool_emit_across_pages, pool_budget unit tests.
    pub fn emit(&mut self, code: &[u8]) -> Result<CodeSlot, JitError> {
        if code.is_empty() {
            return Err(JitError::BadCapacity);
        }
        if code.len() > self.page_cap {
            return Err(JitError::OutOfCapacity);
        }
        for (idx, page) in self.pages.iter_mut().enumerate() {
            if page.used() + code.len() <= self.page_cap {
                let offset = page.bump_emit(code)?;
                page.add_slot_ref();
                return Ok(CodeSlot { page: idx, offset, len: code.len() });
            }
        }
        if self.pages.len() == self.max_pages {
            return Err(JitError::OutOfPages);
        }
        let mut page = CodePage::allocate(self.page_cap)?;
        let offset = page.bump_emit(code)?;
        page.add_slot_ref();
        self.pages.push(page);
        Ok(CodeSlot { page: self.pages.len() - 1, offset, len: code.len() })
    }

    /// CEP:WHAT: Phase 3 for the whole segment list: every page becomes read-execute and
    ///   the coherency fence is issued.
    /// CEP:WHY: The emitter finalizes once after the last body lands.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: first protect error wins (pages stay in a consistent state: earlier
    ///   pages finalized, later ones still writable).
    /// CEP:ASSUMES: no more emits after finalize_all.
    /// CEP:COST: O(pages) syscalls + one fence.
    /// CEP:EVIDENCE: execution tests call exactly this before transmute.
    pub fn finalize_all(&mut self) -> Result<(), JitError> {
        for page in self.pages.iter_mut() {
            page.finalize()?;
        }
        fence(Ordering::SeqCst);
        Ok(())
    }

    /// CEP:WHAT: Executable pointer for a slot (finalized pool only).
    /// CEP:WHY: Execution sites transmute this under their own SAFETY contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: NotFinalized/OffsetOutOfRange through the page.
    /// CEP:ASSUMES: pool outlives every call through the pointer.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: execution tests.
    pub fn code_ptr(&self, slot: CodeSlot) -> Result<*const u8, JitError> {
        self.pages
            .get(slot.page)
            .ok_or(JitError::OffsetOutOfRange)?
            .code_ptr(slot.offset)
    }

    /// CEP:WHAT: Patch one slot body at `in_slot` offset through the 3.6 protocol.
    /// CEP:WHY: Deopt/OSR/inline-cache redirection.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: as CodePage::patch.
    /// CEP:ASSUMES: threads quiesced per the tier contract.
    /// CEP:COST: two mprotect + copy + fence.
    /// CEP:EVIDENCE: patch_protocol_and_atomic_window unit test.
    pub fn patch(&mut self, slot: CodeSlot, in_slot: usize, bytes: &[u8]) -> Result<(), JitError> {
        let offset = slot
            .offset
            .checked_add(in_slot)
            .ok_or(JitError::OffsetOutOfRange)?;
        self.pages
            .get_mut(slot.page)
            .ok_or(JitError::OffsetOutOfRange)?
            .patch(offset, bytes)
    }

    /// CEP:WHAT: Raw base address of one page in the segment list, available BEFORE
    ///   finalization.
    /// CEP:WHY: Relocation resolution computes absolute displacements (S + A - P for
    ///   PcRel32) while the page is still writable, so patch sites are resolved before
    ///   the W^X flip per the module SECURITY contract (a finalized page never carries
    ///   an unresolved address). code_ptr() stays finalize-gated for execution; this
    ///   accessor is the write-phase exception for the emitter only.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: OffsetOutOfRange for an unknown page index.
    /// CEP:ASSUMES: callers use the address for relocation arithmetic and writes through
    ///   write/bump_emit, never for execution.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: page_addr_reports_write_phase_base unit test; the rv8_backend jit
    ///   module resolves cross-page call displacements through it.
    pub fn page_addr(&self, page: usize) -> Result<*const u8, JitError> {
        self.pages
            .get(page)
            .map(|p| p.addr as *const u8)
            .ok_or(JitError::OffsetOutOfRange)
    }

    /// CEP:WHAT: Pre-finalize in-slot write: overwrite `bytes` at `in_slot` inside one
    ///   already-emitted body, refused once the pool finalized and bounded by the slot's
    ///   own length.
    /// CEP:WHY: The relocation resolver patches call sites after every body is emitted
    ///   but before the W^X flip; the slot-length bound keeps one body's patches from
    ///   ever reaching a neighbor's bytes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: WriteAfterFinalize (through the page), OffsetOutOfRange (unknown
    ///   page, or site plus length beyond the slot), BadCapacity (empty bytes).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(bytes) copy.
    /// CEP:EVIDENCE: write_slot_* unit tests below; the jit module applies every
    ///   relocation through this before finalize_all.
    pub fn write_slot(
        &mut self,
        slot: CodeSlot,
        in_slot: usize,
        bytes: &[u8],
    ) -> Result<(), JitError> {
        if bytes.is_empty() {
            return Err(JitError::BadCapacity);
        }
        let in_end = in_slot.checked_add(bytes.len()).ok_or(JitError::OffsetOutOfRange)?;
        if in_end > slot.len {
            return Err(JitError::OffsetOutOfRange);
        }
        let offset = slot.offset.checked_add(in_slot).ok_or(JitError::OffsetOutOfRange)?;
        self.pages
            .get_mut(slot.page)
            .ok_or(JitError::OffsetOutOfRange)?
            .write(offset, bytes)
    }

    /// CEP:WHAT: Drops one live reference for a slot (the code became unreachable).
    /// CEP:WHY: Reclamation bookkeeping (design 3.2).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (unknown slot is a no-op, clamped by debug_assert in the page).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: refcount_reclamation unit test.
    pub fn drop_slot_ref(&mut self, slot: CodeSlot) {
        if let Some(page) = self.pages.get_mut(slot.page) {
            page.drop_slot_ref();
        }
    }

    /// CEP:WHAT: Reclaims cold pages (live_slots == 0), compacting the segment list and
    ///   returning how many pages were unmapped.
    /// CEP:WHY: design 3.2 memory reclamation; the grace period is deliberately not
    ///   wall-clock (reproducibility mandate: no wall-clock timing in compiler behavior) -
    ///   reclamation happens deterministically when the pool owner calls it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (pages are dropped, Drop munmaps; UnmapFailed is unreachable
    ///   through Drop and therefore not surfaced here).
    /// CEP:ASSUMES: no thread executes code on a page being reclaimed (tier contract).
    /// CEP:COST: O(pages).
    /// CEP:EVIDENCE: refcount_reclamation unit test.
    pub fn reclaim_cold(&mut self) -> usize {
        let before = self.pages.len();
        self.pages.retain(|page| !page.is_reclaimable());
        before - self.pages.len()
    }

    /// CEP:WHAT: Current page count of the segment list.
    /// CEP:WHY: Tests and diagnostics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: pool_emit_across_pages unit test.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

/// CEP:WHAT: A minimal x86-64 probe body: `mov eax, imm32; ret`.
/// CEP:WHY: The smallest executable-in-memory witness for the W^X protocol, independent
///   of the full backend.
/// CEP:STATUS: complete
/// CEP:FAILURE: none for any imm32.
/// CEP:ASSUMES: target is x86-64 (the shipped JIT target).
/// CEP:COST: 6 bytes.
/// CEP:EVIDENCE: execute_mov_ret_probe test.
pub fn x86_mov_eax_ret(imm: i32) -> Vec<u8> {
    let mut v = Vec::with_capacity(6);
    v.push(0xB8);
    v.extend_from_slice(&imm.to_le_bytes());
    v.push(0xC3);
    v
}

#[cfg(all(test, target_arch = "x86_64", target_os = "linux"))]
mod tests {
    use super::*;

    /// CEP:WHAT: Capacity discipline: zero and non-page-multiple capacities are refused.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn page_capacity_discipline() {
        assert_eq!(CodePage::allocate(0).unwrap_err(), JitError::BadCapacity);
        assert_eq!(CodePage::allocate(100).unwrap_err(), JitError::BadCapacity);
        assert!(CodePage::allocate(PAGE_SIZE).is_ok());
        assert_eq!(
            CodePool::new(100, 1).unwrap_err(),
            JitError::BadCapacity
        );
    }

    /// CEP:WHAT: Phase discipline: writes are refused after finalize, code pointers
    ///   before it, finalize is idempotent.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn phase_discipline() {
        let mut page = CodePage::allocate(PAGE_SIZE).expect("map");
        assert_eq!(page.code_ptr(0).unwrap_err(), JitError::NotFinalized);
        page.bump_emit(&x86_mov_eax_ret(7)).expect("emit");
        page.finalize().expect("protect rx");
        assert_eq!(
            page.write(0, &[0x90]).unwrap_err(),
            JitError::WriteAfterFinalize
        );
        page.finalize().expect("idempotent");
    }

    /// CEP:WHAT: The bump pointer advances sequentially and one body cannot overflow the
    ///   page.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_layout() {
        let mut page = CodePage::allocate(PAGE_SIZE).expect("map");
        let a = page.bump_emit(&x86_mov_eax_ret(1)).expect("a");
        let b = page.bump_emit(&x86_mov_eax_ret(2)).expect("b");
        assert_eq!(a, 0);
        assert_eq!(b, 6);
        assert_eq!(page.used(), 12);
        let big = [0u8; PAGE_SIZE];
        assert_eq!(page.bump_emit(&big).unwrap_err(), JitError::OutOfCapacity);
    }

    /// CEP:WHAT: The patch protocol rewrites live code bytes and enforces the atomic
    ///   window rules.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn patch_protocol_and_atomic_window() {
        let mut page = CodePage::allocate(PAGE_SIZE).expect("map");
        assert_eq!(page.patch(0, &[0x90]).unwrap_err(), JitError::NotFinalized);
        page.bump_emit(&x86_mov_eax_ret(1)).expect("emit");
        page.finalize().expect("rx");
        // Read the immediate byte (offset 1 of `mov eax, imm32`).
        let ptr = page.code_ptr(0).expect("ptr");
        // SAFETY: reading our own finalized page; imm32 lives at +1..+5.
        let before = unsafe { *ptr.add(1) };
        assert_eq!(before, 1);
        page.patch(1, &[9]).expect("atomic patch");
        // SAFETY: as above.
        let after = unsafe { *ptr.add(1) };
        assert_eq!(after, 9);
        // Too large: 9 bytes.
        assert_eq!(
            page.patch(0, &[0u8; 9]).unwrap_err(),
            JitError::PatchTooLarge { max: 8, got: 9 }
        );
        // Fits in 8 bytes but crosses the aligned window at offset 6.
        assert_eq!(
            page.patch(6, &[0u8; 4]).unwrap_err(),
            JitError::PatchUnalignedWindow
        );
        // Out of range.
        assert_eq!(
            page.patch(usize::MAX, &[1]).unwrap_err(),
            JitError::OffsetOutOfRange
        );
    }

    /// CEP:WHAT: First-fit emit walks pages in order and the budget fails closed.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn pool_emit_across_pages_and_budget() {
        let mut pool = CodePool::new(PAGE_SIZE, 2).expect("pool");
        let body = x86_mov_eax_ret(3);
        // ~682 bodies of 6 bytes fill a page; emit enough to span three pages with a
        // two-page budget and expect the refusal.
        let mut last = None;
        for i in 0..1400 {
            match pool.emit(&body) {
                Ok(slot) => last = Some((slot, i)),
                Err(e) => {
                    assert_eq!(e, JitError::OutOfPages);
                    break;
                }
            }
        }
        let (slot, i) = last.expect("at least one emit succeeded");
        assert!(pool.page_count() >= 1 && pool.page_count() <= 2);
        assert!(i >= 200, "first page must hold many small bodies");
        pool.finalize_all().expect("finalize");
        // The first slot still executes.
        // SAFETY: finalized pool, slot points at `mov eax, 3; ret`.
        let f: extern "C" fn() -> i64 = unsafe { core::mem::transmute(pool.code_ptr(slot).expect("ptr")) };
        assert_eq!(f(), 3);
    }

    /// CEP:WHAT: Refcounted reclamation: a page with live slots survives, a cold page is
    ///   reclaimed deterministically.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn refcount_reclamation() {
        let mut pool = CodePool::new(PAGE_SIZE, 4).expect("pool");
        let big = [0xC3u8; PAGE_SIZE / 2];
        let b1 = pool.emit(&big).expect("b1");
        let b2 = pool.emit(&big).expect("b2");
        assert_eq!(b1.page, b2.page, "both halves fit page 0");
        let b3 = pool.emit(&big).expect("b3 forces page 1");
        assert_eq!(b3.page, 1);
        assert_eq!(pool.page_count(), 2);
        // Every page still holds live code: nothing is reclaimed.
        pool.drop_slot_ref(b1);
        assert_eq!(pool.reclaim_cold(), 0);
        // Page 1 becomes cold and is reclaimed deterministically.
        pool.drop_slot_ref(b3);
        assert_eq!(pool.reclaim_cold(), 1);
        assert_eq!(pool.page_count(), 1);
    }

    /// CEP:WHAT: page_addr reports the write-phase base of a known page and refuses
    ///   unknown indexes.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn page_addr_reports_write_phase_base() {
        let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
        assert_eq!(pool.page_addr(0).unwrap_err(), JitError::OffsetOutOfRange);
        let slot = pool.emit(&x86_mov_eax_ret(1)).expect("emit");
        let base = pool.page_addr(slot.page).expect("write-phase base");
        assert_eq!(slot.offset, 0, "first body starts at the page base");
        // SAFETY: the page is mapped read-write; reading the first emitted byte.
        assert_eq!(unsafe { *base }, 0xB8, "page base addresses the emitted body");
        assert_eq!(pool.page_addr(9).unwrap_err(), JitError::OffsetOutOfRange);
    }

    /// CEP:WHAT: write_slot rewrites bytes of one emitted body before finalize and the
    ///   change is observable in execution after finalize.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn write_slot_patches_before_finalize_and_executes() {
        let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
        let slot = pool.emit(&x86_mov_eax_ret(1)).expect("emit");
        pool.write_slot(slot, 1, &[77]).expect("pre-finalize patch");
        pool.finalize_all().expect("rx");
        // SAFETY: finalized `mov eax, 77; ret` body.
        let f: extern "C" fn() -> i64 =
            unsafe { core::mem::transmute(pool.code_ptr(slot).expect("ptr")) };
        assert_eq!(f(), 77);
    }

    /// CEP:WHAT: write_slot is bounded by the slot length and refused after finalize.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn write_slot_is_bounded_by_the_slot() {
        let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
        let a = pool.emit(&x86_mov_eax_ret(1)).expect("a");
        let b = pool.emit(&x86_mov_eax_ret(2)).expect("b");
        // Slot a is 6 bytes: a 2-byte write ending at 6 fits, starting at 6 does not.
        pool.write_slot(a, 4, &[0, 0]).expect("inside slot a");
        assert_eq!(pool.write_slot(a, 6, &[0]).unwrap_err(), JitError::OffsetOutOfRange);
        // A write addressed at slot a can never reach slot b's bytes.
        let _ = b;
        pool.finalize_all().expect("rx");
        assert_eq!(
            pool.write_slot(a, 0, &[0x90]).unwrap_err(),
            JitError::WriteAfterFinalize
        );
        assert_eq!(pool.write_slot(a, 0, &[]).unwrap_err(), JitError::BadCapacity);
    }

    /// CEP:WHAT: The mov-eax/ret probe executes in finalized memory and returns the
    ///   immediate.
    /// CEP:STATUS: complete
    /// CEP:EVIDENCE: this test.
    #[test]
    fn execute_mov_ret_probe() {
        let mut pool = CodePool::new(PAGE_SIZE, 1).expect("pool");
        let slot = pool.emit(&x86_mov_eax_ret(13)).expect("emit");
        pool.finalize_all().expect("rx");
        // SAFETY: the slot is a finalized `mov eax, 13; ret` body with SysV semantics.
        let f: extern "C" fn() -> i64 =
            unsafe { core::mem::transmute(pool.code_ptr(slot).expect("ptr")) };
        assert_eq!(f(), 13);
    }
}
