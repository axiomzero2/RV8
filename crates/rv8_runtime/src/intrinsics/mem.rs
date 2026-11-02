// CEP:FILE: crates/rv8_runtime/src/intrinsics/mem.rs
// CEP:WHAT: The byte memory intrinsics: memcpy (bounded copy), memset (bounded fill), and
//   memcmp (lexicographic comparison), each over slices so every operation is checked.
// CEP:WHY: design.md lists memcpy, memset, and memcmp as the runtime memory operations;
//   slice-based signatures turn the unchecked C pointer/length contract into a fail-closed
//   one (standard 38.6) while the compiler can still pattern-match the bodies.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: memcpy returns MemError::DestTooSmall when the destination cannot hold the
//   source and MemError::BudgetExceeded when the request exceeds MAX_MEMOP_BYTES; memset
//   and memcmp are infallible over their slices.
// CEP:ASSUMES: memcpy's slice arguments do not overlap (the C memcpy contract; overlapping
//   callers must copy through an intermediate buffer); the budget bounds one intrinsic
//   call, not total memory.
// CEP:COST: O(n) bytewise work; no allocation.
// CEP:EVIDENCE: unit tests here and the vs-core integration test
//   crates/rv8_runtime/tests/runtime_foundation.rs.

/// CEP:WHAT: Byte budget for a single memory-intrinsic call.
/// CEP:WHY: Named budget (38.12): one intrinsic call is bounded and the bound is a named
///   constant instead of a scattered magic number.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: one gibibyte is the standing ceiling for one runtime memory operation on
///   the bare-metal targets the runtime serves.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test memcpy_rejects_oversized_requests.
pub const MAX_MEMOP_BYTES: usize = 1 << 30;

/// CEP:WHAT: The memory-intrinsic failure type.
/// CEP:WHY: Fail-closed law (38.6): a copy that cannot fit is a typed value, not a panic
///   or a silent truncation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: test memcpy_rejects_oversized_requests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemError {
    /// The destination slice cannot hold the requested copy.
    DestTooSmall,
    /// The request exceeds MAX_MEMOP_BYTES.
    BudgetExceeded,
}

impl core::fmt::Display for MemError {
    /// CEP:WHAT: Renders the error in one stable line.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: this module's tests.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MemError::DestTooSmall => write!(f, "memcpy destination too small"),
            MemError::BudgetExceeded => write!(f, "memcpy exceeds MAX_MEMOP_BYTES"),
        }
    }
}

/// CEP:WHAT: Copies src.len() bytes from src to the front of dst, checked against dst's
///   capacity and the named call budget.
/// CEP:WHY: The runtime's checked replacement for C memcpy.
/// CEP:STATUS: complete
/// CEP:FAILURE: MemError::DestTooSmall when dst.len() < src.len(); MemError::BudgetExceeded
///   when src.len() > MAX_MEMOP_BYTES.
/// CEP:ASSUMES: the slices do not overlap.
/// CEP:COST: O(src.len()) via core::ptr::copy_nonoverlapping.
/// CEP:EVIDENCE: test memcpy_matches_core_copy.
pub fn memcpy(dst: &mut [u8], src: &[u8]) -> Result<(), MemError> {
    if src.len() > MAX_MEMOP_BYTES {
        return Err(MemError::BudgetExceeded);
    }
    if src.len() > dst.len() {
        return Err(MemError::DestTooSmall);
    }
    // SAFETY: src.len() <= dst.len(), both slices are valid for their lengths, and the
    // non-overlap contract is documented on this function.
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), src.len());
    }
    Ok(())
}

/// CEP:WHAT: Fills the whole slice with value.
/// CEP:WHY: The runtime's checked replacement for C memset (the length is the slice
///   length, so no caller-side length bookkeeping can go wrong).
/// CEP:STATUS: complete
/// CEP:FAILURE: none; infallible over the slice.
/// CEP:ASSUMES: none
/// CEP:COST: O(buf.len()) via core::ptr::write_bytes.
/// CEP:EVIDENCE: test memset_matches_core_fill.
pub fn memset(buf: &mut [u8], value: u8) {
    // SAFETY: buf is a valid initialized slice; write_bytes over its full length is in
    // bounds and u8 has no drop glue.
    unsafe {
        core::ptr::write_bytes(buf.as_mut_ptr(), value, buf.len());
    }
}

/// CEP:WHAT: Compares two byte slices lexicographically: bytewise over the common prefix,
///   then the shorter slice orders first.
/// CEP:WHY: The runtime's checked replacement for C memcmp; returning an Ordering makes
///   the sign convention of the C return value impossible to misuse.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; infallible over the slices.
/// CEP:ASSUMES: none
/// CEP:COST: O(min(a.len(), b.len())) worst case.
/// CEP:EVIDENCE: test memcmp_matches_core_ordering.
pub fn memcmp(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: memcpy copies exactly and rejects destinations that are too small or over
    ///   budget.
    /// CEP:WHY: The fail-closed contract of the checked copy.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn memcpy_matches_core_copy() {
        let src = [1u8, 2, 3, 4, 5];
        let mut dst = [0u8; 8];
        memcpy(&mut dst, &src).expect("fits");
        assert_eq!(&dst[..5], &src);
        assert_eq!(&dst[5..], &[0, 0, 0]);
        let mut small = [0u8; 2];
        assert_eq!(memcpy(&mut small, &src), Err(MemError::DestTooSmall));
        assert_eq!(small, [0, 0], "failed copy writes nothing");
    }

    /// CEP:WHAT: Requests above MAX_MEMOP_BYTES are rejected before any touch.
    /// CEP:WHY: The named budget must actually bound the operation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: the oversized slice is never dereferenced - memcpy rejects on the
    ///   length check alone, so fabricating its header is safe for this probe.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn memcpy_rejects_oversized_requests() {
        let mut dst = [0u8; 4];
        // Length-only probe: the budget check short-circuits before any byte access, so
        // the fabricated oversized slice header is never dereferenced.
        let oversized: &[u8] =
            unsafe { core::slice::from_raw_parts(core::ptr::NonNull::<u8>::dangling().as_ptr(), MAX_MEMOP_BYTES + 1) };
        assert_eq!(memcpy(&mut dst, oversized), Err(MemError::BudgetExceeded));
        assert_eq!(dst, [0, 0, 0, 0], "failed copy writes nothing");
    }

    /// CEP:WHAT: memset fills every byte and matches the core fill.
    /// CEP:WHY: The checked fill contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn memset_matches_core_fill() {
        let mut buf = [0u8; 16];
        memset(&mut buf, 0xAB);
        let mut reference = [0u8; 16];
        reference.fill(0xAB);
        assert_eq!(buf, reference);
        memset(&mut buf, 0);
        assert!(buf.iter().all(|&b| b == 0));
    }

    /// CEP:WHAT: memcmp ordering matches the core slice ordering on equal, less, greater,
    ///   prefix, and empty inputs.
    /// CEP:WHY: The comparison contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn memcmp_matches_core_ordering() {
        use core::cmp::Ordering;
        assert_eq!(memcmp(b"abc", b"abc"), Ordering::Equal);
        assert_eq!(memcmp(b"abc", b"abd"), Ordering::Less);
        assert_eq!(memcmp(b"abd", b"abc"), Ordering::Greater);
        assert_eq!(memcmp(b"ab", b"abc"), Ordering::Less);
        assert_eq!(memcmp(b"", b""), Ordering::Equal);
        assert_eq!(memcmp(b"", b"a"), Ordering::Less);
    }
}
