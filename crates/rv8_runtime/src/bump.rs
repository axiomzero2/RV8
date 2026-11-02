// CEP:FILE: crates/rv8_runtime/src/bump.rs
// CEP:WHAT: The bare-metal bump allocator: a fixed-region, alignment-respecting bump
//   pointer with no-op deallocation, implementing the runtime Allocator trait.
// CEP:WHY: design.md pins the bare-metal allocator to a fixed-size region whose pointer
//   advances as memory is allocated and whose deallocation is a no-op; memory is reclaimed
//   only by reset or region end.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: allocate fails closed with AllocError::OutOfMemory when the region cannot
//   satisfy the aligned request; no panic paths.
// CEP:ASSUMES: the region handed to new is valid, exclusive, and lives as long as the Bump;
//   single-threaded use (the cursor is an unsynchronized cell); alignment requests are
//   bounded by MAX_LAYOUT_ALIGN through Layout.
// CEP:COST: allocate is a bounds check plus an align-up plus a cursor advance, O(1);
//   deallocate is O(1) no-op; grow and shrink use the trait defaults (fresh block plus copy).
// CEP:EVIDENCE: unit tests in this module.

use crate::allocator::{align_up, AllocError, Allocator, Layout};
use core::cell::Cell;
use core::ptr::NonNull;

/// CEP:WHAT: A bump allocator over one caller-provided fixed region.
/// CEP:WHY: Generated bare-metal programs need allocation without an OS or libc.
/// CEP:STATUS: complete
/// CEP:FAILURE: allocate fails closed on exhaustion.
/// CEP:ASSUMES: region validity and exclusivity are the caller's contract; interior
///   mutability is unsynchronized.
/// CEP:COST: O(1) per operation.
/// CEP:EVIDENCE: unit tests in this module.
pub struct Bump {
    base: NonNull<u8>,
    len: usize,
    offset: Cell<usize>,
    allocated_total: Cell<u64>,
}

impl Bump {
    /// CEP:WHAT: Creates a bump allocator over the byte region [region, region + len).
    /// CEP:WHY: Bare-metal programs hand one static or stack region to the runtime.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (region validity is a caller contract, documented in ASSUMES).
    /// CEP:ASSUMES: region points to len valid, exclusive bytes for the Bump's lifetime;
    ///   len may be zero (every allocation then fails closed).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test bump_aligns_and_advances.
    pub fn new(region: NonNull<u8>, len: usize) -> Bump {
        Bump {
            base: region,
            len,
            offset: Cell::new(0),
            allocated_total: Cell::new(0),
        }
    }

    /// CEP:WHAT: Reports the number of bytes handed out since construction or reset.
    /// CEP:WHY: Memory budgets (38.12) need measured usage.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test bump_exhaustion_fails_closed.
    pub fn allocated_bytes(&self) -> u64 {
        self.allocated_total.get()
    }

    /// CEP:WHAT: Reports the number of bytes not yet handed out.
    /// CEP:WHY: Bare-metal programs size their regions against this.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test bump_exhaustion_fails_closed.
    pub fn remaining_bytes(&self) -> usize {
        self.len - self.offset.get()
    }

    /// CEP:WHAT: Reclaims the whole region at once by rewinding the cursor.
    /// CEP:WHY: design.md: bump memory is reclaimed on program exit or wholesale reset.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: every pointer handed out by this Bump is dead when reset is called.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test bump_reset_reclaims.
    pub fn reset(&self) {
        self.offset.set(0);
        self.allocated_total.set(0);
    }
}

impl Allocator for Bump {
    /// CEP:WHAT: Bump-allocates one block: aligns the cursor, checks the region bound, and
    ///   advances.
    /// CEP:WHY: The allocation half of the Allocator contract for bare metal.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError::OutOfMemory when the aligned block does not fit the region.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: tests bump_aligns_and_advances and bump_exhaustion_fails_closed.
    fn allocate(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        // Alignment applies to the ADDRESS (base + offset), not the offset alone: the
        // region base has no alignment guarantee beyond 1.
        let cursor_addr = self.base.as_ptr() as usize + self.offset.get();
        let aligned_addr = align_up(cursor_addr, layout.align())?;
        let aligned = match aligned_addr.checked_sub(self.base.as_ptr() as usize) {
            Some(offset) => offset,
            None => return Err(AllocError::InvalidLayout),
        };
        let end = match aligned.checked_add(layout.size()) {
            Some(end) => end,
            None => return Err(AllocError::OutOfMemory),
        };
        if end > self.len {
            return Err(AllocError::OutOfMemory);
        }
        self.offset.set(end);
        self.allocated_total
            .set(self.allocated_total.get().saturating_add(layout.size() as u64));
        // CEP:UNSAFE: the range [aligned, end) fits the region by the bound check above;
        // the region pointer is valid for len bytes by the constructor contract.
        let ptr = unsafe { self.base.as_ptr().add(aligned) };
        Ok(match NonNull::new(ptr) {
            Some(ptr) => ptr,
            None => return Err(AllocError::OutOfMemory),
        })
    }

    /// CEP:WHAT: Deallocates nothing: bump regions are reclaimed only by reset or exit.
    /// CEP:WHY: design.md: deallocation is a no-op for the bare-metal bump allocator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: the pointer came from this allocator's allocate.
    /// CEP:COST: O(1) no-op.
    /// CEP:EVIDENCE: test bump_deallocate_is_noop.
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        // Deliberate no-op: bump regions are reclaimed only by reset or program exit.
        let _ = (ptr, layout);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Builds one scratch region and a Bump over it.
    /// CEP:WHY: Shared fixture; the owning vector is leaked so the region outlives the Bump.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn scratch(len: usize) -> Bump {
        let mut buffer = std::vec![0u8; len];
        let region = NonNull::new(buffer.as_mut_ptr()).expect("nonzero");
        let bump = Bump::new(region, buffer.len());
        std::mem::forget(buffer);
        bump
    }

    /// CEP:WHAT: Allocation respects alignments one through thirty-two in sequence and the
    ///   cursor never overlaps blocks.
    /// CEP:WHY: The alignment contract of the allocator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_aligns_and_advances() {
        let bump = scratch(4096);
        for align in [1usize, 2, 4, 8, 16, 32] {
            let layout = Layout::new(8, align).expect("valid");
            let ptr = Allocator::allocate(&bump, layout).expect("fits");
            let addr = ptr.as_ptr() as usize;
            assert_eq!(addr % align, 0, "alignment {align}");
        }
        assert!(bump.allocated_bytes() >= 8 * 6);
    }

    /// CEP:WHAT: Exhausted regions fail closed with OutOfMemory and keep prior allocations
    ///   intact.
    /// CEP:WHY: Fail-closed exhaustion contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_exhaustion_fails_closed() {
        let bump = scratch(64);
        let layout = Layout::new(16, 8).expect("valid");
        let first = Allocator::allocate(&bump, layout).expect("fits");
        // 64-byte region, 16-byte blocks: the first plus three more exhaust it exactly.
        for _ in 0..3 {
            assert!(Allocator::allocate(&bump, layout).is_ok());
        }
        assert_eq!(bump.remaining_bytes(), 0);
        assert_eq!(
            Allocator::allocate(&bump, layout),
            Err(AllocError::OutOfMemory)
        );
        // Prior allocations are untouched: the first pointer still addresses valid bytes.
        unsafe {
            core::ptr::write(first.as_ptr(), 0xAB);
            assert_eq!(core::ptr::read(first.as_ptr()), 0xAB);
        }
    }

    /// CEP:WHAT: Deallocation is a no-op: freed blocks are not reused before reset.
    /// CEP:WHY: design.md pins no-op deallocation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_deallocate_is_noop() {
        let bump = scratch(128);
        let layout = Layout::new(16, 8).expect("valid");
        let first = Allocator::allocate(&bump, layout).expect("fits");
        let before = first.as_ptr() as usize;
        unsafe { Allocator::deallocate(&bump, first, layout) };
        let second = Allocator::allocate(&bump, layout).expect("fits");
        let after = second.as_ptr() as usize;
        assert!(after >= before + 16, "dealloc must not reclaim");
    }

    /// CEP:WHAT: Reset reclaims the whole region and the counter restarts.
    /// CEP:WHY: Wholesale reclamation contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_reset_reclaims() {
        let bump = scratch(256);
        let layout = Layout::new(32, 8).expect("valid");
        for _ in 0..4 {
            Allocator::allocate(&bump, layout).expect("fits");
        }
        let used_before = bump.allocated_bytes();
        assert!(used_before >= 128);
        bump.reset();
        assert_eq!(bump.allocated_bytes(), 0);
        assert_eq!(bump.remaining_bytes(), 256);
        let fresh = Allocator::allocate(&bump, layout).expect("fits");
        assert_eq!(fresh.as_ptr() as usize % 8, 0);
    }

    /// CEP:WHAT: Growth through the trait default allocates a fresh aligned block and
    ///   preserves the copied bytes.
    /// CEP:WHY: Vec growth rides Allocator::grow over Bump.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn bump_grow_preserves_bytes() {
        let bump = scratch(1024);
        let old = Layout::new(8, 8).expect("valid");
        let new = Layout::new(32, 8).expect("valid");
        let ptr = Allocator::allocate(&bump, old).expect("fits");
        unsafe {
            core::ptr::write(ptr.as_ptr() as *mut u32, 0x5Au32);
        }
        let grown = Allocator::grow(&bump, ptr, old, new).expect("fits");
        unsafe {
            assert_eq!(core::ptr::read(grown.as_ptr() as *const u32), 0x5A);
        }
        assert_eq!(grown.as_ptr() as usize % 8, 0);
    }
}
