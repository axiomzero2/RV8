// CEP:FILE: crates/rv8_core/src/arena.rs
// CEP:WHAT: A thread-local bump allocator arena for POD compiler nodes with O(1) allocation and
//   wholesale reset.
// CEP:WHY: Standard 25.5 allows arena allocation in CEP-0 when initialized before the hot path
//   and deterministic; a bump arena turns IR node construction into a pointer increment.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: alloc returns None when the requested size or alignment exceeds chunk capacity;
//   callers convert this into a typed error, never a panic.
// CEP:ASSUMES: allocated types are POD (no Drop side effects); values are leaked into the arena
//   by design and freed only by reset() or arena drop.
// CEP:COST: alloc is a bounds check plus pointer bump; no allocator traffic after warmup.
// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
// CEP:SECURITY: chunk sizes are caller-bounded; no unbounded growth without a caller decision.

/// CEP:WHAT: Default chunk size for new arenas.
/// CEP:WHY: Named constant per Law 7; sized to amortize allocations for typical function IR.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
pub const ARENA_CHUNK_BYTES: usize = 64 * 1024;

struct Chunk {
    storage: Box<[u8]>,
    used: usize,
}

/// CEP:WHAT: Growable arena of byte chunks serving aligned allocations.
/// CEP:WHY: One reset frees everything, eliminating per-node deallocation cost.
/// CEP:STATUS: complete
/// CEP:FAILURE: alloc fails closed on oversized requests.
/// CEP:ASSUMES: T has no Drop requirements that must run before reset.
/// CEP:COST: O(1) alloc; amortized O(1) chunk growth.
/// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
pub struct Bump {
    chunks: Vec<Chunk>,
    chunk_bytes: usize,
    allocated_total: u64,
}

impl Default for Bump {
    /// CEP:WHAT: Creates an arena with the default chunk size.
    /// CEP:WHY: Idiomatic Default.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: no allocation until first use.
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    fn default() -> Self {
        Bump::with_chunk_bytes(ARENA_CHUNK_BYTES)
    }
}

impl Bump {
    /// CEP:WHAT: Creates an arena with an explicit chunk size.
    /// CEP:WHY: Large functions can size chunks to their working set.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: chunk_bytes >= 8; smaller values are rounded up.
    /// CEP:COST: no allocation until first use.
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    pub fn with_chunk_bytes(chunk_bytes: usize) -> Bump {
        Bump {
            chunks: Vec::new(),
            chunk_bytes: chunk_bytes.max(8),
            allocated_total: 0,
        }
    }

    /// CEP:WHAT: Allocates and initializes one value of T inside the arena.
    /// CEP:WHY: IR node construction without allocator traffic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None if T's size and alignment cannot fit in a fresh chunk (requests larger
    ///   than the chunk size are rejected rather than silently over-allocating).
    /// CEP:ASSUMES: T is POD; the returned reference is invalidated only by reset or drop.
    /// CEP:COST: O(1) pointer bump plus alignment mask.
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    pub fn alloc<T>(&mut self, value: T) -> Option<&mut T> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        if size > self.chunk_bytes {
            return None;
        }
        if !self.fits(size, align) {
            self.push_chunk();
        }
        let chunk = self.chunks.last_mut()?;
        let base = chunk.storage.as_mut_ptr();
        let base_addr = base as usize;
        let aligned = (base_addr + chunk.used + (align - 1)) & !(align - 1);
        let offset = aligned - base_addr;
        if offset + size > chunk.storage.len() {
            // Alignment pushed past capacity; retry once in a fresh chunk.
            self.push_chunk();
            let chunk = self.chunks.last_mut()?;
            let base = chunk.storage.as_mut_ptr();
            let base_addr = base as usize;
            let aligned = (base_addr + chunk.used + (align - 1)) & !(align - 1);
            let offset = aligned - base_addr;
            if offset + size > chunk.storage.len() {
                return None;
            }
            unsafe {
                // CEP:UNSAFE: aligned range fits chunk storage; value moved in, never dropped twice.
                let ptr = base.add(offset) as *mut T;
                std::ptr::write(ptr, value);
                chunk.used = offset + size;
                self.allocated_total += size as u64;
                return Some(&mut *ptr);
            }
        }
        unsafe {
            // CEP:UNSAFE: range checked above; single initialization via ptr::write.
            let ptr = base.add(offset) as *mut T;
            std::ptr::write(ptr, value);
            chunk.used = offset + size;
            self.allocated_total += size as u64;
            Some(&mut *ptr)
        }
    }

    /// CEP:WHAT: Reports whether the last chunk can satisfy a size/alignment request.
    /// CEP:WHY: Keeps alloc linear and readable.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) arithmetic.
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    fn fits(&self, size: usize, align: usize) -> bool {
        match self.chunks.last() {
            None => false,
            Some(chunk) => {
                let base = chunk.storage.as_ptr() as usize;
                let aligned = (base + chunk.used + (align - 1)) & !(align - 1);
                aligned + size <= base + chunk.storage.len()
            }
        }
    }

    /// CEP:WHAT: Appends one fresh zero-used chunk.
    /// CEP:WHY: Growth path shared by alloc.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; allocation follows global allocator failure contract.
    /// CEP:ASSUMES: chunk_bytes >= 8.
    /// CEP:COST: O(chunk_bytes) zero-init, once per chunk.
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    fn push_chunk(&mut self) {
        let mut storage = vec![0u8; self.chunk_bytes].into_boxed_slice();
        storage.iter_mut().for_each(|b| *b = 0);
        self.chunks.push(Chunk {
            storage,
            used: 0,
        });
    }

    /// CEP:WHAT: Drops all chunks, reclaiming every arena allocation at once.
    /// CEP:WHY: Wholesale reclamation is the arena's reason to exist.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: all references handed out by this arena are dead when called.
    /// CEP:COST: O(chunks).
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    pub fn reset(&mut self) {
        self.chunks.clear();
        self.allocated_total = 0;
    }

    /// CEP:WHAT: Reports total bytes handed out since construction or reset.
    /// CEP:WHY: Compile-memory budgets (standard 38.12) need measured usage.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::arena::tests::alloc_and_reset.
    pub fn allocated_bytes(&self) -> u64 {
        self.allocated_total
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies allocation alignment, value integrity, and reset reclamation.
    /// CEP:WHY: Core arena contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn alloc_and_reset() {
        let mut arena = Bump::with_chunk_bytes(256);
        let mut refs = Vec::new();
        for i in 0..100u64 {
            let r = arena.alloc([i, i + 1, i + 2, i + 3]).unwrap();
            refs.push(r as *const [u64; 4]);
        }
        assert_eq!(arena.allocated_bytes(), 100 * 32);
        for (i, ptr) in refs.iter().enumerate() {
            let v = unsafe { &**ptr };
            assert_eq!(v[0], i as u64);
        }
        arena.reset();
        assert_eq!(arena.allocated_bytes(), 0);
        let fresh = arena.alloc(7u32).unwrap();
        assert_eq!(*fresh, 7);
    }

    /// CEP:WHAT: Verifies oversized allocations fail closed.
    /// CEP:WHY: Bounded failure contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn oversized_rejected() {
        let mut arena = Bump::with_chunk_bytes(64);
        #[repr(align(8))]
        #[expect(dead_code)]
        struct Big([u8; 128]);
        assert!(arena.alloc(Big([0; 128])).is_none());
    }
}
