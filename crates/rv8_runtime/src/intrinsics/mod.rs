// CEP:FILE: crates/rv8_runtime/src/intrinsics/mod.rs
// CEP:WHAT: The software intrinsics of the runtime (compiler-rt): 128-bit signed and
//   unsigned division and remainder, IEEE 754 soft-float add/sub/mul/div for f32 and f64,
//   and the byte memory operations memcpy, memset, and memcmp.
// CEP:WHY: Standard 5.8 and design.md require software implementations for operations the
//   hardware may not support; the compiler recognizes these symbols and may replace calls
//   with hardware instructions when the target has them.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every fallible intrinsic returns a typed error (IntMathError, MemError);
//   the unchecked division wrappers carry the documented std-mirroring panic semantics.
// CEP:ASSUMES: the compiler may lower calls to these functions to machine instructions on
//   capable targets; on the host they are plain functions exercised by tests.
// CEP:COST: int128 division is O(128) limb steps; soft-float ops are O(1) fixed-width
//   integer work; memory ops are O(n) bytewise.
// CEP:EVIDENCE: unit tests in int128, softfloat, and mem; integration tests
//   crates/rv8_runtime/tests/runtime_foundation.rs.

pub mod int128;
pub mod mem;
pub mod softfloat;
