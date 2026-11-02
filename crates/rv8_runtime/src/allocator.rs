// CEP:FILE: crates/rv8_runtime/src/allocator.rs
// CEP:WHAT: The runtime allocator contract: Layout (size plus power-of-two alignment),
//   AllocError, and the Allocator trait with allocate, deallocate, grow, and shrink over
//   NonNull<u8>.
// CEP:WHY: design.md defines the allocator interface taken by Box, Vec, and the bare-metal
//   Bump; every method takes a Layout and every failure path returns AllocError, so the
//   runtime never panics on allocation failure.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: Layout::new rejects zero size or alignment, non-power-of-two alignment, and
//   sizes beyond MAX_LAYOUT_SIZE; grow and shrink return AllocError when the allocator
//   cannot satisfy the request; no panic paths.
// CEP:ASSUMES: allocators are used from one thread (no interior synchronization required by
//   this contract); pointers handed to deallocate, grow, and shrink come from a prior
//   allocate of the same allocator.
// CEP:COST: allocate/deallocate are allocator-defined; the default grow and shrink are one
//   allocate plus one copy plus one deallocate.
// CEP:EVIDENCE: unit tests in this module; bump and vec tests.

use core::fmt;
use core::ptr::NonNull;

/// CEP:WHAT: Upper bound on the alignment of one runtime allocation.
/// CEP:WHY: Named budget (38.12); one page is the largest alignment any bare-metal target
///   of the runtime requires.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test layout_rejects_invalid.
pub const MAX_LAYOUT_ALIGN: usize = 1 << 12;

/// CEP:WHAT: Upper bound on the size of one runtime allocation.
/// CEP:WHY: Named budget (38.12); keeps checked arithmetic and allocators bounded.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test layout_rejects_invalid.
pub const MAX_LAYOUT_SIZE: usize = 1 << 30;

/// CEP:WHAT: One memory block request: size in bytes and alignment in bytes.
/// CEP:WHY: Every Allocator method takes a Layout per design.md.
/// CEP:STATUS: complete
/// CEP:FAILURE: constructed only through Layout::new, which rejects invalid pairs.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: test layout_rejects_invalid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    size: usize,
    align: usize,
}

impl Layout {
    /// CEP:WHAT: Validates and constructs a layout: nonzero size, nonzero power-of-two
    ///   alignment within MAX_LAYOUT_ALIGN, and size within MAX_LAYOUT_SIZE.
    /// CEP:WHY: Fail-closed law (38.6); invalid layouts are rejected, never coerced.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError::InvalidLayout for any invalid pair.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test layout_rejects_invalid.
    pub fn new(size: usize, align: usize) -> Result<Layout, AllocError> {
        if size == 0 || align == 0 || size > MAX_LAYOUT_SIZE || align > MAX_LAYOUT_ALIGN {
            return Err(AllocError::InvalidLayout);
        }
        if align & (align - 1) != 0 {
            return Err(AllocError::InvalidLayout);
        }
        Ok(Layout { size, align })
    }

    /// CEP:WHAT: Reports the block size in bytes.
    /// CEP:WHY: Allocators and growth arithmetic need the size.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test layout_rejects_invalid.
    pub fn size(&self) -> usize {
        self.size
    }

    /// CEP:WHAT: Reports the block alignment in bytes.
    /// CEP:WHY: Allocators align returned pointers to this value.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test layout_rejects_invalid.
    pub fn align(&self) -> usize {
        self.align
    }
}

/// CEP:WHAT: Rounds one offset up to the next multiple of a power-of-two alignment.
/// CEP:WHY: Bump allocation and layout math share this single checked helper.
/// CEP:STATUS: complete
/// CEP:FAILURE: AllocError::InvalidLayout when the addition overflows usize.
/// CEP:ASSUMES: align is a power of two.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: test layout_rejects_invalid.
pub fn align_up(value: usize, align: usize) -> Result<usize, AllocError> {
    match value.checked_add(align - 1) {
        Some(masked) => Ok(masked & !(align - 1)),
        None => Err(AllocError::InvalidLayout),
    }
}

/// CEP:WHAT: The allocation failure type of the runtime.
/// CEP:WHY: Fail-closed law (38.6): allocation failure is a typed value, never a panic.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: bump exhaustion and vec overflow tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllocError {
    OutOfMemory,
    InvalidLayout,
}

impl fmt::Display for AllocError {
    /// CEP:WHAT: Renders the error in one stable line.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: this module's tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AllocError::OutOfMemory => write!(f, "out of memory"),
            AllocError::InvalidLayout => write!(f, "invalid layout"),
        }
    }
}

/// CEP:WHAT: The allocator interface: allocate, deallocate, grow, and shrink, each over a
///   Layout, with default grow and shrink implemented as allocate-copy-deallocate.
/// CEP:WHY: design.md defines exactly these four methods; Box, Vec, and Bump share it.
/// CEP:STATUS: complete
/// CEP:FAILURE: allocate, grow, and shrink return AllocError; deallocate is infallible.
/// CEP:ASSUMES: pointer provenance rules documented on each method; single-threaded use.
/// CEP:COST: allocator-defined; defaults are one allocate plus one copy plus one deallocate.
/// CEP:EVIDENCE: bump tests (allocate, exhaustion) and vec tests (grow through the trait).
pub trait Allocator {
    /// CEP:WHAT: Allocates one zeroed-unspecified block of the layout, aligned.
    /// CEP:WHY: The primitive request of Box, Vec, and Bump users.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError when the request cannot be satisfied; contents unspecified.
    /// CEP:ASSUMES: none
    /// CEP:COST: allocator-defined.
    /// CEP:EVIDENCE: bump tests.
    fn allocate(&self, layout: Layout) -> Result<NonNull<u8>, AllocError>;

    /// CEP:WHAT: Releases one block previously returned by this allocator.
    /// CEP:WHY: The deallocation half of the contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (infallible by contract).
    /// CEP:ASSUMES: safety contract of the caller: ptr is live from this allocator with the
    ///   same layout, and is not deallocated twice.
    /// CEP:COST: allocator-defined (no-op for Bump).
    /// CEP:EVIDENCE: bump tests (deallocation never reclaims).
    ///
    /// # Safety
    ///
    /// The caller guarantees `ptr` was returned by this allocator's `allocate` (or `grow`
    /// / `shrink`) with the same layout, is still live, and is not deallocated twice.
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout);

    /// CEP:WHAT: Grows one block to a larger layout, returning the new pointer and copying
    ///   the overlap.
    /// CEP:WHY: Vec growth goes through this method.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError when the request cannot be satisfied; the old block stays
    ///   valid and must still be deallocated by the caller in that case.
    /// CEP:ASSUMES: safety contract of deallocate for the old block on success.
    /// CEP:COST: default is one allocate plus one copy plus one deallocate.
    /// CEP:EVIDENCE: vec growth tests over Bump.
    fn grow(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<u8>, AllocError> {
        let fresh = self.allocate(new)?;
        let overlap = match old.size() < new.size() {
            true => old.size(),
            false => new.size(),
        };
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.as_ptr(), fresh.as_ptr(), overlap);
            self.deallocate(ptr, old);
        }
        Ok(fresh)
    }

    /// CEP:WHAT: Shrinks one block to a smaller layout, returning the (possibly moved)
    ///   pointer.
    /// CEP:WHY: The shrink half of the design contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError when the request cannot be satisfied; the old block stays
    ///   valid and must still be deallocated by the caller in that case.
    /// CEP:ASSUMES: safety contract of deallocate for the old block on success.
    /// CEP:COST: default is one allocate plus one copy plus one deallocate.
    /// CEP:EVIDENCE: vec shrink path compiles and runs in tests.
    fn shrink(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<u8>, AllocError> {
        let fresh = self.allocate(new)?;
        let overlap = match old.size() < new.size() {
            true => old.size(),
            false => new.size(),
        };
        unsafe {
            core::ptr::copy_nonoverlapping(ptr.as_ptr(), fresh.as_ptr(), overlap);
            self.deallocate(ptr, old);
        }
        Ok(fresh)
    }
}

impl<A: Allocator> Allocator for &A {
    /// CEP:WHAT: Forwards the allocate call through a shared reference.
    /// CEP:WHY: Box and Vec store the allocator by value; passing a shared Bump keeps one
    ///   bump cursor across every collection.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: forwarded verbatim.
    /// CEP:ASSUMES: none
    /// CEP:COST: one call.
    /// CEP:EVIDENCE: vec and boxed tests pass &Bump.
    fn allocate(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        (**self).allocate(layout)
    }

    /// CEP:WHAT: Forwards the deallocate call through a shared reference.
    /// CEP:WHY: Same provenance rule as the inner allocator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: forwarded verbatim.
    /// CEP:ASSUMES: safety contract of the inner deallocate.
    /// CEP:COST: one call.
    /// CEP:EVIDENCE: boxed tests drop Box over &Bump.
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        (**self).deallocate(ptr, layout)
    }

    /// CEP:WHAT: Forwards the grow call through a shared reference.
    /// CEP:WHY: Vec growth over a shared Bump updates the shared cursor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: forwarded verbatim.
    /// CEP:ASSUMES: none
    /// CEP:COST: forwarded.
    /// CEP:EVIDENCE: vec growth tests.
    fn grow(&self, ptr: NonNull<u8>, old: Layout, new: Layout) -> Result<NonNull<u8>, AllocError> {
        (**self).grow(ptr, old, new)
    }

    /// CEP:WHAT: Forwards the shrink call through a shared reference.
    /// CEP:WHY: Completes the forwarding surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: forwarded verbatim.
    /// CEP:ASSUMES: none
    /// CEP:COST: forwarded.
    /// CEP:EVIDENCE: this module's tests.
    fn shrink(
        &self,
        ptr: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<u8>, AllocError> {
        (**self).shrink(ptr, old, new)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Layout validation rejects every invalid pair and accepts valid ones.
    /// CEP:WHY: Fail-closed layout contract, one case per rejection class.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn layout_rejects_invalid() {
        assert_eq!(Layout::new(0, 8), Err(AllocError::InvalidLayout));
        assert_eq!(Layout::new(8, 0), Err(AllocError::InvalidLayout));
        assert_eq!(Layout::new(8, 3), Err(AllocError::InvalidLayout));
        assert_eq!(Layout::new(8, MAX_LAYOUT_ALIGN * 2), Err(AllocError::InvalidLayout));
        assert_eq!(
            Layout::new(MAX_LAYOUT_SIZE + 1, 8),
            Err(AllocError::InvalidLayout)
        );
        let layout = Layout::new(16, 8).expect("valid");
        assert_eq!(layout.size(), 16);
        assert_eq!(layout.align(), 8);
        assert_eq!(align_up(5, 8), Ok(8));
        assert_eq!(align_up(8, 8), Ok(8));
        assert_eq!(align_up(usize::MAX, 8), Err(AllocError::InvalidLayout));
    }

    /// CEP:WHAT: The shared-reference forwarding keeps one allocator identity.
    /// CEP:WHY: Box and Vec hold A by value; &Bump must reach the same cursor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn shared_reference_forwards() {
        let mut buffer = alloc_vec();
        let bump = crate::bump::Bump::new(
            core::ptr::NonNull::new(buffer.as_mut_ptr()).expect("nonzero"),
            buffer.len(),
        );
        let reference = &bump;
        let layout = Layout::new(16, 8).expect("valid");
        let first = Allocator::allocate(reference, layout).expect("fits");
        let second = Allocator::allocate(&reference, layout).expect("fits");
        let a = first.as_ptr() as usize;
        let b = second.as_ptr() as usize;
        assert!(b >= a + 16, "sequential allocations do not overlap");
    }

    /// CEP:WHAT: Builds one aligned scratch buffer for allocator tests.
    /// CEP:WHY: Shared fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    pub fn alloc_vec() -> std::vec::Vec<u8> {
        std::vec![0u8; 4096]
    }
}
