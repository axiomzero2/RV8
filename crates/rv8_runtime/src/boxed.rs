// CEP:FILE: crates/rv8_runtime/src/boxed.rs
// CEP:WHAT: The heap box of the runtime: Box<T, A> over the Allocator trait with Deref,
//   DerefMut, into_inner, and drop glue that runs the value's destructor and deallocates.
// CEP:WHY: design.md defines Box as one pointer-sized value whose creation calls the
//   allocator and whose scope exit deallocates and drops the contained value.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: new_in returns AllocError when the allocator fails; no panic paths.
// CEP:ASSUMES: T is Sized; the allocator reference outlives the Box (A is stored by value).
// CEP:COST: one allocation on creation, one deallocation plus drop glue on scope exit.
// CEP:EVIDENCE: unit tests in this module.

use crate::allocator::{AllocError, Allocator, Layout};
use core::mem;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

/// CEP:WHAT: An owning pointer to one value allocated through the runtime Allocator.
/// CEP:WHY: The heap-allocation primitive of generated code.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction fails closed with AllocError.
/// CEP:ASSUMES: none
/// CEP:COST: one allocation plus one deallocation per value.
/// CEP:EVIDENCE: unit tests in this module.
pub struct Box<T, A: Allocator> {
    ptr: NonNull<T>,
    alloc: A,
}

impl<T, A: Allocator> Box<T, A> {
    /// CEP:WHAT: Allocates one value through the allocator and moves it in.
    /// CEP:WHY: The Box creation contract of design.md.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AllocError::OutOfMemory or AllocError::InvalidLayout from the layout or
    ///   allocator path.
    /// CEP:ASSUMES: none
    /// CEP:COST: one allocation.
    /// CEP:EVIDENCE: test box_new_deref_and_into_inner.
    pub fn new_in(value: T, alloc: A) -> Result<Box<T, A>, AllocError> {
        let layout = Layout::new(mem::size_of::<T>().max(1), mem::align_of::<T>())?;
        let raw = alloc.allocate(layout)?;
        // CEP:UNSAFE: raw names layout.size() writable, aligned bytes from this allocator;
        // ptr::write initializes the single value exactly once.
        unsafe {
            core::ptr::write(raw.as_ptr() as *mut T, value);
        }
        Ok(Box {
            ptr: raw.cast::<T>(),
            alloc,
        })
    }

    /// CEP:WHAT: Consumes the Box and returns the contained value, deallocating the block.
    /// CEP:WHY: The extraction half of the ownership contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: one deallocation.
    /// CEP:EVIDENCE: test box_new_deref_and_into_inner.
    pub fn into_inner(self) -> T {
        let mut this = core::mem::ManuallyDrop::new(self);
        // CEP:UNSAFE: the pointer names one initialized value owned by this Box; reading it
        // moves the value out and the ManuallyDrop wrapper prevents the second drop.
        let value = unsafe { core::ptr::read(this.ptr.as_ptr()) };
        unsafe {
            this.dealloc();
        }
        value
    }

    /// CEP:WHAT: Deallocates the block (drop glue helper).
    /// CEP:WHY: Shared by Drop and into_inner.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: the pointer is live and owned.
    /// CEP:COST: one deallocation.
    /// CEP:EVIDENCE: this module's tests.
    unsafe fn dealloc(&mut self) {
        let layout = Layout::new(mem::size_of::<T>().max(1), mem::align_of::<T>())
            .expect("boxed layout was valid at creation");
        self.alloc.deallocate(self.ptr.cast::<u8>(), layout);
    }
}

impl<T, A: Allocator> Deref for Box<T, A> {
    type Target = T;

    /// CEP:WHAT: Dereferences to the contained value.
    /// CEP:WHY: Box is the smart pointer of the runtime.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test box_new_deref_and_into_inner.
    fn deref(&self) -> &T {
        // CEP:UNSAFE: the pointer names one initialized value for the Box's lifetime.
        unsafe { &*self.ptr.as_ptr() }
    }
}

impl<T, A: Allocator> DerefMut for Box<T, A> {
    /// CEP:WHAT: Mutably dereferences to the contained value.
    /// CEP:WHY: In-place mutation through the box.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test box_runs_drop_glue.
    fn deref_mut(&mut self) -> &mut T {
        // CEP:UNSAFE: the pointer names one initialized value; &mut self is exclusive.
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

impl<T, A: Allocator> Drop for Box<T, A> {
    /// CEP:WHAT: Drops the contained value and deallocates the block.
    /// CEP:WHY: design.md: scope exit deallocates the memory and calls the destructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: drop glue plus one deallocation.
    /// CEP:EVIDENCE: test box_runs_drop_glue.
    fn drop(&mut self) {
        // CEP:UNSAFE: the pointer names one initialized value; drop_in_place runs its
        // destructor exactly once and dealloc then releases the block.
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
            self.dealloc();
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::bump::Bump;
    use core::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
    use core::ptr::NonNull;

    /// CEP:WHAT: Builds a Bump over a leaked scratch region.
    /// CEP:WHY: Shared fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    fn bump(len: usize) -> Bump {
        let mut buffer = std::vec![0u8; len];
        let region = NonNull::new(buffer.as_mut_ptr()).expect("nonzero");
        let allocator = Bump::new(region, buffer.len());
        std::mem::forget(buffer);
        allocator
    }

    /// CEP:WHAT: A destructor-counting type for drop-glue tests.
    /// CEP:WHY: Drop contract needs an observable destructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: single-threaded tests.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this module.
    struct Probe {
        counter: &'static AtomicU32,
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.counter.fetch_add(1, AtomicOrdering::SeqCst);
        }
    }

    /// CEP:WHAT: Box construction, deref, mutation, and into_inner round-trip.
    /// CEP:WHY: Core Box contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn box_new_deref_and_into_inner() {
        let alloc = bump(1024);
        let boxed = Box::new_in(41u32, &alloc).expect("fits");
        assert_eq!(*boxed, 41);
        let mut boxed = boxed;
        *boxed += 1;
        assert_eq!(*boxed, 42);
        assert_eq!(boxed.into_inner(), 42);
    }

    /// CEP:WHAT: Box drop runs the value's destructor exactly once.
    /// CEP:WHY: design.md drop-glue contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn box_runs_drop_glue() {
        let alloc = bump(1024);
        static DROPS: AtomicU32 = AtomicU32::new(0);
        {
            let _boxed = Box::new_in(
                Probe {
                    counter: &DROPS,
                },
                &alloc,
            )
            .expect("fits");
            assert_eq!(DROPS.load(AtomicOrdering::SeqCst), 0);
        }
        assert_eq!(DROPS.load(AtomicOrdering::SeqCst), 1);
    }

    /// CEP:WHAT: Box allocation failure fails closed.
    /// CEP:WHY: The no-panic allocation law.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn box_out_of_memory_fails_closed() {
        let alloc = bump(16);
        assert!(matches!(
            Box::new_in([0u64; 8], &alloc),
            Err(AllocError::OutOfMemory)
        ));
    }
}
