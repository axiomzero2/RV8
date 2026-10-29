// CEP:FILE: crates/rv8_core/src/lib.rs
// CEP:WHAT: rv8_core, the shared foundation of the RV8 compiler: source spans, symbol interning,
//   structured diagnostics, a lock-free MPMC queue, a bump arena, deterministic hashing, SHA-256,
//   and a deterministic pseudo-random number generator.
// CEP:WHY: All RV8 crates share these primitives; centralizing them keeps cost, failure behavior,
//   and determinism auditable in one place and prevents per-crate divergence.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none at module level; each public function documents its own failure contract.
// CEP:ASSUMES: host provides std (this crate is compiler tooling, not the generated-code runtime).
// CEP:COST: cold at load; contained types are used on hot paths and document per-item cost.
// CEP:EVIDENCE: unit tests rv8_core::{span, interner, diag, queue, arena, hash, sha256, rng}::tests.
// CEP:SECURITY: diagnostics queue is bounded; no environment, time, or address values enter output.

pub mod arena;
pub mod diag;
pub mod hash;
pub mod interner;
pub mod queue;
pub mod rng;
pub mod sha256;
pub mod span;

pub use diag::{DiagEngine, DiagId, Diagnostic, Level};
pub use hash::{fx_mix, fxhash64, FxBuildHasher, FxHashMap, FxHashSet};
pub use interner::{Interner, SymbolId};
pub use queue::MpmcQueue;
pub use rng::SplitMix64;
pub use span::{SourceFile, SourceMap, Span};
