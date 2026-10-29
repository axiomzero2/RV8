// CEP:FILE: crates/rv8_middle/src/lib.rs
// CEP:WHAT: rv8_middle, the RV8 middle end: iterative Sea-of-Nodes analyses, three
//   deterministic passes (CONST_FOLD, GVN, DCE) under a verify-after-every-pass pipeline,
//   the deep Level-1 verifier, the graph-CSP borrow checker, partial escape analysis, and
//   Heterogeneous-IR dead-node elimination.
// CEP:WHY: Standard sections 2, 5.4, 5.6, and 6 assign verification, borrow checking, and
//   escape analysis to this crate; one crate boundary keeps determinism and budget policy
//   auditable in one place.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: every public entry point returns Result<_, MiddleError>; no panics in
//   library code; budgets are named constants in rv8_middle::analysis and the modules
//   that own them.
// CEP:ASSUMES: input graphs are SeaOfNodes/HetIr values from rv8_ir with dense ids; the
//   flattening input conventions documented in rv8_middle::analysis hold.
// CEP:COST: pass pipelines are O(N * fanin) per pass plus per-pass verification.
// CEP:EVIDENCE: unit tests across all modules; determinism tests in pipeline, borrowck,
//   and pea; negative verifier tests per spec 2.3 invariant.
// CEP:SECURITY: all traversals are iterative with named budgets; no recursion over
//   untrusted structure; no environment, time, or address data enters any output.
// CEP:DETERMINISM: deterministic end to end; pass order fixed; hash maps are lookup-only.

pub mod analysis;
pub mod borrowck;
pub mod error;
pub mod passes;
pub mod pea;
pub mod pipeline;
pub mod verifier;

pub use analysis::{
    build_cfg, build_dataflow, check_budget, detect_loops, dominators, liveness,
    ControlFlowGraph, DataFlowIndex, Dominators, Liveness, Loops, MAX_ANALYSIS_BLOCKS,
    MAX_LIVENESS_WORDS, MAX_MIDDLE_NODES,
};
pub use borrowck::{check, BorrowReport, Conflict, ConflictKind};
pub use error::MiddleError;
pub use passes::{const_fold, dce, gvn, PassReport};
pub use pea::{run_pea, PeaReport};
pub use pipeline::{optimize_hetir, optimize_son, OptSummary};
pub use verifier::{verify_son, VerifyKind, VerifyViolation};
