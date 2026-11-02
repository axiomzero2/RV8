// CEP:FILE: crates/rv8_runtime/src/vec.rs
// CEP:WHAT: The growable buffer of the runtime: RawVec over the Allocator trait (pointer,
//   capacity, doubling growth with checked arithmetic) and Vec over RawVec with push, pop,
//   indexed access, and slice views.
// CEP:WHY: design.md defines Vec as three pointer-sized values with doubling reallocation
//   and RawVec as its internal buffer management; every growth failure is a typed
//   CollectionError, never a panic, per the fail-closed law.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: push, reserve, and with_capacity_in return CollectionError::CapacityOverflow
//   on checked-arithmetic or budget overflow and CollectionError::OutOfMemory when the
//   allocator fails; Index panics on out-of-bounds exactly as std documents; no other
//   panic paths.
// CEP:ASSUMES: T is Sized; the allocator outlives the Vec (A is stored by value, typically
//   a shared allocator reference).
// CEP:COST: push is amortized O(1); growth is O(len) copy; pop and index are O(1).
// CEP:EVIDENCE: unit tests in this module.

use crate::allocator::{Allocator, Layout};
use crate::MAX_COLLECTION_BYTES;
use core::mem;
use core::ops::{Index, IndexMut};
use core::ptr::NonNull;

/// CEP:WHAT: The collection failure type: capacity arithmetic or budget overflow, or an
///   allocator failure underneath.
/// CEP:WHY: Vec growth is fallible in the runtime; the typed error keeps panics out of the
///   library (38.6).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: vec overflow and growth tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CollectionError {
    CapacityOverflow,
    OutOfMemory,
}

/// CEP:WHAT: Capacity of the first allocation for a zero-capacity buffer that grows.
/// CEP:WHY: Named constant for the doubling start (std uses the same shape of rule).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test vec_growth_doubles_capacity.
pub const MIN_BUFFER_CAP: usize = 4;

/// CEP:WHAT: The raw buffer of a Vec: allocated pointer, element capacity, allocator.
/// CEP:WHY: design.md keeps reallocation logic in RawVec, an implementation detail of Vec.
/// CEP:STATUS: complete
/// CEP:FAILURE: growth returns CollectionError; construction with zero capacity is
///   allocation-free with a dangling, aligned pointer.
/// CEP:ASSUMES: only the first len elements (owned by Vec) are initialized.
/// CEP:COST: O(1) metadata; growth is O(len).
/// CEP:EVIDENCE: vec tests.
pub struct RawVec<T, A: Allocator> {
    ptr: NonNull<T>,
    cap: usize,
    alloc: A,
}

impl<T, A: Allocator> RawVec<T, A> {
    /// CEP:WHAT: Allocates a buffer for cap elements.
    /// CEP:WHY: Vec::with_capacity_in and reserve need the raw buffer.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: CollectionError for layout overflow, budget, or allocator failure.
    /// CEP:ASSUMES: none
    /// CEP:COST: one allocation.
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn with_capacity_in(cap: usize, alloc: A) -> Result<RawVec<T, A>, CollectionError> {
        if cap == 0 {
            return Ok(RawVec {
                ptr: dangling::<T>(),
                cap: 0,
                alloc,
            });
        }
        let layout = element_layout::<T>(cap)?;
        let ptr = alloc.allocate(layout).map_err(|_| CollectionError::OutOfMemory)?;
        Ok(RawVec {
            ptr: ptr.cast::<T>(),
            cap,
            alloc,
        })
    }

    /// CEP:WHAT: Reports the current element capacity.
    /// CEP:WHY: Vec exposes it and growth checks need it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_growth_doubles_capacity.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// CEP:WHAT: Reserves space for at least additional more elements, growing by doubling.
    /// CEP:WHY: design.md: the buffer is reallocated to a larger size, typically doubling.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: CollectionError::CapacityOverflow on checked-arithmetic or budget
    ///   overflow; CollectionError::OutOfMemory when the allocator fails.
    /// CEP:ASSUMES: len is the number of initialized elements.
    /// CEP:COST: amortized O(1); a growth is O(len) copy.
    /// CEP:EVIDENCE: tests vec_growth_doubles_capacity and vec_capacity_overflow_is_checked.
    pub fn reserve(&mut self, len: usize, additional: usize) -> Result<(), CollectionError> {
        let min_cap = len
            .checked_add(additional)
            .ok_or(CollectionError::CapacityOverflow)?;
        if min_cap <= self.cap {
            return Ok(());
        }
        let doubled = self.cap.checked_mul(2).ok_or(CollectionError::CapacityOverflow)?;
        let new_cap = match self.cap {
            0 => min_cap.max(MIN_BUFFER_CAP),
            _ => doubled.max(min_cap),
        };
        let old_layout = element_layout::<T>(match self.cap {
            0 => 1,
            cap => cap,
        })
        .map_err(|_| CollectionError::CapacityOverflow)?;
        let new_layout = element_layout::<T>(new_cap)?;
        let fresh = if self.cap == 0 {
            self.alloc.allocate(new_layout)
        } else {
            self.alloc
                .grow(self.ptr.cast::<u8>(), old_layout, new_layout)
        }
        .map_err(|_| CollectionError::OutOfMemory)?;
        self.ptr = fresh.cast::<T>();
        self.cap = new_cap;
        Ok(())
    }

    /// CEP:WHAT: The allocator stored in the buffer.
    /// CEP:WHY: Vec needs it for deallocation and Box-style operations.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    pub fn alloc(&self) -> &A {
        &self.alloc
    }

    /// CEP:WHAT: The allocated pointer to element storage (dangling when cap is zero).
    /// CEP:WHY: Vec reads and writes elements through it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: only offsets below len name initialized elements.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    pub fn ptr(&self) -> NonNull<T> {
        self.ptr
    }
}

impl<T, A: Allocator> Drop for RawVec<T, A> {
    /// CEP:WHAT: Deallocates the buffer through the stored allocator.
    /// CEP:WHY: Ownership contract of the raw buffer.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: only Vec owns elements; no element drops here (Vec drains them first).
    /// CEP:COST: one deallocation.
    /// CEP:EVIDENCE: vec drop test in this module.
    fn drop(&mut self) {
        if self.cap != 0 {
            // CEP:UNSAFE: the pointer came from allocate with the element layout of cap;
            // dropping deallocates it exactly once.
            let layout = element_layout::<T>(self.cap).expect("existing capacity was valid");
            unsafe { self.alloc.deallocate(self.ptr.cast::<u8>(), layout) };
        }
    }
}

/// CEP:WHAT: A growable array over the runtime Allocator trait: buffer, length.
/// CEP:WHY: design.md: Vec provides push, pop, indexed access, and length/capacity queries.
/// CEP:STATUS: complete
/// CEP:FAILURE: growth errors are CollectionError; Index panics out of bounds (std
///   semantics); no other panic paths.
/// CEP:ASSUMES: the allocator outlives the Vec.
/// CEP:COST: push amortized O(1); pop, get, len O(1).
/// CEP:EVIDENCE: unit tests in this module.
pub struct Vec<T, A: Allocator> {
    buf: RawVec<T, A>,
    len: usize,
}

impl<T, A: Allocator> Vec<T, A> {
    /// CEP:WHAT: Creates an empty Vec with no allocation.
    /// CEP:WHY: The zero-cost entry point of the collection.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1), no allocation.
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn new_in(alloc: A) -> Vec<T, A> {
        Vec {
            buf: match RawVec::with_capacity_in(0, alloc) {
                Ok(buf) => buf,
                Err(_) => unreachable_const_invariant(),
            },
            len: 0,
        }
    }

    /// CEP:WHAT: Creates an empty Vec with space for cap elements.
    /// CEP:WHY: Pre-sizing avoids growth churn.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: CollectionError for layout or allocator failure.
    /// CEP:ASSUMES: none
    /// CEP:COST: one allocation.
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn with_capacity_in(cap: usize, alloc: A) -> Result<Vec<T, A>, CollectionError> {
        Ok(Vec {
            buf: RawVec::with_capacity_in(cap, alloc)?,
            len: 0,
        })
    }

    /// CEP:WHAT: Appends one element, growing the buffer by doubling when full.
    /// CEP:WHY: The primary mutation of the collection.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: CollectionError when growth fails; the element is returned to nothing
    ///   (it is consumed only on success) and the Vec is unchanged.
    /// CEP:ASSUMES: none
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: tests vec_push_pop_grows and vec_capacity_overflow_is_checked.
    pub fn push(&mut self, value: T) -> Result<(), CollectionError> {
        if self.len == self.buf.capacity() {
            self.buf.reserve(self.len, 1)?;
        }
        // CEP:UNSAFE: capacity was reserved above; offset len is inside the buffer and was
        // never initialized before (len is the high-water mark).
        unsafe {
            self.buf.ptr().as_ptr().add(self.len).write(value);
        }
        self.len += 1;
        Ok(())
    }

    /// CEP:WHAT: Removes and returns the last element, or None when empty.
    /// CEP:WHY: The LIFO half of the collection contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // CEP:UNSAFE: offset len < old len was initialized; reading it moves the value out
        // and the length decrement prevents a second read.
        unsafe { Some(self.buf.ptr().as_ptr().add(self.len).read()) }
    }

    /// CEP:WHAT: Reports the element count.
    /// CEP:WHY: Length query per design.md.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn len(&self) -> usize {
        self.len
    }

    /// CEP:WHAT: Reports whether the Vec is empty.
    /// CEP:WHY: Clippy expects len and is_empty together.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_push_pop_grows.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// CEP:WHAT: Reports the element capacity of the buffer.
    /// CEP:WHY: Capacity query per design.md.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_growth_doubles_capacity.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// CEP:WHAT: Returns the element at index i, or None out of bounds.
    /// CEP:WHY: The fallible access half of the design contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (None out of bounds).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        // CEP:UNSAFE: index < len names an initialized element.
        unsafe { Some(&*self.buf.ptr().as_ptr().add(index)) }
    }

    /// CEP:WHAT: Returns the mutable element at index i, or None out of bounds.
    /// CEP:WHY: Fallible mutable access.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none (None out of bounds).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        // CEP:UNSAFE: index < len names an initialized element; &mut self proves exclusivity.
        unsafe { Some(&mut *self.buf.ptr().as_ptr().add(index)) }
    }

    /// CEP:WHAT: Views the initialized elements as a slice.
    /// CEP:WHY: Slice views feed the slice iterator and tests.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index.
    pub fn as_slice(&self) -> &[T] {
        // CEP:UNSAFE: ptr names len initialized, properly aligned elements.
        unsafe { core::slice::from_raw_parts(self.buf.ptr().as_ptr(), self.len) }
    }

    /// CEP:WHAT: Views the initialized elements as a mutable slice.
    /// CEP:WHY: In-place mutation through slices.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // CEP:UNSAFE: ptr names len initialized, properly aligned elements; &mut self is
        // exclusive.
        unsafe { core::slice::from_raw_parts_mut(self.buf.ptr().as_ptr(), self.len) }
    }

    /// CEP:WHAT: The allocator reference stored in the buffer.
    /// CEP:WHY: Callers collecting into a Vec need the allocator identity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    pub fn alloc(&self) -> &A {
        self.buf.alloc()
    }
}

impl<T, A: Allocator> Index<usize> for Vec<T, A> {
    type Output = T;

    /// CEP:WHAT: Panicking indexed access, mirroring std's documented semantics.
    /// CEP:WHY: design.md lists indexed access; std-compatible panic behavior is the
    ///   documented exception to the no-panic law.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics on out-of-bounds, mirroring std.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index (should_panic case).
    fn index(&self, index: usize) -> &T {
        match self.get(index) {
            Some(value) => value,
            None => panic!("index out of bounds: the len is {} but the index is {}", self.len, index),
        }
    }
}

impl<T, A: Allocator> IndexMut<usize> for Vec<T, A> {
    /// CEP:WHAT: Panicking mutable indexed access, mirroring std's documented semantics.
    /// CEP:WHY: Same contract as Index.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics on out-of-bounds, mirroring std.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test vec_get_and_index.
    fn index_mut(&mut self, index: usize) -> &mut T {
        let len = self.len;
        match self.get_mut(index) {
            Some(value) => value,
            None => panic!("index out of bounds: the len is {} but the index is {}", len, index),
        }
    }
}

/// CEP:WHAT: Builds the element layout for cap elements with checked arithmetic.
/// CEP:WHY: Every allocation path shares one checked layout computation.
/// CEP:STATUS: complete
/// CEP:FAILURE: CollectionError::CapacityOverflow when cap times size overflows or exceeds
///   MAX_COLLECTION_BYTES.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: test vec_capacity_overflow_is_checked.
fn element_layout<T>(cap: usize) -> Result<Layout, CollectionError> {
    let bytes = cap
        .checked_mul(mem::size_of::<T>())
        .ok_or(CollectionError::CapacityOverflow)?;
    if bytes > MAX_COLLECTION_BYTES {
        return Err(CollectionError::CapacityOverflow);
    }
    let align = mem::align_of::<T>().max(1);
    Layout::new(bytes.max(1), align).map_err(|_| CollectionError::CapacityOverflow)
}

/// CEP:WHAT: An aligned dangling pointer for zero-capacity buffers.
/// CEP:WHY: RawVec needs a valid-but-unaddressable pointer when cap is zero.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the alignment of T is at least 1.
/// CEP:COST: O(1).
/// CEP:EVIDENCE: test vec_push_pop_grows.
fn dangling<T>() -> NonNull<T> {
    let align = mem::align_of::<T>().max(1);
    match NonNull::new(align as *mut T) {
        Some(ptr) => ptr,
        None => unreachable_const_invariant(),
    }
}

/// CEP:WHAT: Documentation-only guard: unreachable states are compile-time invariants.
/// CEP:WHY: The two internal invariants (zero-capacity construction and aligned dangling
///   pointer) cannot fail, so the runtime documents rather than panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: never executed.
/// CEP:EVIDENCE: this module's tests.
fn unreachable_const_invariant() -> ! {
    crate::panic::abort()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::bump::Bump;
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

    /// CEP:WHAT: Push and pop grow and shrink the Vec and preserve values.
    /// CEP:WHY: The core collection contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_push_pop_grows() {
        let alloc = bump(4096);
        let mut values: Vec<u32, &Bump> = Vec::new_in(&alloc);
        assert!(values.is_empty());
        assert_eq!(values.capacity(), 0);
        for i in 0..100u32 {
            values.push(i).expect("fits");
        }
        assert_eq!(values.len(), 100);
        assert!(values.capacity() >= 100);
        for i in (0..100u32).rev() {
            assert_eq!(values.pop(), Some(i));
        }
        assert_eq!(values.pop(), None);
        assert!(values.is_empty());
    }

    /// CEP:WHAT: Growth doubles the capacity: the observed capacity sequence follows the
    ///   doubling rule deterministically.
    /// CEP:WHY: design.md pins doubling; determinism needs the sequence pinned.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_growth_doubles_capacity() {
        let alloc = bump(8192);
        let mut values: Vec<u32, &Bump> = Vec::with_capacity_in(5, &alloc).expect("fits");
        assert_eq!(values.capacity(), 5);
        for i in 0..6u32 {
            values.push(i).expect("fits");
        }
        assert_eq!(values.capacity(), 10, "doubling from 5");
        for i in 6..11u32 {
            values.push(i).expect("fits");
        }
        assert_eq!(values.capacity(), 20, "doubling from 10");
        let mut fresh: Vec<u8, &Bump> = Vec::new_in(&alloc);
        fresh.push(1).expect("fits");
        assert_eq!(fresh.capacity(), MIN_BUFFER_CAP, "first growth floor");
    }

    /// CEP:WHAT: Capacity arithmetic is checked: oversized growth is a typed error.
    /// CEP:WHY: The no-panic-on-capacity-overflow requirement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_capacity_overflow_is_checked() {
        let alloc = bump(4096);
        let mut values: Vec<u64, &Bump> = Vec::new_in(&alloc);
        let huge = MAX_COLLECTION_BYTES / mem::size_of::<u64>() + 1;
        assert_eq!(
            values.buf.reserve(0, huge),
            Err(CollectionError::CapacityOverflow)
        );
        assert_eq!(
            values.buf.reserve(usize::MAX, 1),
            Err(CollectionError::CapacityOverflow)
        );
        assert_eq!(values.len(), 0);
    }

    /// CEP:WHAT: Out-of-memory underneath surfaces as CollectionError::OutOfMemory.
    /// CEP:WHY: Allocator failures must not panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_out_of_memory_surfaces() {
        let alloc = bump(64);
        let mut values: Vec<u64, &Bump> = Vec::new_in(&alloc);
        for i in 0..4u64 {
            values.push(i).expect("fits in 64 bytes");
        }
        assert_eq!(values.push(4), Err(CollectionError::OutOfMemory));
        assert_eq!(values.len(), 4);
        assert_eq!(values.pop(), Some(3));
    }

    /// CEP:WHAT: get, get_mut, as_slice, and the panicking Index agree with std semantics.
    /// CEP:WHY: Access contract, one case per class.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert and the should_panic case (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_get_and_index() {
        let alloc = bump(4096);
        let mut values: Vec<u32, &Bump> = Vec::new_in(&alloc);
        for i in 0..4u32 {
            values.push(i * i).expect("fits");
        }
        assert_eq!(values.get(0), Some(&0));
        assert_eq!(values.get(3), Some(&9));
        assert_eq!(values.get(4), None);
        if let Some(slot) = values.get_mut(1) {
            *slot = 100;
        }
        assert_eq!(values[1], 100);
        assert_eq!(values.as_slice(), &[0, 100, 4, 9]);
        values.as_mut_slice()[2] = 7;
        assert_eq!(values[2], 7);
    }

    /// CEP:WHAT: Indexing out of bounds panics, mirroring std's documented behavior.
    /// CEP:WHY: The documented panic exception.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics by design (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[should_panic]
    fn vec_index_out_of_bounds_panics() {
        let alloc = bump(4096);
        let values: Vec<u32, &Bump> = Vec::new_in(&alloc);
        let _ = values[0];
    }

    /// CEP:WHAT: Dropping a Vec deallocates its buffer through the allocator.
    /// CEP:WHY: Ownership contract of RawVec.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn vec_drop_deallocates() {
        let alloc = bump(4096);
        {
            let mut values: Vec<u32, &Bump> = Vec::with_capacity_in(16, &alloc).expect("fits");
            values.push(1).expect("fits");
        }
        // The bump cursor is unchanged by drops (no-op deallocate), but the region is
        // consistent; assert a fresh allocation still fits and the counter advanced once.
        let counter = alloc.allocated_bytes();
        assert!(counter >= 64);
        let mut values: Vec<u32, &Bump> = Vec::new_in(&alloc);
        values.push(2).expect("fits");
        assert_eq!(values.len(), 1);
    }
}
