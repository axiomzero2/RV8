#![cfg_attr(not(test), no_std)]
// CEP:FILE: crates/rv8_runtime/src/lib.rs
// CEP:WHAT: The rv8_runtime foundation crate: marker and operator trait surface, Option and
//   Result contracts, RawVec/Vec growable buffers and Box over the runtime Allocator trait,
//   the Iterator trait with map/filter/fold adapters, software intrinsics (128-bit division
//   and remainder via 64-bit-limb shift-and-subtract, IEEE 754 soft-float add/sub/mul/div,
//   memcpy/memset/memcmp), the fixed-region Bump allocator, the bare-metal panic hook, and
//   the JIT executable-memory manager (jitmem: W^X code pages, RV8-141).
// CEP:WHY: Standard 5.8 and design.md (the runtime section) require generated code to run
//   without libc or std: primitives are compiler intrinsics, so the runtime defines the
//   trait and type vocabulary, the collections, the allocator, and the software fallbacks.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every fallible library operation returns Result (Vec growth, Box and Bump
//   allocation, checked 128-bit division); documented panic semantics mirror std exactly
//   where the design says so (Option::unwrap, Result::unwrap, Vec indexing, unchecked
//   division by zero); no other panic path exists.
// CEP:ASSUMES: the crate is no_std outside cfg(test); host tests run under std via the
//   cfg_attr shim, which is the documented test-only std allowance; alloc is available in
//   every configuration; the global allocator is NOT registered here (bare-metal programs
//   hand a fixed region to Bump explicitly).
// CEP:COST: collections and iterators are O(1) amortized per element; 128-bit division is
//   O(128) limb steps; soft-float ops are O(1) fixed-width integer work.
// CEP:EVIDENCE: unit tests rv8_runtime::*::tests and integration tests
//   crates/rv8_runtime/tests/runtime_foundation.rs.
// CEP:SECURITY: Bump trusts its region parameter (documented at the constructor); every
//   other module validates inputs and fails closed.

extern crate alloc;

pub mod allocator;
pub mod boxed;
pub mod bump;
pub mod intrinsics;
pub mod iter;
pub mod jitmem;
pub mod option;
pub mod panic;
pub mod traits;
pub mod vec;

pub use allocator::{AllocError, Allocator, Layout};
pub use boxed::Box;
pub use bump::Bump;
pub use vec::{CollectionError, RawVec, Vec};

/// CEP:WHAT: Byte budget for one collection buffer.
/// CEP:WHY: Named budget (38.12); checked capacity arithmetic rejects growth beyond it
///   with CollectionError::CapacityOverflow instead of wrapping.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: one gibibyte is the standing ceiling for a single runtime buffer on the
///   bare-metal targets the runtime serves.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: test vec_capacity_overflow_is_checked.
pub const MAX_COLLECTION_BYTES: usize = 1 << 30;
