// CEP:FILE: crates/rv8_middle/src/lib.rs
// CEP:WHAT: Implementation lands in the next commit; placeholder module marker only.
// CEP:WHY: Keeps the workspace buildable while components are built in parallel.
// CEP:CLASS: HPC-1
// CEP:STATUS: stub
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: workspace build.

pub mod error;
pub mod analysis;
pub mod borrowck;
pub mod pea;
pub mod verifier;
pub mod pipeline;
pub mod passes;
